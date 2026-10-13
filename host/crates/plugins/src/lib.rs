//! Runs PerritoDeck plugins: separate programs the host starts and talks to over
//! stdin/stdout, one JSON object per line (see `plugins/PROTOCOL.md`).
//!
//! Each installed plugin gets a supervisor that starts it, restarts it with a growing
//! delay if it exits (giving up after a few quick crashes), answers `run` requests
//! from key presses, and passes the live values it publishes to the host.

pub mod manifest;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use perritodeck_actions::{job::ProcessTree, ActionError, BoxFuture, Outcome, PluginRunner};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::{mpsc, oneshot},
};
use tracing::{debug, error, info, warn};

pub use manifest::{Manifest, Runtime, MANIFEST};

pub const PROTOCOL: u32 = 1;
const RUN_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_QUICK_RESTARTS: u32 = 5;
/// First restart delay; doubles after each quick crash.
const BACKOFF_MS: u64 = if cfg!(test) { 10 } else { 500 };
/// A run longer than this resets the crash count.
const STABLE_AFTER: Duration = Duration::from_secs(30);

/// Called with (`plugin:<plugin id>:<value id>`, value) when a plugin publishes a value.
pub type ValueSink = Arc<dyn Fn(String, String) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    Starting,
    Running,
    /// Its runtime (node, python) isn't installed.
    #[serde(rename_all = "camelCase")]
    MissingRuntime {
        runtime: String,
    },
    /// It kept exiting; the host stopped restarting it.
    #[serde(rename_all = "camelCase")]
    Crashed {
        reason: String,
    },
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub manifest: Manifest,
    pub status: Status,
    pub folder: String,
}

/// A manifest that couldn't be read, so the UI can say what's wrong.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokenPlugin {
    pub folder: String,
    pub problem: String,
}

struct Running {
    manifest: Manifest,
    dir: PathBuf,
    status: Mutex<Status>,
    to_plugin: Mutex<Option<mpsc::UnboundedSender<String>>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Outcome, ActionError>>>>,
    next_id: AtomicU64,
    stop: AtomicBool,
    /// The plugin's settings (from the Plugins page).
    settings: Mutex<Value>,
    /// Argument values profiles use, e.g. `up:192.168.1.10`, sent as `watch`.
    watching: Mutex<Vec<String>>,
    /// The plugin's own writable folder (`PERRITODECK_PLUGIN_DATA`).
    data_dir: PathBuf,
    /// Wakes the supervisor to stop the plugin (killing it if it doesn't exit).
    stop_signal: tokio::sync::Notify,
    /// The supervisor has finished: no process is running for this plugin.
    exited: AtomicBool,
}

/// How long a plugin gets to exit after `shutdown` before it's killed.
const STOP_GRACE: Duration = Duration::from_secs(2);

impl Running {
    fn new(manifest: Manifest, dir: PathBuf, settings: Value, data_dir: PathBuf) -> Self {
        Self {
            manifest,
            dir,
            status: Mutex::new(Status::Starting),
            to_plugin: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            stop: AtomicBool::new(false),
            settings: Mutex::new(settings),
            watching: Mutex::new(Vec::new()),
            data_dir,
            stop_signal: tokio::sync::Notify::new(),
            exited: AtomicBool::new(false),
        }
    }

    /// Ask the plugin to stop: no more actions go to it, and the supervisor sends it
    /// `shutdown` (ready or not), then kills it if it doesn't exit in time.
    fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.to_plugin.lock().expect("stdin poisoned").take();
        self.stop_signal.notify_one();
    }

    /// Wait (blocking, up to `limit`) until the plugin's process is gone, so its folder
    /// can be deleted (Windows can't delete a running program).
    fn wait_exited(&self, limit: Duration) {
        let until = Instant::now() + limit;
        while !self.exited.load(Ordering::Relaxed) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn send(&self, msg: Value) {
        if let Some(tx) = self.to_plugin.lock().expect("stdin poisoned").as_ref() {
            let _ = tx.send(msg.to_string());
        }
    }

    /// Does the plugin publish this value (`id`, or `id:<argument>`)?
    fn publishes(&self, value: &str) -> bool {
        self.manifest
            .values
            .iter()
            .any(|d| match value.split_once(':') {
                Some((id, arg)) => d.arg.is_some() && d.id == id && !arg.is_empty(),
                None => d.id == value,
            })
    }

    fn set_status(&self, s: Status) {
        *self.status.lock().expect("status poisoned") = s;
    }

    fn fail_pending(&self, why: &str) {
        for (_, tx) in self.pending.lock().expect("pending poisoned").drain() {
            let _ = tx.send(Err(plugin_err(&self.manifest.name, why)));
        }
    }
}

fn plugin_err(name: &str, reason: impl Into<String>) -> ActionError {
    ActionError::Integration {
        service: name.to_owned(),
        reason: reason.into(),
    }
}

pub struct PluginHost {
    /// The runtime plugins are supervised on, so installs from any thread (a UI
    /// command, say) can start them.
    runtime: tokio::runtime::Handle,
    dir: PathBuf,
    host_version: String,
    values: ValueSink,
    plugins: Mutex<HashMap<String, Arc<Running>>>,
    broken: Mutex<Vec<BrokenPlugin>>,
    /// Plugin settings, one JSON file per plugin (kept when a plugin is updated).
    settings_dir: PathBuf,
    /// The live value names last passed to [`PluginHost::watch`], for plugins that start later.
    last_watch: Mutex<Vec<String>>,
}

impl PluginHost {
    /// Start every plugin installed in `dir` (one folder per plugin). Call from
    /// inside a Tokio runtime.
    pub fn start(dir: PathBuf, host_version: &str, values: ValueSink) -> Arc<Self> {
        let settings_dir = dir
            .parent()
            .map_or_else(|| dir.join(".settings"), |p| p.join("plugin-settings"));
        let host = Arc::new(Self {
            runtime: tokio::runtime::Handle::current(),
            settings_dir,
            last_watch: Mutex::new(Vec::new()),
            dir,
            host_version: host_version.to_owned(),
            values,
            plugins: Mutex::new(HashMap::new()),
            broken: Mutex::new(Vec::new()),
        });
        host.scan();
        host
    }

    /// (Re)read the plugins folder: start new plugins, stop removed ones.
    pub fn scan(self: &Arc<Self>) {
        let mut found = HashMap::new();
        let mut broken = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let folder = entry.path();
                let manifest_path = folder.join(MANIFEST);
                if !manifest_path.is_file() {
                    continue;
                }
                match std::fs::read_to_string(&manifest_path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| Manifest::parse(&t))
                {
                    Ok(m) => {
                        found.insert(m.id.clone(), (m, folder));
                    }
                    Err(problem) => broken.push(BrokenPlugin {
                        folder: folder.display().to_string(),
                        problem,
                    }),
                }
            }
        }
        *self.broken.lock().expect("broken poisoned") = broken;
        let mut plugins = self.plugins.lock().expect("plugins poisoned");
        plugins.retain(|id, p| {
            let keep = found.get(id).is_some_and(|(m, _)| *m == p.manifest);
            if !keep {
                p.request_stop();
            }
            keep
        });
        for (id, (manifest, folder)) in found {
            if plugins.contains_key(&id) {
                continue;
            }
            info!(
                "plugin {} {} ({})",
                manifest.name,
                manifest.version,
                folder.display()
            );
            let running = Arc::new(Running::new(
                manifest,
                folder,
                self.load_settings(&id),
                self.settings_dir.with_file_name("plugin-data").join(&id),
            ));
            plugins.insert(id, running.clone());
            self.runtime.spawn(supervise(
                running,
                self.host_version.clone(),
                self.values.clone(),
            ));
        }
        drop(plugins);
        // New and restarted plugins learn what profiles show (sent once they're ready).
        let last = self.last_watch.lock().expect("watch poisoned").clone();
        self.watch(&last);
    }

    pub fn list(&self) -> Vec<PluginInfo> {
        let mut out: Vec<PluginInfo> = self
            .plugins
            .lock()
            .expect("plugins poisoned")
            .values()
            .map(|p| PluginInfo {
                manifest: p.manifest.clone(),
                status: p.status.lock().expect("status poisoned").clone(),
                folder: p.dir.display().to_string(),
            })
            .collect();
        out.sort_by(|a, b| {
            a.manifest
                .name
                .to_lowercase()
                .cmp(&b.manifest.name.to_lowercase())
        });
        out
    }

    pub fn broken(&self) -> Vec<BrokenPlugin> {
        self.broken.lock().expect("broken poisoned").clone()
    }

    pub fn folder(&self) -> &Path {
        &self.dir
    }

    /// Copy a plugin folder into the plugins folder and start it. Returns its id.
    pub fn install_from(self: &Arc<Self>, src: &Path) -> Result<String, String> {
        let text = std::fs::read_to_string(src.join(MANIFEST)).map_err(|_| {
            format!(
                "{} has no {MANIFEST}; that isn't a PerritoDeck plugin.",
                src.display()
            )
        })?;
        let manifest = Manifest::parse(&text)?;
        let dest = self.dir.join(&manifest.id);
        // Stop the old version first: a running program can't be deleted on Windows.
        let old = self
            .plugins
            .lock()
            .expect("plugins poisoned")
            .remove(&manifest.id);
        if let Some(old) = old {
            old.request_stop();
            old.wait_exited(STOP_GRACE * 2);
        }
        if dest.exists() {
            remove_folder(&dest).map_err(|e| format!("Couldn't replace the old version: {e}"))?;
        }
        copy_dir(src, &dest).map_err(|e| format!("Couldn't copy the plugin: {e}"))?;
        // An updated plugin is a new manifest, so scan restarts it.
        self.scan();
        Ok(manifest.id)
    }

    /// Stop a plugin and delete its folder.
    pub fn remove(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let p = self
            .plugins
            .lock()
            .expect("plugins poisoned")
            .remove(id)
            .ok_or_else(|| format!("There's no plugin called {id}."))?;
        p.request_stop();
        p.wait_exited(STOP_GRACE * 2);
        let folder = p.dir.clone();
        remove_folder(&folder).map_err(|e| format!("Couldn't remove {}: {e}", folder.display()))?;
        self.scan();
        Ok(())
    }

    /// Restart a plugin (after it crashed, or after editing it).
    pub fn restart(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let p = self
            .plugins
            .lock()
            .expect("plugins poisoned")
            .remove(id)
            .ok_or_else(|| format!("There's no plugin called {id}."))?;
        p.request_stop();
        self.scan();
        Ok(())
    }

    async fn run(
        &self,
        plugin: &str,
        action: &str,
        settings: &Value,
    ) -> Result<Outcome, ActionError> {
        let p = self
            .plugins
            .lock()
            .expect("plugins poisoned")
            .get(plugin)
            .cloned()
            .ok_or_else(|| plugin_err(plugin, "this plugin isn't installed"))?;
        let name = p.manifest.name.clone();
        if !p.manifest.actions.iter().any(|a| a.id == action) {
            return Err(plugin_err(&name, format!("it has no action {action:?}")));
        }
        let tx = p
            .to_plugin
            .lock()
            .expect("stdin poisoned")
            .clone()
            .ok_or_else(|| plugin_err(&name, "it isn't running"))?;
        let id = p.next_id.fetch_add(1, Ordering::Relaxed);
        let (done, result) = oneshot::channel();
        p.pending.lock().expect("pending poisoned").insert(id, done);
        let msg = json!({ "event": "run", "id": id, "action": action, "settings": settings });
        if tx.send(msg.to_string()).is_err() {
            p.pending.lock().expect("pending poisoned").remove(&id);
            return Err(plugin_err(&name, "it isn't running"));
        }
        match tokio::time::timeout(RUN_TIMEOUT, result).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(plugin_err(&name, "it stopped before answering")),
            Err(_) => {
                p.pending.lock().expect("pending poisoned").remove(&id);
                Err(plugin_err(&name, "it didn't answer within 10 seconds"))
            }
        }
    }

    fn settings_path(&self, id: &str) -> PathBuf {
        self.settings_dir.join(format!("{id}.json"))
    }

    fn load_settings(&self, id: &str) -> Value {
        std::fs::read(self.settings_path(id))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}))
    }

    /// A plugin's settings, as saved on the Plugins page.
    pub fn settings(&self, id: &str) -> Result<Value, String> {
        let p = self.get(id)?;
        let s = p.settings.lock().expect("settings poisoned").clone();
        Ok(s)
    }

    /// Save a plugin's settings and send them to it. Only settings its manifest declares
    /// are kept.
    pub fn set_settings(&self, id: &str, settings: Value) -> Result<(), String> {
        let p = self.get(id)?;
        let Value::Object(given) = settings else {
            return Err("Settings must be an object.".into());
        };
        let kept: serde_json::Map<String, Value> = given
            .into_iter()
            .filter(|(k, _)| p.manifest.settings.iter().any(|f| &f.id == k))
            .collect();
        let value = Value::Object(kept);
        std::fs::create_dir_all(&self.settings_dir).map_err(|e| e.to_string())?;
        let path = self.settings_path(id);
        let tmp = path.with_extension("json.tmp");
        // They may hold passwords or tokens: readable by this user only, from the start.
        write_private(&tmp, &serde_json::to_vec_pretty(&value).expect("JSON"))
            .map_err(|e| format!("Couldn't save the settings: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("Couldn't save the settings: {e}"))?;
        *p.settings.lock().expect("settings poisoned") = value.clone();
        p.send(json!({ "event": "settings", "settings": value }));
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Arc<Running>, String> {
        self.plugins
            .lock()
            .expect("plugins poisoned")
            .get(id)
            .cloned()
            .ok_or_else(|| format!("There's no plugin called {id}."))
    }

    /// Tell plugins which of their argument values profiles use, given the live value
    /// names in use (`plugin:<id>:<value>:<argument>`). Each plugin hears only about
    /// changes to its own list.
    pub fn watch(&self, tokens: &[String]) {
        *self.last_watch.lock().expect("watch poisoned") = tokens.to_vec();
        let mut wanted: HashMap<&str, Vec<String>> = HashMap::new();
        for t in tokens {
            let Some(rest) = t.strip_prefix("plugin:") else {
                continue;
            };
            let Some((plugin, value)) = rest.split_once(':') else {
                continue;
            };
            if value.contains(':') {
                wanted.entry(plugin).or_default().push(value.to_owned());
            }
        }
        for (id, p) in self.plugins.lock().expect("plugins poisoned").iter() {
            let mut list = wanted.remove(id.as_str()).unwrap_or_default();
            list.retain(|v| p.publishes(v));
            list.sort();
            list.dedup();
            let mut watching = p.watching.lock().expect("watching poisoned");
            if *watching != list {
                *watching = list.clone();
                drop(watching);
                p.send(json!({ "event": "watch", "values": list }));
            }
        }
    }

    /// Stop every plugin (the host is quitting).
    pub fn shutdown(&self) {
        for p in self.plugins.lock().expect("plugins poisoned").values() {
            p.request_stop();
        }
    }

    /// Stop every plugin and wait (blocking, up to `limit`) for their processes to end,
    /// for an app that's about to exit.
    pub fn shutdown_and_wait(&self, limit: Duration) {
        self.shutdown();
        let until = Instant::now() + limit;
        let running: Vec<Arc<Running>> = self
            .plugins
            .lock()
            .expect("plugins poisoned")
            .values()
            .cloned()
            .collect();
        for p in running {
            p.wait_exited(until.saturating_duration_since(Instant::now()));
        }
    }
}

impl PluginRunner for PluginHost {
    fn run_plugin<'a>(
        &'a self,
        plugin: &'a str,
        action: &'a str,
        settings: &'a Value,
    ) -> BoxFuture<'a, Result<Outcome, ActionError>> {
        Box::pin(self.run(plugin, action, settings))
    }
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let to = dest.join(entry.file_name());
        if kind.is_dir() {
            // Skip the heavy, rebuildable folders.
            if matches!(entry.file_name().to_str(), Some(".git" | "__pycache__")) {
                continue;
            }
            copy_dir(&entry.path(), &to)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

/// Delete a plugin's folder. On Windows a process that was just ended can keep a file
/// in it busy for a moment longer (its program, or its working folder), so retry for
/// up to a second before giving up.
fn remove_folder(dir: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::remove_dir_all(dir) {
            Err(e) if tries < 20 && still_in_use(&e) => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            done => return done,
        }
    }
}

/// Whether deleting failed because a file is still in use (Windows: access denied,
/// sharing violation, or a folder whose files are still being deleted).
fn still_in_use(e: &std::io::Error) -> bool {
    cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32 | 145))
}

/// The programs and arguments that may run a plugin, in the order to try them.
async fn command_for(m: &Manifest, dir: &Path) -> Vec<(String, Vec<String>)> {
    let main = dir.join(&m.main).display().to_string();
    match m.runtime {
        Runtime::Node => vec![("node".into(), vec![main])],
        // None when there's no Python 3, so the plugin shows as missing its runtime.
        Runtime::Python if cfg!(windows) => windows_python()
            .await
            .map(|python| (python, vec![main]))
            .into_iter()
            .collect(),
        Runtime::Python => vec![
            ("python3".into(), vec![main.clone()]),
            ("python".into(), vec![main]),
        ],
        Runtime::Exec => vec![(main, vec![])],
    }
}

/// Where to look for Python 3 on Windows, in order: the launcher's choice (`py -3`),
/// then `python` on PATH.
const WINDOWS_PYTHONS: &[(&str, &[&str])] = &[("py", &["-3"]), ("python", &[])];

/// Prints the interpreter's own path, or nothing if it's Python 2.
const PRINT_EXECUTABLE: &str =
    "import sys; sys.stdout.write(sys.executable if sys.version_info[0] >= 3 else '')";

/// The `python.exe` Python plugins run with on Windows, found once and remembered.
///
/// Plugins run Python itself rather than `py -3`. The launcher runs Python as a child
/// of its own, in a job of its own that lets Python's children break away, so the
/// process the host watched and killed was only the launcher. Looking first also tells
/// a missing Python from a crashing plugin: `py` exits with 103 when it knows no
/// Python 3, and `python` may be the Microsoft Store shortcut, which exits with 9009.
async fn windows_python() -> Option<String> {
    static FOUND: tokio::sync::Mutex<Option<PathBuf>> = tokio::sync::Mutex::const_new(None);
    // Held while looking, so plugins starting together look only once.
    let mut found = FOUND.lock().await;
    // Look again if that Python has gone (uninstalled, or replaced by an upgrade).
    if !found.as_deref().is_some_and(exists) {
        *found = find_python(WINDOWS_PYTHONS).await;
    }
    found.as_ref().map(|p| p.display().to_string())
}

/// The first of `candidates` (program, arguments) that is Python 3, as the path of
/// its interpreter.
async fn find_python(candidates: &[(&str, &[&str])]) -> Option<PathBuf> {
    for (program, args) in candidates {
        let mut cmd = Command::new(program);
        cmd.args(*args)
            .args(["-c", PRINT_EXECUTABLE])
            // A path with non-ASCII letters (a user name) must come back intact.
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let out = match tokio::time::timeout(Duration::from_secs(10), cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                debug!("{program} isn't Python 3: {e}");
                continue;
            }
            Err(_) => {
                debug!("{program} didn't answer");
                continue;
            }
        };
        match interpreter_from(out.status.success(), &out.stdout) {
            Some(python) => {
                info!("Python plugins run with {}", python.display());
                return Some(python);
            }
            None => debug!("{program} isn't Python 3 ({})", out.status),
        }
    }
    None
}

/// The interpreter a [`PRINT_EXECUTABLE`] run named, if it ran fine and named a file
/// that's there. The Store shortcut fails, and Python 2 prints nothing.
fn interpreter_from(ok: bool, stdout: &[u8]) -> Option<PathBuf> {
    let path = PathBuf::from(String::from_utf8_lossy(stdout).trim());
    (ok && !path.as_os_str().is_empty() && exists(&path)).then_some(path)
}

/// Whether something is at `path`. Not `is_file`: a Microsoft Store Python's path is an
/// app execution alias, a link that can't always be followed.
fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Windows process creation flag: no console window for the process.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Write a file only its owner can read (0600 on Unix, set when it's created).
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Marks a plugin's process as gone when its supervisor returns, however it returns.
struct Exited<'a>(&'a AtomicBool);

impl Drop for Exited<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

async fn supervise(p: Arc<Running>, host_version: String, values: ValueSink) {
    let _exited = Exited(&p.exited);
    let _ = std::fs::create_dir_all(&p.data_dir);
    let mut quick_restarts = 0u32;
    loop {
        if p.stop.load(Ordering::Relaxed) {
            p.set_status(Status::Stopped);
            return;
        }
        let started = Instant::now();
        let mut spawned = None;
        let mut failed = None;
        for (program, args) in command_for(&p.manifest, &p.dir).await {
            let mut cmd = Command::new(&program);
            cmd.args(&args)
                .current_dir(&p.dir)
                // Plugins talk UTF-8 whatever the system's code page is.
                .env("PYTHONUTF8", "1")
                .env("PYTHONIOENCODING", "utf-8")
                .env("PERRITODECK_PLUGIN_DATA", &p.data_dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            // On Windows the plugin and everything it starts end together (a job
            // object), and no console window pops up for each plugin.
            match ProcessTree::spawn(&mut cmd, CREATE_NO_WINDOW) {
                Ok(child_and_tree) => {
                    spawned = Some(child_and_tree);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    warn!("plugin {} couldn't start: {e}", p.manifest.id);
                    failed = Some(e);
                    break;
                }
            }
        }
        let Some((mut child, tree)) = spawned else {
            if let Some(e) = failed {
                // Its program is there but didn't start (not a program, not allowed,
                // or Windows couldn't let it run): say why, not that it's missing.
                p.set_status(Status::Crashed {
                    reason: format!("it couldn't start: {e}"),
                });
                return;
            }
            let runtime = match p.manifest.runtime {
                Runtime::Node => "Node.js",
                Runtime::Python => "Python 3",
                Runtime::Exec => "its program",
            };
            warn!(
                "plugin {} needs {runtime}, which wasn't found",
                p.manifest.id
            );
            p.set_status(Status::MissingRuntime {
                runtime: runtime.into(),
            });
            return;
        };

        let mut stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let settings = p.settings.lock().expect("settings poisoned").clone();
        let _ = tx.send(
            json!({ "event": "hello", "protocol": PROTOCOL, "host": host_version, "settings": settings })
                .to_string(),
        );
        let writer = tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                if stdin
                    .write_all(format!("{line}\n").as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
                let _ = stdin.flush().await;
            }
        });
        let id = p.manifest.id.clone();
        let log_id = id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                info!(plugin = %log_id, "{line}");
            }
        });
        // Actions go to the plugin only after it says it's ready.
        let ready_tx = tx.clone();
        let reader = {
            let p = p.clone();
            let values = values.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                        debug!(plugin = %p.manifest.id, "not JSON: {line}");
                        continue;
                    };
                    handle(&p, &msg, &values, &ready_tx);
                }
            })
        };

        let status = tokio::select! {
            s = child.wait() => s,
            _ = p.stop_signal.notified() => {
                // Asked to stop: say so (even if it isn't ready yet, so it needn't sit
                // out the grace period), give it a moment, then kill it.
                let _ = tx.send(json!({ "event": "shutdown" }).to_string());
                match tokio::time::timeout(STOP_GRACE, child.wait()).await {
                    Ok(s) => s,
                    Err(_) => {
                        warn!("plugin {id} didn't stop; ending it");
                        let _ = child.kill().await;
                        child.wait().await
                    }
                }
            }
        };
        // End whatever the plugin left running, and wait until it's gone, so that
        // `exited` means its folder can be deleted.
        if let Some(tree) = tree {
            tree.end().await;
        }
        reader.abort();
        writer.abort();
        // Wait until the reader has really stopped, so a `ready` it was still handling
        // can't hand out this run's sender again after it's cleared here.
        let _ = reader.await;
        *p.to_plugin.lock().expect("stdin poisoned") = None;
        p.fail_pending("it stopped");
        if p.stop.load(Ordering::Relaxed) {
            p.set_status(Status::Stopped);
            return;
        }
        let reason = match status {
            Ok(s) => format!("it exited ({s})"),
            Err(e) => format!("it couldn't be watched: {e}"),
        };
        if started.elapsed() > STABLE_AFTER {
            quick_restarts = 0;
        }
        quick_restarts += 1;
        if quick_restarts > MAX_QUICK_RESTARTS {
            error!("plugin {id} keeps stopping ({reason}); not restarting it");
            p.set_status(Status::Crashed { reason });
            return;
        }
        warn!("plugin {id} stopped ({reason}); restarting");
        p.set_status(Status::Starting);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(BACKOFF_MS * 2u64.pow(quick_restarts))) => {}
            // Stopped while waiting (Quit, Remove, Update): no need to sit out the delay.
            _ = p.stop_signal.notified() => {}
        }
    }
}

fn handle(p: &Running, msg: &Value, values: &ValueSink, tx: &mpsc::UnboundedSender<String>) {
    let name = &p.manifest.name;
    match msg["event"].as_str() {
        Some("ready") => {
            {
                // Checked and set under one lock: request_stop sets `stop` before it
                // takes this lock to clear the sender, so a stop can't fall in between
                // and leave a plugin that's being stopped taking actions.
                let mut to_plugin = p.to_plugin.lock().expect("stdin poisoned");
                if p.stop.load(Ordering::Relaxed) {
                    return; // being stopped: no more actions for it
                }
                *to_plugin = Some(tx.clone());
                p.set_status(Status::Running);
            }
            // Catch it up on what profiles show (it may have restarted).
            let watching = p.watching.lock().expect("watching poisoned").clone();
            if !watching.is_empty() {
                let _ = tx.send(json!({ "event": "watch", "values": watching }).to_string());
            }
            info!("plugin {} is ready", p.manifest.id);
        }
        Some("result") => {
            let Some(id) = msg["id"].as_u64() else { return };
            let Some(done) = p.pending.lock().expect("pending poisoned").remove(&id) else {
                return;
            };
            let message = msg["message"].as_str().map(str::to_owned);
            let result = match (msg["ok"].as_bool().unwrap_or(false), message) {
                (true, None) => Ok(Outcome::Ok),
                (true, Some(m)) => Ok(Outcome::Alert(m)),
                (false, m) => Err(plugin_err(
                    name,
                    m.unwrap_or_else(|| "it reported a problem".into()),
                )),
            };
            let _ = done.send(result);
        }
        Some("value") => {
            if p.stop.load(Ordering::Relaxed) {
                return; // an old copy that's being stopped
            }
            if let (Some(id), Some(v)) = (msg["id"].as_str(), msg["value"].as_str()) {
                if p.publishes(id) {
                    values(format!("plugin:{}:{id}", p.manifest.id), v.to_owned());
                }
            }
        }
        Some("log") => {
            let text = msg["message"].as_str().unwrap_or_default();
            match msg["level"].as_str() {
                Some("error") => error!(plugin = %p.manifest.id, "{text}"),
                Some("warn") => warn!(plugin = %p.manifest.id, "{text}"),
                _ => info!(plugin = %p.manifest.id, "{text}"),
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
