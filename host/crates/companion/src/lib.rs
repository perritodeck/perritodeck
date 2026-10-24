//! Bitfocus Companion's Satellite API, client side: each PerritoDeck tablet showing a
//! "Companion" profile appears in Companion as a surface, shows the buttons Companion
//! draws, and sends its presses back.
//!
//! Written from Companion's public Satellite API documentation (TCP, port 16622, one
//! text line per message). Only the "simple" surface layout is used: a grid of keys.
//!
//! [`Client::start`] runs in the background and reconnects by itself; surfaces added
//! while Companion is away are registered as soon as it's back.

pub mod line;

use std::{collections::HashMap, sync::Arc, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::mpsc,
};
use tracing::{debug, info, warn};

pub use line::Line;

/// Companion's Satellite port.
pub const DEFAULT_PORT: u16 = 16622;
const PING_EVERY: Duration = Duration::from_secs(2);
/// No word from Companion for this long and the connection is considered dead.
const SILENCE: Duration = Duration::from_secs(if cfg!(test) { 3 } else { 10 });
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(15);

/// One tablet, as Companion sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surface {
    /// Unique per surface; also its serial in Companion, so it keeps its settings.
    pub id: String,
    /// What Companion's Surfaces page shows.
    pub name: String,
    pub cols: u32,
    pub rows: u32,
    /// Key image size; `None` for a tablet that only shows text and colours.
    pub bitmap_px: Option<u32>,
}

/// How a key should look, from Companion.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyState {
    /// The button drawn by Companion, as PNG.
    pub png: Option<Vec<u8>>,
    /// `#rrggbb`
    pub color: Option<String>,
    pub text_color: Option<String>,
    pub text: Option<String>,
    pub pressed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Connected {
        companion_version: String,
    },
    /// Not connected, and why; it tries again shortly.
    Disconnected {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Status(Status),
    Key {
        surface: String,
        key: u32,
        state: KeyState,
    },
    /// Every key goes blank.
    Clear {
        surface: String,
    },
}

enum Command {
    Add(Surface),
    Remove(String),
    Press {
        surface: String,
        key: u32,
        down: bool,
    },
}

pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

/// Talks to one Companion. Cheap to clone; dropping every clone stops it.
#[derive(Clone)]
pub struct Client {
    tx: mpsc::UnboundedSender<Command>,
}

impl Client {
    /// Connect to Companion at `host:port` and keep connected. Events arrive on
    /// `events` (from a background task).
    pub fn start(host: String, port: u16, events: EventSink) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(host, port, rx, events));
        Self { tx }
    }

    /// Show a surface in Companion (or update it if its size changed).
    pub fn add(&self, surface: Surface) {
        let _ = self.tx.send(Command::Add(surface));
    }

    pub fn remove(&self, id: &str) {
        let _ = self.tx.send(Command::Remove(id.to_owned()));
    }

    pub fn press(&self, surface: &str, key: u32, down: bool) {
        let _ = self.tx.send(Command::Press {
            surface: surface.to_owned(),
            key,
            down,
        });
    }
}

/// Who answered a probe: Companion, and the Satellite API version it speaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Greeting {
    pub companion_version: String,
    pub api: String,
}

/// Why a probe got no greeting.
#[derive(Debug)]
pub enum ProbeError {
    /// Nothing took the connection in time.
    ConnectTimeout,
    Io(std::io::Error),
    /// The connection was taken, but nothing was said in time.
    Silent,
    /// It hung up without a word.
    Closed,
    /// Something other than Companion's Satellite port answered; its first word.
    NotSatellite(String),
    /// The connection broke while reading.
    Lost(String),
}

/// Settings → Integrations' Test connection: connect, read Companion's BEGIN line and
/// hang up, registering nothing.
pub async fn probe(host: &str, port: u16, wait: Duration) -> Result<Greeting, ProbeError> {
    let stream = match tokio::time::timeout(wait, TcpStream::connect((host, port))).await {
        Err(_) => return Err(ProbeError::ConnectTimeout),
        Ok(Err(e)) => return Err(ProbeError::Io(e)),
        Ok(Ok(stream)) => stream,
    };
    let (read, mut write) = stream.into_split();
    let mut lines = LineReader::new(read);
    let first = match tokio::time::timeout(wait, lines.next_line()).await {
        Err(_) => return Err(ProbeError::Silent),
        Ok(Err(e)) if e == CLOSED => return Err(ProbeError::Closed),
        Ok(Err(e)) => return Err(ProbeError::Lost(e)),
        Ok(Ok(line)) => line::parse(&line),
    };
    let answer = match greeting(&first) {
        Some((companion_version, (a, b, c))) => Ok(Greeting {
            companion_version,
            api: format!("{a}.{b}.{c}"),
        }),
        None => Err(ProbeError::NotSatellite(
            first
                .command
                .chars()
                .filter(char::is_ascii_graphic)
                .take(24)
                .collect(),
        )),
    };
    let _ = write.write_all(b"QUIT\n").await;
    answer
}

/// Companion's version and Satellite API version, from the BEGIN line it greets with.
fn greeting(l: &Line) -> Option<(String, (u32, u32, u32))> {
    (l.command == "BEGIN").then(|| {
        (
            l.get("CompanionVersion").unwrap_or("?").to_owned(),
            parse_version(l.get("ApiVersion").unwrap_or("1.0.0")),
        )
    })
}

/// What the connected Companion can do.
#[derive(Debug, Clone, Default)]
struct Caps {
    api: (u32, u32, u32),
    png: bool,
}

impl Caps {
    fn at_least(&self, major: u32, minor: u32, patch: u32) -> bool {
        self.api >= (major, minor, patch)
    }
}

fn parse_version(v: &str) -> (u32, u32, u32) {
    let mut it = v
        .split(['.', '-', '+'])
        .map(|p| p.parse::<u32>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

async fn run(host: String, port: u16, mut rx: mpsc::UnboundedReceiver<Command>, events: EventSink) {
    let mut surfaces: HashMap<String, Surface> = HashMap::new();
    let mut backoff = Duration::from_secs(1);
    loop {
        events(Event::Status(Status::Connecting));
        let reason =
            match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), port)))
                .await
            {
                Err(_) => format!("Companion at {host}:{port} didn't answer."),
                Ok(Err(e)) => format!("Couldn't reach Companion at {host}:{port}: {e}"),
                Ok(Ok(stream)) => {
                    let _ = stream.set_nodelay(true);
                    match session(stream, &mut rx, &mut surfaces, &events, &mut backoff).await {
                        Ok(()) => return, // every Client was dropped
                        Err(reason) => reason,
                    }
                }
            };
        info!("companion: {reason}");
        events(Event::Status(Status::Disconnected { reason }));
        // Keep track of surfaces while waiting, then try again.
        let wait = tokio::time::sleep(backoff);
        tokio::pin!(wait);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                cmd = rx.recv() => match cmd {
                    None => return,
                    Some(Command::Add(s)) => { surfaces.insert(s.id.clone(), s); }
                    Some(Command::Remove(id)) => { surfaces.remove(&id); }
                    Some(Command::Press { .. }) => {}
                },
            }
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// One connection. `Ok(())` means the client was dropped; `Err` says why it ended.
async fn session(
    stream: TcpStream,
    rx: &mut mpsc::UnboundedReceiver<Command>,
    surfaces: &mut HashMap<String, Surface>,
    events: &EventSink,
    backoff: &mut Duration,
) -> Result<(), String> {
    let (read, mut write) = stream.into_split();
    let mut lines = LineReader::new(read);

    // BEGIN, then (from API 1.10) CAPS.
    let begin = line::parse(&next(&mut lines).await?);
    let Some((companion_version, api)) = greeting(&begin) else {
        return Err(format!(
            "That isn't Companion's Satellite port (it said {}).",
            begin.command
        ));
    };
    let mut caps = Caps { api, png: false };
    let mut pending = Vec::new();
    if caps.at_least(1, 10, 0) {
        // CAPS comes straight after BEGIN; don't wait forever for it.
        if let Ok(l) = tokio::time::timeout(Duration::from_secs(2), next(&mut lines)).await {
            let l = line::parse(&l?);
            if l.command == "CAPS" {
                caps.png = l
                    .get("BITMAP_FORMATS")
                    .is_some_and(|f| f.split(',').any(|x| x.trim() == "png"));
            } else {
                pending.push(l);
            }
        }
    }
    info!(
        "companion: connected to Companion {companion_version} (Satellite API {}.{}.{})",
        caps.api.0, caps.api.1, caps.api.2
    );
    events(Event::Status(Status::Connected { companion_version }));
    *backoff = Duration::from_secs(1);

    for s in surfaces.values() {
        send(&mut write, add_device(s, &caps)).await?;
    }
    let sizes: HashMap<String, Option<u32>> = surfaces
        .iter()
        .map(|(k, s)| (k.clone(), s.bitmap_px))
        .collect();
    let mut sizes = sizes;
    for l in pending {
        handle(&l, &sizes, events, &mut write).await?;
    }

    let mut ping = tokio::time::interval(PING_EVERY);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut n = 0u64;
    let mut last_heard = std::time::Instant::now();
    loop {
        tokio::select! {
            l = lines.next_line() => {
                last_heard = std::time::Instant::now();
                let l = line::parse(&l?);
                handle(&l, &sizes, events, &mut write).await?;
            }
            cmd = rx.recv() => match cmd {
                None => {
                    let _ = write.write_all(b"QUIT\n").await;
                    return Ok(());
                }
                Some(Command::Add(s)) => {
                    if surfaces.get(&s.id) != Some(&s) {
                        if surfaces.contains_key(&s.id) {
                            send(&mut write, line::format("REMOVE-DEVICE", &[("DEVICEID", s.id.clone())])).await?;
                        }
                        send(&mut write, add_device(&s, &caps)).await?;
                        sizes.insert(s.id.clone(), s.bitmap_px);
                        surfaces.insert(s.id.clone(), s);
                    }
                }
                Some(Command::Remove(id)) => {
                    if surfaces.remove(&id).is_some() {
                        sizes.remove(&id);
                        send(&mut write, line::format("REMOVE-DEVICE", &[("DEVICEID", id)])).await?;
                    }
                }
                Some(Command::Press { surface, key, down }) => {
                    if surfaces.contains_key(&surface) {
                        send(&mut write, line::format("KEY-PRESS", &[
                            ("DEVICEID", surface),
                            ("KEY", key.to_string()),
                            ("PRESSED", if down { "true" } else { "false" }.into()),
                        ])).await?;
                    }
                }
            },
            _ = ping.tick() => {
                // Companion answers every ping; silence means the link is dead even if
                // the connection never closed.
                if last_heard.elapsed() > SILENCE {
                    return Err("Companion stopped answering.".to_owned());
                }
                n += 1;
                send(&mut write, format!("PING {n}\n")).await?;
            }
        }
    }
}

/// Why reading stopped when Companion hung up.
const CLOSED: &str = "Companion closed the connection.";

/// The longest line accepted from Companion: a 512 px raw RGB bitmap in base64 is ~1 MB.
const MAX_LINE: usize = 4 * 1024 * 1024;

/// Reads lines with a length limit, so a rogue server can't fill memory. Cancel-safe:
/// a partly read line stays in `buf` until the next call.
struct LineReader {
    r: BufReader<tokio::net::tcp::OwnedReadHalf>,
    buf: Vec<u8>,
}

impl LineReader {
    fn new(r: tokio::net::tcp::OwnedReadHalf) -> Self {
        Self {
            r: BufReader::new(r),
            buf: Vec::new(),
        }
    }

    async fn next_line(&mut self) -> Result<String, String> {
        use tokio::io::AsyncBufReadExt as _;
        loop {
            let avail = self
                .r
                .fill_buf()
                .await
                .map_err(|e| format!("Connection to Companion lost: {e}"))?;
            if avail.is_empty() {
                return Err(CLOSED.to_owned());
            }
            let (take, done) = match avail.iter().position(|&b| b == b'\n') {
                Some(i) => (i + 1, true),
                None => (avail.len(), false),
            };
            self.buf.extend_from_slice(&avail[..take]);
            self.r.consume(take);
            if self.buf.len() > MAX_LINE {
                return Err("Companion sent a line that's far too long.".to_owned());
            }
            if done {
                let line = String::from_utf8_lossy(&self.buf)
                    .trim_end_matches(['\r', '\n'])
                    .to_owned();
                self.buf.clear();
                return Ok(line);
            }
        }
    }
}

/// The next line within the silence limit (used before the ping loop starts).
async fn next(lines: &mut LineReader) -> Result<String, String> {
    match tokio::time::timeout(SILENCE, lines.next_line()).await {
        Err(_) => Err("Companion stopped answering.".to_owned()),
        Ok(r) => r,
    }
}

async fn send(w: &mut tokio::net::tcp::OwnedWriteHalf, text: String) -> Result<(), String> {
    w.write_all(text.as_bytes())
        .await
        .map_err(|e| format!("Connection to Companion lost: {e}"))
}

fn add_device(s: &Surface, caps: &Caps) -> String {
    // Before API 1.5.1 Companion allowed at most 8 × 4 keys.
    let (mut cols, mut total) = (s.cols.max(1), (s.cols * s.rows).max(1));
    if !caps.at_least(1, 5, 1) {
        cols = cols.min(8);
        total = total.min(32);
    }
    let mut args = vec![
        ("DEVICEID", s.id.clone()),
        ("PRODUCT_NAME", s.name.clone()),
        ("KEYS_TOTAL", total.to_string()),
        ("KEYS_PER_ROW", cols.to_string()),
        ("COLORS", "hex".into()),
        ("TEXT", "true".into()),
        ("BRIGHTNESS", "false".into()),
    ];
    let bitmaps = match s.bitmap_px {
        // Numbers are sizes from API 1.5.0; before that it's on (72 px) or off.
        Some(px) if caps.at_least(1, 5, 0) => px.clamp(36, 512).to_string(),
        Some(_) => "true".into(),
        None => "false".into(),
    };
    args.push(("BITMAPS", bitmaps));
    if caps.at_least(1, 10, 0) {
        args.push(("SERIAL", format!("perritodeck:{}", s.id)));
    }
    if s.bitmap_px.is_some() && caps.png {
        args.push(("BITMAP_FORMAT", "png".into()));
    }
    // COLORS=hex only exists from 1.6.0; earlier it's a boolean meaning the same.
    if !caps.at_least(1, 6, 0) {
        args[4].1 = "true".into();
    }
    line::format("ADD-DEVICE", &args)
}

async fn handle(
    l: &Line,
    sizes: &HashMap<String, Option<u32>>,
    events: &EventSink,
    write: &mut tokio::net::tcp::OwnedWriteHalf,
) -> Result<(), String> {
    match l.command.as_str() {
        "PING" => {
            let payload = l.words.join(" ");
            send(write, format!("PONG {payload}\n")).await?;
        }
        "KEY-STATE" => {
            let (Some(surface), Some(key)) = (
                l.get("DEVICEID"),
                l.get("KEY").and_then(|k| k.parse::<u32>().ok()),
            ) else {
                return Ok(());
            };
            let px = sizes.get(surface).copied().flatten();
            let state = KeyState {
                png: l.get("BITMAP").and_then(|b| bitmap_png(b, px)),
                color: l.get("COLOR").and_then(css_hex),
                text_color: l.get("TEXTCOLOR").and_then(css_hex),
                text: l
                    .get("TEXT")
                    .and_then(|t| STANDARD.decode(t).ok())
                    .map(|b| String::from_utf8_lossy(&b).into_owned()),
                pressed: l.flag("PRESSED"),
            };
            events(Event::Key {
                surface: surface.to_owned(),
                key,
                state,
            });
        }
        "KEYS-CLEAR" => {
            if let Some(surface) = l.get("DEVICEID") {
                events(Event::Clear {
                    surface: surface.to_owned(),
                });
            }
        }
        "ADD-DEVICE" if l.words.first().map(String::as_str) == Some("ERROR") => {
            warn!(
                "companion refused a tablet ({}): {}",
                l.get("DEVICEID").unwrap_or("?"),
                l.get("MESSAGE").unwrap_or("no reason given")
            );
        }
        "ERROR" => warn!(
            "companion: {}",
            l.get("MESSAGE").unwrap_or("error without a message")
        ),
        _ => debug!("companion: {}", l.command),
    }
    Ok(())
}

/// `#rrggbb` from Companion's hex or `rgb(r,g,b)` colours.
fn css_hex(c: &str) -> Option<String> {
    let c = c.trim();
    if let Some(hex) = c.strip_prefix('#') {
        return (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| format!("#{}", hex.to_ascii_lowercase()));
    }
    let inner = c.strip_prefix("rgb(")?.strip_suffix(')')?;
    let parts: Vec<u8> = inner
        .split(',')
        .map(|p| p.trim().parse::<u8>().ok())
        .collect::<Option<_>>()?;
    match parts[..] {
        [r, g, b] => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        _ => None,
    }
}

/// A KEY-STATE bitmap as PNG: either a PNG data URL, or raw RGB (square) that we encode.
fn bitmap_png(b: &str, px: Option<u32>) -> Option<Vec<u8>> {
    if let Some(rest) = b.strip_prefix("data:") {
        let (kind, data) = rest.split_once(";base64,")?;
        let bytes = STANDARD.decode(data).ok()?;
        return match kind {
            "image/png" => Some(bytes),
            // Anything else (WebP) we didn't ask for: re-encode what we can read.
            _ => None,
        };
    }
    let raw = STANDARD.decode(b).ok()?;
    let side = px
        .filter(|p| (p * p * 3) as usize == raw.len())
        .unwrap_or_else(|| ((raw.len() / 3) as f64).sqrt() as u32);
    let img = image::RgbImage::from_raw(side, side, raw)?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

#[cfg(test)]
mod tests;
