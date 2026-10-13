//! Logging for the host: the console, plus log files in a `logs` folder next to the
//! host's other data, so a bug report can include what happened.
//!
//! * Files: `perritodeck.log` is the current one; older ones are `perritodeck.1.log`
//!   (newest) to `perritodeck.6.log`. A new file starts each day, or when the current
//!   one passes [`MAX_FILE_BYTES`], so the folder stays under about 10 MB.
//! * Everything written (files and console) goes through [`redact`] first, so tokens,
//!   passwords and the like don't end up in logs people attach to bug reports.
//! * "Detailed logs" ([`set_detailed`]) switches PerritoDeck's own messages to `debug`
//!   while the host runs; `RUST_LOG` still works for anything finer.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use tracing_subscriber::{
    fmt::MakeWriter, layer::SubscriberExt, reload, util::SubscriberInitExt, EnvFilter, Layer,
};

/// The current log file's name.
pub const LOG_FILE: &str = "perritodeck.log";
/// How many files are kept, the current one included.
pub const KEEP_FILES: usize = 7;
/// A file this big is closed and a new one started (7 × 1.4 MB ≈ 10 MB in all).
pub const MAX_FILE_BYTES: u64 = 1_400 * 1024;

/// The log folder for a data folder.
pub fn logs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// Where the log output goes besides the files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Console {
    Stdout,
    /// For `perritodeck-host mcp`, whose stdout carries the protocol.
    Stderr,
    Off,
}

#[derive(Debug, Clone)]
pub struct LogOptions {
    /// The log folder; `None` logs to the console only.
    pub dir: Option<PathBuf>,
    pub console: Console,
    /// Start with detailed (debug) logs.
    pub detailed: bool,
    /// The level when not detailed and `RUST_LOG` isn't set (normally `info`).
    pub level: &'static str,
}

impl LogOptions {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            console: Console::Stdout,
            detailed: false,
            level: "info",
        }
    }
}

type Reloader = Box<dyn Fn(bool) + Send + Sync>;
static RELOAD: OnceLock<Reloader> = OnceLock::new();

/// Libraries quietened unless `RUST_LOG` says otherwise. fontdb (the font list key
/// images are drawn with) warns at every start about each broken font file in the
/// system's fonts folder, which PerritoDeck can do nothing about.
const QUIET: &str = "fontdb=error";

/// The filter when `RUST_LOG` isn't set.
fn default_filter(level: &str) -> String {
    format!("{level},{QUIET}")
}

fn filter(base: &str, detailed: bool) -> EnvFilter {
    let text = if detailed {
        format!("{base},perritodeck=debug")
    } else {
        base.to_owned()
    };
    EnvFilter::try_new(&text).unwrap_or_else(|_| EnvFilter::new("info"))
}

/// Set up logging for this process (once; later calls do nothing). Returns the log
/// file's path when file logging works.
pub fn init(opts: LogOptions) -> Option<PathBuf> {
    let base = std::env::var("RUST_LOG")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| default_filter(opts.level));
    let (filter_layer, handle) = reload::Layer::new(filter(&base, opts.detailed));
    let mut file_problem = None;
    let file = opts
        .dir
        .as_ref()
        .and_then(|dir| match RotatingFile::open(dir) {
            Ok(f) => Some(f),
            Err(e) => {
                file_problem = Some(format!("can't write logs in {}: {e}", dir.display()));
                None
            }
        });
    let path = file.as_ref().map(RotatingFile::path);
    let file_layer = file.map(|f| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(Redact(f))
            .boxed()
    });
    let console_layer = match opts.console {
        Console::Stdout => Some(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(Redact(io::stdout))
                .boxed(),
        ),
        Console::Stderr => Some(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(false)
                .with_writer(Redact(io::stderr))
                .boxed(),
        ),
        Console::Off => None,
    };
    let installed = tracing_subscriber::registry()
        .with(filter_layer)
        .with(file_layer)
        .with(console_layer)
        .try_init()
        .is_ok();
    if !installed {
        return None;
    }
    let _ = RELOAD.set(Box::new(move |detailed| {
        let _ = handle.reload(filter(&base, detailed));
    }));
    if let Some(problem) = file_problem {
        tracing::warn!("{problem}");
    }
    path
}

/// Turn detailed (debug) logs on or off now. Does nothing before [`init`].
pub fn set_detailed(detailed: bool) {
    if let Some(reload) = RELOAD.get() {
        reload(detailed);
        tracing::info!("detailed logs {}", if detailed { "on" } else { "off" });
    }
}

// ---- Redaction ----

/// Names whose values are secrets, matched at the end of a field name, ignoring case
/// (`token`, `access_token`, `mcpToken`, `password`, …).
const SECRET_NAMES: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "apikey",
    "api_key",
    "api-key",
    "authorization",
    "private_key",
    "pairing_code",
    "pairingcode",
];

const REDACTED: &str = "[redacted]";

/// Hide secrets in a log line: `Bearer …` tokens, `user:password@` in URLs, values of
/// fields named like a secret (`token=…`, `"password": "…"`), long hex strings (access
/// tokens, hashes) and JSON web tokens.
pub fn redact(line: &str) -> String {
    let out = redact_bearer(line);
    let out = redact_userinfo(&out);
    let out = redact_named(&out);
    redact_long_tokens(&out)
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-._~+/=".contains(c)
}

fn redact_bearer(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(at) = lower.find("bearer ") else {
            out.push_str(rest);
            return out;
        };
        let start = at + "bearer ".len();
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let end = after
            .char_indices()
            .find(|(_, c)| !is_token_char(*c))
            .map_or(after.len(), |(i, _)| i);
        if end > 0 && &after[..end] != REDACTED {
            out.push_str(REDACTED);
        } else {
            out.push_str(&after[..end]);
        }
        rest = &after[end..];
    }
}

fn redact_userinfo(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find("://") {
        let start = at + 3;
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let authority_end = after
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace() || c == '"')
            .unwrap_or(after.len());
        let authority = &after[..authority_end];
        match authority.rfind('@') {
            Some(i) if authority[..i].contains(':') => {
                out.push_str(REDACTED);
                out.push_str(&authority[i..]);
            }
            _ => out.push_str(authority),
        }
        rest = &after[authority_end..];
    }
    out.push_str(rest);
    out
}

fn redact_named(s: &str) -> String {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut out = String::with_capacity(s.len());
    let mut copied = 0; // bytes of `s` already in `out`
    let mut i = 0;
    while i < chars.len() {
        if !ident(chars[i].1) || (i > 0 && ident(chars[i - 1].1)) {
            i += 1;
            continue;
        }
        // A whole name starts at i.
        let mut j = i;
        while j < chars.len() && ident(chars[j].1) {
            j += 1;
        }
        let name_end = chars.get(j).map_or(s.len(), |c| c.0);
        let name = s[chars[i].0..name_end].to_ascii_lowercase();
        if !SECRET_NAMES.iter().any(|n| name.ends_with(n)) {
            i = j;
            continue;
        }
        // name, then optionally a closing quote, spaces, `=` or `:`, spaces, a quote.
        let mut k = j;
        if k < chars.len() && matches!(chars[k].1, '"' | '\'') {
            k += 1;
        }
        while k < chars.len() && chars[k].1 == ' ' {
            k += 1;
        }
        if k >= chars.len() || !matches!(chars[k].1, '=' | ':') {
            i = j;
            continue;
        }
        k += 1;
        while k < chars.len() && chars[k].1 == ' ' {
            k += 1;
        }
        let quote = chars
            .get(k)
            .map(|c| c.1)
            .filter(|c| matches!(c, '"' | '\''));
        if quote.is_some() {
            k += 1;
        }
        let value_start = k;
        let from = chars.get(value_start).map_or(s.len(), |c| c.0);
        if s[from..].starts_with(REDACTED) {
            i = k.max(j);
            continue;
        }
        while k < chars.len() {
            let c = chars[k].1;
            let stop = match quote {
                Some(q) => c == q,
                None => c.is_whitespace() || ",;&}])\"'".contains(c),
            };
            if stop {
                break;
            }
            k += 1;
        }
        let to = chars.get(k).map_or(s.len(), |c| c.0);
        let value = &s[from..to];
        if !value.is_empty() {
            out.push_str(&s[copied..from]);
            out.push_str(REDACTED);
            copied = to;
        }
        i = k.max(j);
    }
    out.push_str(&s[copied..]);
    out
}

/// Hex runs of 32+ characters and `eyJ…` JSON web tokens.
fn redact_long_tokens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut word_start = None;
    let flush = |out: &mut String, word: &str| {
        let hex = word.len() >= 32 && word.chars().all(|c| c.is_ascii_hexdigit());
        let jwt = word.starts_with("eyJ") && word.len() >= 30 && word.matches('.').count() == 2;
        if hex || jwt {
            out.push_str(REDACTED);
        } else {
            out.push_str(word);
        }
    };
    for (i, c) in s.char_indices() {
        let part = c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-';
        match (part, word_start) {
            (true, None) => word_start = Some(i),
            (false, Some(st)) => {
                flush(&mut out, &s[st..i]);
                word_start = None;
                out.push(c);
            }
            (false, None) => out.push(c),
            (true, Some(_)) => {}
        }
    }
    if let Some(st) = word_start {
        flush(&mut out, &s[st..]);
    }
    out
}

/// A `MakeWriter` that redacts everything written through it.
#[derive(Clone)]
pub struct Redact<M>(pub M);

pub struct RedactWriter<W>(W);

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for Redact<M> {
    type Writer = RedactWriter<M::Writer>;
    fn make_writer(&'a self) -> Self::Writer {
        RedactWriter(self.0.make_writer())
    }
}

impl<W: Write> Write for RedactWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // tracing writes each event in one call, so `buf` holds whole lines.
        let text = String::from_utf8_lossy(buf);
        self.0.write_all(redact(&text).as_bytes())?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

// ---- Rotating files ----

/// How long to wait after a failed rotation before trying again.
const ROTATE_RETRY: Duration = Duration::from_secs(60);

struct Current {
    file: File,
    size: u64,
    /// The local date (YYYY-MM-DD) the file is for.
    day: String,
    /// When a rotation failed, the earliest time to try again.
    retry_rotation: Option<Instant>,
}

/// Log files that start afresh each day or when too big, keeping [`KEEP_FILES`].
#[derive(Clone)]
pub struct RotatingFile {
    dir: PathBuf,
    max_bytes: u64,
    current: Arc<Mutex<Option<Current>>>,
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn file_day(path: &Path) -> Option<String> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let local: chrono::DateTime<chrono::Local> = modified.into();
    Some(local.format("%Y-%m-%d").to_string())
}

/// `perritodeck.log`, then `perritodeck.1.log` … older.
fn numbered(dir: &Path, n: usize) -> PathBuf {
    if n == 0 {
        dir.join(LOG_FILE)
    } else {
        dir.join(format!("perritodeck.{n}.log"))
    }
}

impl RotatingFile {
    pub fn open(dir: &Path) -> io::Result<Self> {
        Self::with_limit(dir, MAX_FILE_BYTES)
    }

    pub fn with_limit(dir: &Path, max_bytes: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let me = Self {
            dir: dir.to_owned(),
            max_bytes,
            current: Arc::new(Mutex::new(None)),
        };
        // A file from an earlier day is moved aside by the first line written.
        let day = file_day(&numbered(dir, 0)).unwrap_or_else(today);
        *me.current.lock().expect("log poisoned") = Some(me.open_current(day)?);
        Ok(me)
    }

    pub fn path(&self) -> PathBuf {
        numbered(&self.dir, 0)
    }

    fn open_current(&self, day: String) -> io::Result<Current> {
        let path = numbered(&self.dir, 0);
        let mut opts = OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(&path)?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Current {
            file,
            size,
            day,
            retry_rotation: None,
        })
    }

    /// Shift every file up one number, dropping the oldest.
    ///
    /// Files only ever move into a free number, from the first gap down. A rename can
    /// fail when another program holds the file open without allowing that (some log
    /// viewers and virus scanners do, on Windows); stopping there loses nothing, and the
    /// next try carries on from the gap it left instead of dropping another old file.
    fn rotate(&self) -> io::Result<()> {
        let gap = (1..KEEP_FILES)
            .find(|&n| !numbered(&self.dir, n).exists())
            .unwrap_or_else(|| {
                let _ = fs::remove_file(numbered(&self.dir, KEEP_FILES - 1));
                KEEP_FILES - 1
            });
        for n in (0..gap).rev() {
            let from = numbered(&self.dir, n);
            if from.exists() {
                fs::rename(&from, numbered(&self.dir, n + 1))?;
            }
        }
        Ok(())
    }

    fn write_bytes(&self, buf: &[u8]) -> io::Result<()> {
        let mut guard = self.current.lock().unwrap_or_else(|p| p.into_inner());
        let day = today();
        let now = Instant::now();
        let full = guard.as_ref().is_some_and(|c| {
            (c.day != day || (c.size > 0 && c.size + buf.len() as u64 > self.max_bytes))
                && c.retry_rotation.map_or(true, |at| now >= at)
        });
        if full || guard.is_none() {
            // Close it before renaming (Windows).
            let was = guard.take().map(|c| c.day);
            let next = match was {
                Some(was) if full => match self.rotate() {
                    Ok(()) => self.open_current(day)?,
                    // Keep adding to the same file, and try again in a while.
                    Err(_) => {
                        let mut c = self.open_current(was)?;
                        c.retry_rotation = Some(now + ROTATE_RETRY);
                        c
                    }
                },
                _ => self.open_current(day)?,
            };
            *guard = Some(next);
        }
        let current = guard.as_mut().expect("just opened");
        current.file.write_all(buf)?;
        current.size += buf.len() as u64;
        Ok(())
    }
}

pub struct RotatingWriter(RotatingFile);

impl Write for RotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_bytes(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for RotatingFile {
    type Writer = RotatingWriter;
    fn make_writer(&'a self) -> Self::Writer {
        RotatingWriter(self.clone())
    }
}

/// The last `n` lines logged (from the newest files), redacted.
pub fn tail(dir: &Path, n: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for k in 0..KEEP_FILES {
        if lines.len() >= n {
            break;
        }
        let Ok(bytes) = fs::read(numbered(dir, k)) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut older: Vec<String> = text.lines().map(redact).collect();
        older.append(&mut lines);
        lines = older;
    }
    let skip = lines.len().saturating_sub(n);
    lines.split_off(skip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_redacted() {
        let cases = [
            (
                "Authorization: Bearer abc.DEF-123_x",
                "Authorization: [redacted] [redacted]",
            ),
            (
                "sent bearer tok123 to host",
                "sent bearer [redacted] to host",
            ),
            (
                "couldn't open http://me:hunter2@example.com/x?y=1",
                "couldn't open http://[redacted]@example.com/x?y=1",
            ),
            (
                "connected to ws://127.0.0.1:4455",
                "connected to ws://127.0.0.1:4455",
            ),
            ("token=abc123 next", "token=[redacted] next"),
            (
                r#"{"password": "p a s s", "url": "x"}"#,
                r#"{"password": "[redacted]", "url": "x"}"#,
            ),
            ("access_token: zzz, rest", "access_token: [redacted], rest"),
            ("mcpToken=\"q\"", "mcpToken=\"[redacted]\""),
            (
                "https://h/api?api_key=K&x=1",
                "https://h/api?api_key=[redacted]&x=1",
            ),
            (
                "token 0123456789abcdef0123456789abcdef01 found",
                "token [redacted] found",
            ),
            (
                "got eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJlLXZhbHVl ok",
                "got [redacted] ok",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(redact(input), want, "{input}");
        }
    }

    #[test]
    fn ordinary_lines_are_untouched() {
        for line in [
            "2026-10-11T10:00:00Z  INFO device=tab key=3: key 4 pressed",
            "plugin=dev.perritodeck.http request failed: 404",
            "watching live tokens: cpu, mem",
            "tokens: cpu",
            "showing profile streaming",
            "fingerprint 3FA9 07C2 D41E",
            "",
        ] {
            assert_eq!(redact(line), line);
        }
        // Idempotent.
        let once = redact("token=abc Bearer xyz");
        assert_eq!(redact(&once), once);
    }

    #[test]
    fn files_rotate_by_size_and_keep_seven() {
        let dir = tempfile::tempdir().unwrap();
        let logs = RotatingFile::with_limit(dir.path(), 100).unwrap();
        let mut w = logs.make_writer();
        for i in 0..40 {
            writeln!(w, "line {i:02} {}", "x".repeat(40)).unwrap();
        }
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), KEEP_FILES, "{names:?}");
        assert!(names.contains(&LOG_FILE.to_owned()));
        let current = fs::read_to_string(dir.path().join(LOG_FILE)).unwrap();
        assert!(current.contains("line 39"));
        assert!(current.len() <= 100);
        let tail = tail(dir.path(), 5);
        assert_eq!(tail.len(), 5);
        assert!(tail[4].starts_with("line 39"), "{tail:?}");
        assert!(tail[0].starts_with("line 35"), "{tail:?}");
        // Older lines than the kept files are gone.
        assert!(!super::tail(dir.path(), 1000)
            .iter()
            .any(|l| l.starts_with("line 00")));
    }

    #[test]
    fn rotation_fills_a_gap_before_dropping_the_oldest() {
        let dir = tempfile::tempdir().unwrap();
        // As a rotation that stopped part way leaves them: no perritodeck.2.log.
        for n in [0, 1, 3, 4, 5, 6] {
            fs::write(numbered(dir.path(), n), format!("file {n}\n")).unwrap();
        }
        let logs = RotatingFile {
            dir: dir.path().to_owned(),
            max_bytes: 100,
            current: Arc::new(Mutex::new(None)),
        };
        logs.rotate().unwrap();
        let read = |n| fs::read_to_string(numbered(dir.path(), n)).ok();
        assert_eq!(read(0), None);
        assert_eq!(read(1).as_deref(), Some("file 0\n"));
        assert_eq!(read(2).as_deref(), Some("file 1\n"));
        assert_eq!(read(3).as_deref(), Some("file 3\n"));
        assert_eq!(read(6).as_deref(), Some("file 6\n"), "the oldest is kept");
        // With no gap left, the oldest goes.
        fs::write(numbered(dir.path(), 0), "file 7\n").unwrap();
        logs.rotate().unwrap();
        assert_eq!(read(1).as_deref(), Some("file 7\n"));
        assert_eq!(read(6).as_deref(), Some("file 5\n"));
    }

    /// Some log viewers and virus scanners open a file without letting it be renamed or
    /// deleted (no `FILE_SHARE_DELETE`), so the current file can't be moved aside.
    #[cfg(windows)]
    #[test]
    fn a_file_held_open_by_another_program_loses_no_lines() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_SHARE_WRITE: u32 = 0x2;
        let dir = tempfile::tempdir().unwrap();
        let logs = RotatingFile::with_limit(dir.path(), 100).unwrap();
        let mut w = logs.make_writer();
        let line = |i: usize| format!("line {i:02} {}", "x".repeat(40));
        for i in 0..5 {
            writeln!(w, "{}", line(i)).unwrap();
        }
        let viewer = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(dir.path().join(LOG_FILE))
            .unwrap();
        for i in 5..45 {
            writeln!(w, "{}", line(i)).unwrap();
            // Try again on every line rather than after a minute.
            if let Some(c) = logs.current.lock().unwrap().as_mut() {
                c.retry_rotation = None;
            }
        }
        let held = fs::metadata(dir.path().join(LOG_FILE)).unwrap().len();
        assert!(held > 100, "the viewer kept it from being moved aside");
        let all: Vec<String> = (0..45).map(line).collect();
        assert_eq!(tail(dir.path(), 1000), all, "every line, in order");

        // Once it's let go, the file is moved aside as usual.
        drop(viewer);
        writeln!(w, "{}", line(45)).unwrap();
        let current = fs::read_to_string(dir.path().join(LOG_FILE)).unwrap();
        assert_eq!(current.trim_end(), line(45));
        let all: Vec<String> = (0..46).map(line).collect();
        assert_eq!(tail(dir.path(), 1000), all);
    }

    #[test]
    fn font_warnings_stay_out_of_the_default_log() {
        #[derive(Clone, Default)]
        struct Seen(Arc<Mutex<Vec<u8>>>);
        impl Write for Seen {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let logged = |text: &str, detailed: bool| {
            let seen = Seen::default();
            let writer = seen.clone();
            let subscriber = tracing_subscriber::registry()
                .with(filter(text, detailed))
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(move || writer.clone()),
                );
            tracing::subscriber::with_default(subscriber, || {
                tracing::warn!(target: "fontdb", "Failed to load a font face 0");
                tracing::error!(target: "fontdb", "no fonts at all");
                tracing::warn!(target: "perritodeck_host", "one of ours");
            });
            let out = seen.0.lock().unwrap().clone();
            String::from_utf8(out).unwrap()
        };
        for detailed in [false, true] {
            let out = logged(&default_filter("info"), detailed);
            assert!(!out.contains("font face"), "{out}");
            assert!(out.contains("no fonts at all"), "{out}");
            assert!(out.contains("one of ours"), "{out}");
        }
        // `RUST_LOG` replaces the default, so it can bring them back.
        let out = logged("info", false);
        assert!(out.contains("font face"), "{out}");
    }

    #[test]
    fn tail_redacts_old_lines_too() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(LOG_FILE), "a token=xyz\nb\n").unwrap();
        assert_eq!(tail(dir.path(), 10), ["a token=[redacted]", "b"]);
    }
}
