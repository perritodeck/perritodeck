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

use perritodeck_actions::{ActionError, BoxFuture, Outcome, PluginRunner};
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
}

impl Running {
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
    dir: PathBuf,
    host_version: String,
    values: ValueSink,
    plugins: Mutex<HashMap<String, Arc<Running>>>,
    broken: Mutex<Vec<BrokenPlugin>>,
}

impl PluginHost {
    /// Start every plugin installed in `dir` (one folder per plugin).
    pub fn start(dir: PathBuf, host_version: &str, values: ValueSink) -> Arc<Self> {
        let host = Arc::new(Self {
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
                p.stop.store(true, Ordering::Relaxed);
                if let Some(tx) = p.to_plugin.lock().expect("stdin poisoned").take() {
                    let _ = tx.send(json!({ "event": "shutdown" }).to_string());
                }
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
            let running = Arc::new(Running {
                manifest,
                dir: folder,
                status: Mutex::new(Status::Starting),
                to_plugin: Mutex::new(None),
                pending: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
                stop: AtomicBool::new(false),
            });
            plugins.insert(id, running.clone());
            tokio::spawn(supervise(
                running,
                self.host_version.clone(),
                self.values.clone(),
            ));
        }
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
        if dest.exists() {
            std::fs::remove_dir_all(&dest)
                .map_err(|e| format!("Couldn't replace the old version: {e}"))?;
        }
        copy_dir(src, &dest).map_err(|e| format!("Couldn't copy the plugin: {e}"))?;
        // An updated plugin is a new manifest, so scan restarts it.
        self.scan();
        Ok(manifest.id)
    }

    /// Stop a plugin and delete its folder.
    pub fn remove(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let folder = self
            .plugins
            .lock()
            .expect("plugins poisoned")
            .get(id)
            .map(|p| p.dir.clone())
            .ok_or_else(|| format!("There's no plugin called {id}."))?;
        std::fs::remove_dir_all(&folder)
            .map_err(|e| format!("Couldn't remove {}: {e}", folder.display()))?;
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
        p.stop.store(true, Ordering::Relaxed);
        if let Some(tx) = p.to_plugin.lock().expect("stdin poisoned").take() {
            let _ = tx.send(json!({ "event": "shutdown" }).to_string());
        }
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

    /// Stop every plugin (the host is quitting).
    pub fn shutdown(&self) {
        for p in self.plugins.lock().expect("plugins poisoned").values() {
            p.stop.store(true, Ordering::Relaxed);
            if let Some(tx) = p.to_plugin.lock().expect("stdin poisoned").take() {
                let _ = tx.send(json!({ "event": "shutdown" }).to_string());
            }
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

/// The program and arguments that run a plugin.
fn command_for(m: &Manifest, dir: &Path) -> Vec<(String, Vec<String>)> {
    let main = dir.join(&m.main).display().to_string();
    match m.runtime {
        Runtime::Node => vec![("node".into(), vec![main])],
        Runtime::Python if cfg!(windows) => vec![
            ("py".into(), vec!["-3".into(), main.clone()]),
            ("python".into(), vec![main]),
        ],
        Runtime::Python => vec![
            ("python3".into(), vec![main.clone()]),
            ("python".into(), vec![main]),
        ],
        Runtime::Exec => vec![(main, vec![])],
    }
}

async fn supervise(p: Arc<Running>, host_version: String, values: ValueSink) {
    let mut quick_restarts = 0u32;
    loop {
        if p.stop.load(Ordering::Relaxed) {
            p.set_status(Status::Stopped);
            return;
        }
        let started = Instant::now();
        let mut child = None;
        for (program, args) in command_for(&p.manifest, &p.dir) {
            match Command::new(&program)
                .args(&args)
                .current_dir(&p.dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
            {
                Ok(c) => {
                    child = Some(c);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    warn!("plugin {} couldn't start: {e}", p.manifest.id);
                    break;
                }
            }
        }
        let Some(mut child) = child else {
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
        let _ = tx.send(
            json!({ "event": "hello", "protocol": PROTOCOL, "host": host_version }).to_string(),
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

        let status = child.wait().await;
        reader.abort();
        writer.abort();
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
        tokio::time::sleep(Duration::from_millis(BACKOFF_MS * 2u64.pow(quick_restarts))).await;
    }
}

fn handle(p: &Running, msg: &Value, values: &ValueSink, tx: &mpsc::UnboundedSender<String>) {
    let name = &p.manifest.name;
    match msg["event"].as_str() {
        Some("ready") => {
            *p.to_plugin.lock().expect("stdin poisoned") = Some(tx.clone());
            p.set_status(Status::Running);
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
            if let (Some(id), Some(v)) = (msg["id"].as_str(), msg["value"].as_str()) {
                if p.manifest.values.iter().any(|d| d.id == id) {
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
