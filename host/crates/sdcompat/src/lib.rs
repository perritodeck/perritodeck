//! Runs Stream Deck plugins (`.sdPlugin` folders) so their actions work on PerritoDeck
//! keys.
//!
//! Written from Elgato's public plugin documentation: the host starts each plugin with
//! `-port -pluginUUID -registerEvent -info`, the plugin connects back over a WebSocket on
//! 127.0.0.1 and registers, then gets `willAppear`/`keyDown`/`keyUp`/… and answers with
//! `setTitle`/`setImage`/`setState`/`showOk`/`setSettings`/….
//!
//! Each profile key with an `sdPlugin` action is one plugin action instance (context),
//! shared by every tablet showing it, like PerritoDeck's own toggles. Settings a plugin
//! saves are kept in `settings.json` in the plugins folder.
//!
//! Supported: Node.js plugins (the current SDK), native plugins on Windows and macOS, and
//! property inspectors, each action's settings page, which `inspector` serves over HTTP on
//! 127.0.0.1 for the app to show in a frame (settings can also be edited as JSON there).
//! Not supported: dials and touch strips, multi-actions, and the old HTML plugins.

pub mod images;
mod inspector;
pub mod manifest;
mod node;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use perritodeck_deck::{AgentKey, Deck, KeyAgents};
use perritodeck_protocol::Grid;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    net::TcpListener,
    sync::{mpsc, Notify},
};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, info, warn};

pub use manifest::{Code, Manifest};

/// Where installed plugins' saved settings live, inside the plugins folder.
const STORE_FILE: &str = "settings.json";
const MAX_CRASHES: u32 = 5;
/// Windows process creation flag: no console window for the process.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    Starting,
    Running,
    /// Needs Node.js, which isn't installed.
    NeedsNode,
    Unsupported {
        reason: String,
    },
    Crashed {
        reason: String,
    },
}

/// An installed plugin, for the app.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub uuid: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub folder: String,
    pub status: Status,
    /// The plugin's icon as a PNG data URL.
    pub icon: Option<String>,
    /// Actions that work on keys.
    pub actions: Vec<ActionInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionInfo {
    pub uuid: String,
    pub name: String,
    pub tooltip: String,
    pub icon: Option<String>,
    /// It has a settings page (property inspector), which PerritoDeck can't show.
    pub has_settings_page: bool,
}

/// A folder in the plugins folder that couldn't be loaded.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokenPlugin {
    pub folder: String,
    pub problem: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    /// Per key (context) settings saved by plugins.
    #[serde(default)]
    contexts: HashMap<String, Value>,
    /// Per plugin global settings.
    #[serde(default)]
    global: HashMap<String, Value>,
}

struct Plugin {
    manifest: Manifest,
    folder: PathBuf,
    status: Status,
    /// To the plugin's WebSocket, once it has registered.
    out: Option<(u64, mpsc::UnboundedSender<String>)>,
    /// Wakes the plugin's supervisor to stop it. Also tells a supervisor whether this
    /// entry is still its own (after a restart there's a new one).
    stop: Arc<Notify>,
    /// Set when the supervisor is done and the plugin's processes, and any they
    /// started, are gone, so the folder can be deleted.
    exited: Arc<std::sync::atomic::AtomicBool>,
}

struct Instance {
    key: AgentKey,
    state: usize,
    /// Images set by the plugin, per state (`None`: back to the manifest's).
    images: HashMap<usize, Vec<u8>>,
    titles: HashMap<usize, Option<String>>,
}

#[derive(Default)]
struct State {
    plugins: BTreeMap<String, Plugin>,
    /// Plugins being removed or replaced: their folders are still there while they
    /// stop, and `scan` mustn't start them again meanwhile.
    removing: HashSet<String>,
    broken: Vec<BrokenPlugin>,
    devices: BTreeMap<String, Grid>,
    instances: HashMap<String, Instance>,
    store: Store,
    /// Keys whose settings page the app asked for (context → key).
    pi_pending: HashMap<String, AgentKey>,
    /// The settings page that's open.
    pi: Option<inspector::PiConn>,
}

/// Changes for the deck, applied in order from one task (never while the deck is
/// calling us, which would deadlock).
enum Draw {
    Look {
        context: String,
        title: Option<Option<String>>,
        image: Option<Option<Vec<u8>>>,
    },
    Feedback {
        context: String,
        ok: bool,
    },
}

struct Inner {
    /// The runtime plugins are supervised on, so a restart or install from any thread
    /// (a UI command, say) can start them.
    runtime: tokio::runtime::Handle,
    dir: PathBuf,
    port: u16,
    state: Mutex<State>,
    deck: OnceLock<Weak<Deck>>,
    draws: mpsc::UnboundedSender<Draw>,
    /// The Node.js program, used as given. `None`: `node` from PATH, else wherever
    /// [`node::find`] finds it.
    node: Option<String>,
    connections: AtomicU64,
    /// Where settings pages are served, and the secret in their URLs.
    http_port: u16,
    pi_secret: String,
}

/// The Stream Deck plugin host. Cheap to clone.
#[derive(Clone)]
pub struct SdHost(Arc<Inner>);

impl SdHost {
    /// Start the WebSocket server on 127.0.0.1 and every plugin in `dir`.
    pub async fn start(dir: PathBuf) -> std::io::Result<Self> {
        Self::start_inner(dir, None).await
    }

    /// Like [`SdHost::start`], with a different Node.js program (tests), used exactly
    /// as given.
    pub async fn start_with_node(dir: PathBuf, node: String) -> std::io::Result<Self> {
        Self::start_inner(dir, Some(node)).await
    }

    async fn start_inner(dir: PathBuf, node: Option<String>) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let http = TcpListener::bind("127.0.0.1:0").await?;
        let http_port = http.local_addr()?.port();
        let pi_secret: String = rand::random::<[u8; 16]>()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let store = std::fs::read(dir.join(STORE_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let (draws, mut draw_rx) = mpsc::unbounded_channel::<Draw>();
        let inner = Arc::new(Inner {
            runtime: tokio::runtime::Handle::current(),
            dir,
            port,
            state: Mutex::new(State {
                store,
                ..Default::default()
            }),
            deck: OnceLock::new(),
            draws,
            node,
            connections: AtomicU64::new(0),
            http_port,
            pi_secret,
        });
        tokio::spawn(inspector::serve(Arc::downgrade(&inner), http));
        let weak = Arc::downgrade(&inner);
        tokio::spawn(async move {
            while let Some(d) = draw_rx.recv().await {
                let Some(inner) = weak.upgrade() else { return };
                let Some(deck) = inner.deck.get().and_then(Weak::upgrade) else {
                    continue;
                };
                match d {
                    Draw::Look {
                        context,
                        title,
                        image,
                    } => deck.agent_draw(&context, title, image),
                    Draw::Feedback { context, ok } => deck.agent_feedback(&context, ok),
                }
            }
        });
        let accept = Arc::downgrade(&inner);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let Some(inner) = accept.upgrade() else {
                    return;
                };
                tokio::spawn(connection(inner, stream));
            }
        });
        info!("Stream Deck plugins: listening on 127.0.0.1:{port}");
        let host = Self(inner);
        host.scan();
        Ok(host)
    }

    /// Run the plugins' keys on `deck`.
    pub fn attach(&self, deck: &Arc<Deck>) {
        let _ = self.0.deck.set(Arc::downgrade(deck));
        deck.set_agents(Arc::new(self.clone()));
    }

    pub fn folder(&self) -> &Path {
        &self.0.dir
    }

    /// Load plugins added since the last look, and forget removed ones.
    pub fn scan(&self) {
        let mut found = BTreeMap::new();
        let mut broken = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.0.dir) {
            for e in entries.flatten() {
                let folder = e.path();
                let hidden = e.file_name().to_string_lossy().starts_with('.');
                if !folder.is_dir() || hidden {
                    continue;
                }
                match Manifest::read(&folder) {
                    Ok(m) => {
                        found.insert(m.uuid.clone(), (m, folder));
                    }
                    Err(problem) => broken.push(BrokenPlugin {
                        folder: folder.display().to_string(),
                        problem,
                    }),
                }
            }
        }
        let mut start = Vec::new();
        {
            let mut st = self.0.state.lock().expect("state poisoned");
            st.broken = broken;
            found.retain(|uuid, _| !st.removing.contains(uuid));
            let gone: Vec<String> = st
                .plugins
                .keys()
                .filter(|k| !found.contains_key(*k))
                .cloned()
                .collect();
            for uuid in gone {
                if let Some(p) = st.plugins.remove(&uuid) {
                    stop(&p);
                }
            }
            for (uuid, (manifest, folder)) in found {
                let changed = st
                    .plugins
                    .get(&uuid)
                    .map_or(true, |p| p.manifest != manifest || p.folder != folder);
                if !changed {
                    continue;
                }
                if let Some(p) = st.plugins.remove(&uuid) {
                    stop(&p);
                }
                st.plugins.insert(
                    uuid.clone(),
                    Plugin {
                        manifest,
                        folder,
                        status: Status::Starting,
                        out: None,
                        stop: Arc::new(Notify::new()),
                        exited: Arc::default(),
                    },
                );
                start.push(uuid);
            }
        }
        for uuid in start {
            self.0.runtime.spawn(supervise(self.0.clone(), uuid));
        }
    }

    pub fn list(&self) -> Vec<PluginInfo> {
        let st = self.0.state.lock().expect("state poisoned");
        st.plugins
            .values()
            .map(|p| {
                let icon = |base: &Option<String>| {
                    base.as_deref()
                        .and_then(|b| images::load(b, &p.folder))
                        .map(|png| format!("data:image/png;base64,{}", STANDARD.encode(png)))
                };
                PluginInfo {
                    uuid: p.manifest.uuid.clone(),
                    name: p.manifest.name.clone(),
                    version: p.manifest.version.clone(),
                    author: p.manifest.author.clone(),
                    description: p.manifest.description.clone(),
                    folder: p.folder.display().to_string(),
                    status: p.status.clone(),
                    icon: icon(&p.manifest.icon),
                    actions: p
                        .manifest
                        .actions
                        .iter()
                        .filter(|a| a.on_keys())
                        .map(|a| ActionInfo {
                            uuid: a.uuid.clone(),
                            name: a.name.clone(),
                            tooltip: a.tooltip.clone().unwrap_or_default(),
                            icon: icon(&a.icon),
                            has_settings_page: a.property_inspector_path.is_some(),
                        })
                        .collect(),
                }
            })
            .collect()
    }

    pub fn broken(&self) -> Vec<BrokenPlugin> {
        self.0.state.lock().expect("state poisoned").broken.clone()
    }

    /// Install (or update) a plugin from a `.sdPlugin` folder or a `.streamDeckPlugin`
    /// file. Returns its UUID.
    pub fn install(&self, src: &Path) -> Result<String, String> {
        let tmp = tempdir_in(&self.0.dir)?;
        let result = self.install_via(src, &tmp);
        let _ = std::fs::remove_dir_all(&tmp);
        result
    }

    fn install_via(&self, src: &Path, tmp: &Path) -> Result<String, String> {
        let folder = if src.is_dir() {
            src.to_owned()
        } else {
            let bytes =
                std::fs::read(src).map_err(|e| format!("Couldn't read {}: {e}", src.display()))?;
            unzip(&bytes, tmp)?;
            find_plugin_folder(tmp)?
        };
        let manifest =
            Manifest::read(&folder).map_err(|p| format!("That plugin can't be used: {p}."))?;
        let dest = self.0.dir.join(format!("{}.sdPlugin", manifest.uuid));
        // Stop the old version first: Windows can't delete a running program.
        let (old, replacing) = self.0.take_out(&manifest.uuid);
        if let Some(old) = old {
            wait_exited(&old);
        }
        if dest.exists() {
            remove_folder(&dest).map_err(|e| format!("Couldn't replace the old version: {e}"))?;
        }
        copy_dir(&folder, &dest).map_err(|e| format!("Couldn't copy the plugin: {e}"))?;
        drop(replacing);
        self.scan();
        Ok(manifest.uuid)
    }

    pub fn remove(&self, uuid: &str) -> Result<(), String> {
        let (p, _removing) = self.0.take_out(uuid);
        let p = p.ok_or_else(|| format!("There's no Stream Deck plugin {uuid}."))?;
        wait_exited(&p);
        remove_folder(&p.folder).map_err(|e| format!("Couldn't remove {}: {e}", p.folder.display()))
    }

    pub fn restart(&self, uuid: &str) -> Result<(), String> {
        {
            let mut st = self.0.state.lock().expect("state poisoned");
            let p = st
                .plugins
                .remove(uuid)
                .ok_or_else(|| format!("There's no Stream Deck plugin {uuid}."))?;
            stop(&p);
        }
        self.scan();
        Ok(())
    }

    /// Stop every plugin (this doesn't wait; see [`SdHost::wait_stopped`]).
    pub fn shutdown(&self) {
        let st = self.0.state.lock().expect("state poisoned");
        for p in st.plugins.values() {
            stop(p);
        }
    }

    /// Wait (blocking, up to `limit`) until the processes of every plugin stopped by
    /// [`SdHost::shutdown`] are gone, with the programs they started, for an app that's
    /// about to exit.
    pub fn wait_stopped(&self, limit: Duration) {
        let until = Instant::now() + limit;
        let exited: Vec<_> = {
            let st = self.0.state.lock().expect("state poisoned");
            st.plugins.values().map(|p| p.exited.clone()).collect()
        };
        while exited.iter().any(|e| !e.load(Ordering::Relaxed)) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The address of a key's Stream Deck settings page (property inspector), for the
    /// app to show in a frame. `key` comes from `Deck::agent_key_for`.
    pub fn settings_page(&self, key: &AgentKey) -> Result<String, String> {
        inspector::url_for(&self.0, key)
    }

    /// The WebSocket port plugins connect to.
    pub fn port(&self) -> u16 {
        self.0.port
    }
}

fn stop(p: &Plugin) {
    p.stop.notify_one();
}

/// Wait (blocking, a few seconds at most) for a stopped plugin's process to be gone.
fn wait_exited(p: &Plugin) {
    let until = Instant::now() + Duration::from_secs(4);
    while !p.exited.load(Ordering::Relaxed) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Keeps a plugin that's being removed or replaced out of `scan` until it's dropped.
struct Removing<'a> {
    inner: &'a Inner,
    uuid: String,
}

impl Drop for Removing<'_> {
    fn drop(&mut self) {
        if let Ok(mut st) = self.inner.state.lock() {
            st.removing.remove(&self.uuid);
        }
    }
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

/// Settings plugins save are kept per plugin action and key, so a different action put
/// on the same key never sees them.
fn store_key(key: &AgentKey) -> String {
    format!("{}|{}|{}", key.plugin, key.action, key.context)
}

/// Write a file only its owner can read (0600 on Unix, from creation).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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

impl Inner {
    /// Take a plugin out of the list to remove or replace it, and stop it. Until the
    /// guard is dropped, `scan` leaves its folder alone rather than start it again.
    fn take_out(&self, uuid: &str) -> (Option<Plugin>, Removing<'_>) {
        let mut st = self.state.lock().expect("state poisoned");
        st.removing.insert(uuid.to_owned());
        let p = st.plugins.remove(uuid);
        if let Some(p) = &p {
            stop(p);
        }
        let guard = Removing {
            inner: self,
            uuid: uuid.to_owned(),
        };
        (p, guard)
    }

    fn send(&self, uuid: &str, msg: Value) {
        let st = self.state.lock().expect("state poisoned");
        if let Some((_, tx)) = st.plugins.get(uuid).and_then(|p| p.out.as_ref()) {
            let _ = tx.send(msg.to_string());
        }
    }

    fn save_store(&self, st: &State) {
        let path = self.dir.join(STORE_FILE);
        match serde_json::to_vec_pretty(&st.store) {
            Ok(b) => {
                let tmp = path.with_extension("json.tmp");
                if write_private(&tmp, &b)
                    .and_then(|_| std::fs::rename(&tmp, &path))
                    .is_err()
                {
                    warn!(
                        "couldn't save Stream Deck plugin settings to {}",
                        path.display()
                    );
                }
            }
            Err(e) => warn!("couldn't save Stream Deck plugin settings: {e}"),
        }
    }

    fn info(&self, st: &State, p: &Plugin) -> Value {
        let platform = if cfg!(target_os = "macos") {
            "mac"
        } else {
            "windows"
        };
        json!({
            "application": {
                "font": if cfg!(target_os = "macos") { ".AppleSystemUIFont" } else { "Segoe UI" },
                "language": "en",
                "platform": platform,
                "platformVersion": platform_version(),
                "version": "6.9.0",
            },
            "plugin": { "uuid": p.manifest.uuid, "version": p.manifest.version },
            "devicePixelRatio": 2,
            "colors": {
                "buttonMouseOverBackgroundColor": "#464646FF",
                "buttonPressedBackgroundColor": "#303030FF",
                "buttonPressedBorderColor": "#646464FF",
                "buttonPressedTextColor": "#969696FF",
                "highlightColor": "#0078FFFF",
            },
            "devices": st.devices.iter().map(|(id, g)| device_info(id, *g)).collect::<Vec<_>>(),
        })
    }
}

fn device_info(id: &str, g: Grid) -> Value {
    json!({
        "id": id,
        "name": "PerritoDeck tablet",
        "size": { "columns": g.cols, "rows": g.rows },
        // Stream Deck Mobile: the closest thing to a tablet.
        "type": 3,
    })
}

/// The settings a key's plugin action sees: what it saved, else the profile's.
fn settings_of(st: &State, key: &AgentKey) -> Value {
    st.store
        .contexts
        .get(&store_key(key))
        .cloned()
        .or_else(|| key.settings.is_object().then(|| key.settings.clone()))
        .unwrap_or_else(|| json!({}))
}

fn action_event(st: &State, event: &str, inst: &Instance) -> Value {
    let k = &inst.key;
    json!({
        "event": event,
        "action": k.action,
        "context": k.context,
        "device": k.device,
        "payload": {
            "controller": "Keypad",
            "coordinates": { "column": k.col, "row": k.row },
            "isInMultiAction": false,
            "settings": settings_of(st, k),
            "state": inst.state,
        },
    })
}

/// The manifest's image for an action's state, as PNG.
fn default_image(p: &Plugin, action: &str, state: usize) -> Option<Vec<u8>> {
    let a = p.manifest.action(action)?;
    let base = a
        .states
        .get(state)
        .or(a.states.first())
        .and_then(|s| s.image.clone())?;
    images::load(&base, &p.folder)
}

impl Inner {
    /// Show an instance's current image and title for its state.
    fn redraw(&self, st: &State, inst: &Instance) {
        let image = inst.images.get(&inst.state).cloned().or_else(|| {
            st.plugins
                .get(&inst.key.plugin)
                .and_then(|p| default_image(p, &inst.key.action, inst.state))
        });
        let title = inst.titles.get(&inst.state).cloned().unwrap_or(None);
        let _ = self.draws.send(Draw::Look {
            context: inst.key.context.clone(),
            title: Some(title),
            image: Some(image),
        });
    }
}

impl KeyAgents for SdHost {
    fn device(&self, device_id: &str, grid: Option<Grid>) {
        let mut st = self.0.state.lock().expect("state poisoned");
        let msg = match grid {
            Some(g) => {
                st.devices.insert(device_id.to_owned(), g);
                json!({ "event": "deviceDidConnect", "device": device_id, "deviceInfo": {
                    "name": "PerritoDeck tablet",
                    "size": { "columns": g.cols, "rows": g.rows },
                    "type": 3,
                }})
            }
            None => {
                st.devices.remove(device_id);
                json!({ "event": "deviceDidDisconnect", "device": device_id })
            }
        };
        for p in st.plugins.values() {
            if let Some((_, tx)) = &p.out {
                let _ = tx.send(msg.to_string());
            }
        }
    }

    fn appear(&self, key: &AgentKey) {
        let mut st = self.0.state.lock().expect("state poisoned");
        let inst = Instance {
            key: key.clone(),
            state: 0,
            images: HashMap::new(),
            titles: HashMap::new(),
        };
        self.0.redraw(&st, &inst);
        let msg = action_event(&st, "willAppear", &inst);
        st.instances.insert(key.context.clone(), inst);
        drop(st);
        self.0.send(&key.plugin, msg);
    }

    fn disappear(&self, key: &AgentKey) {
        let mut st = self.0.state.lock().expect("state poisoned");
        let Some(inst) = st.instances.remove(&key.context) else {
            return;
        };
        let msg = action_event(&st, "willDisappear", &inst);
        drop(st);
        self.0.send(&key.plugin, msg);
    }

    fn press(&self, key: &AgentKey, down: bool) {
        let st = self.0.state.lock().expect("state poisoned");
        let Some(inst) = st.instances.get(&key.context) else {
            return;
        };
        let running = st.plugins.get(&key.plugin).is_some_and(|p| p.out.is_some());
        if !running {
            if down {
                let _ = self.0.draws.send(Draw::Feedback {
                    context: key.context.clone(),
                    ok: false,
                });
            }
            return;
        }
        let msg = action_event(&st, if down { "keyDown" } else { "keyUp" }, inst);
        drop(st);
        self.0.send(&key.plugin, msg);
    }
}

/// Start a plugin and keep it running.
async fn supervise(inner: Arc<Inner>, uuid: String) {
    let Some(mine) = inner
        .state
        .lock()
        .expect("state poisoned")
        .plugins
        .get(&uuid)
        .map(|p| p.stop.clone())
    else {
        return;
    };
    let exited = inner
        .state
        .lock()
        .expect("state poisoned")
        .plugins
        .get(&uuid)
        .map(|p| p.exited.clone());
    let _done = Exited(exited);
    let mut crashes = 0;
    loop {
        let started = std::time::Instant::now();
        let (code, folder, stop, args) = {
            let st = inner.state.lock().expect("state poisoned");
            let Some(p) = st
                .plugins
                .get(&uuid)
                .filter(|p| Arc::ptr_eq(&p.stop, &mine))
            else {
                return;
            };
            let args = vec![
                "-port".to_owned(),
                inner.port.to_string(),
                "-pluginUUID".to_owned(),
                uuid.clone(),
                "-registerEvent".to_owned(),
                "registerPlugin".to_owned(),
                "-info".to_owned(),
                inner.info(&st, p).to_string(),
            ];
            (
                p.manifest.code(&p.folder),
                p.folder.clone(),
                p.stop.clone(),
                args,
            )
        };
        let set_status = |s: Status| {
            if let Some(p) = inner
                .state
                .lock()
                .expect("state poisoned")
                .plugins
                .get_mut(&uuid)
                .filter(|p| Arc::ptr_eq(&p.stop, &mine))
            {
                p.status = s;
            }
        };
        let program: &OsStr = match &code {
            Code::Node(_) => OsStr::new(inner.node.as_deref().unwrap_or("node")),
            Code::Native(program) => program.as_os_str(),
            Code::Unsupported(reason) => {
                set_status(Status::Unsupported {
                    reason: reason.clone(),
                });
                return;
            }
        };
        let is_node = matches!(code, Code::Node(_));
        // On Windows the plugin and everything it starts end together (a job object it
        // is in before it runs), and no console window pops up for it.
        let spawn = |program: &OsStr| {
            let mut cmd = tokio::process::Command::new(program);
            if let Code::Node(script) = &code {
                cmd.arg(script);
            }
            cmd.args(&args)
                .current_dir(&folder)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            perritodeck_actions::job::ProcessTree::spawn(&mut cmd, CREATE_NO_WINDOW)
        };
        let not_found = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
        let mut spawned = spawn(program);
        if is_node && inner.node.is_none() && spawned.as_ref().is_err_and(not_found) {
            // Node.js installed since PerritoDeck started isn't on the PATH it started
            // with: look where a program started now would find it.
            if let Some(found) = node::find() {
                info!(
                    plugin = %uuid,
                    "Node.js isn't on PerritoDeck's PATH yet; using {}",
                    found.display()
                );
                spawned = spawn(found.as_os_str());
            }
        }
        let (mut child, tree) = match spawned {
            Ok(c) => c,
            Err(e) if is_node && not_found(&e) => {
                set_status(Status::NeedsNode);
                return;
            }
            Err(e) => {
                set_status(Status::Crashed {
                    reason: format!("couldn't start it: {e}"),
                });
                return;
            }
        };
        set_status(Status::Starting);
        for (pipe, is_err) in [
            (
                child
                    .stdout
                    .take()
                    .map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
                false,
            ),
            (
                child
                    .stderr
                    .take()
                    .map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
                true,
            ),
        ] {
            let Some(pipe) = pipe else { continue };
            let uuid = uuid.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = tokio::io::BufReader::new(pipe).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    if is_err {
                        info!(plugin = %uuid, "{l}");
                    } else {
                        debug!(plugin = %uuid, "{l}");
                    }
                }
            });
        }
        let exit = tokio::select! {
            status = child.wait() => status,
            _ = stop.notified() => {
                let _ = child.kill().await;
                end(tree).await;
                return;
            }
        };
        end(tree).await;
        // A plugin that ran for a while before stopping starts its count again.
        if started.elapsed() > Duration::from_secs(60) {
            crashes = 0;
        }
        crashes += 1;
        let reason = match exit {
            Ok(s) => format!("it stopped ({s})"),
            Err(e) => e.to_string(),
        };
        warn!(plugin = %uuid, "Stream Deck plugin {reason}");
        if crashes >= MAX_CRASHES {
            set_status(Status::Crashed { reason });
            return;
        }
        set_status(Status::Starting);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(2u64.pow(crashes))) => {}
            _ = stop.notified() => return,
        }
    }
}

/// End whatever a plugin left running, and wait until it's gone, so that `exited`
/// means its folder can be deleted.
async fn end(tree: Option<perritodeck_actions::job::ProcessTree>) {
    if let Some(tree) = tree {
        tree.end().await;
    }
}

struct Exited(Option<Arc<std::sync::atomic::AtomicBool>>);

impl Drop for Exited {
    fn drop(&mut self) {
        if let Some(e) = &self.0 {
            e.store(true, Ordering::Relaxed);
        }
    }
}

/// One plugin's WebSocket.
async fn connection(inner: Arc<Inner>, stream: tokio::net::TcpStream) {
    // Plugins are local programs and send no Origin. Web pages always do: refuse them, so a
    // site open in a browser can't pose as a plugin on this port. The one exception is
    // our own settings-page server, whose pages connect from its origin.
    let settings_origin = inspector::origin(inner.http_port);
    let from_page = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = from_page.clone();
    #[allow(clippy::result_large_err)]
    let no_browsers =
        move |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
              resp: tokio_tungstenite::tungstenite::handshake::server::Response| {
            if let Some(origin) = req.headers().get("origin") {
                if origin.as_bytes() == settings_origin.as_bytes() {
                    flag.store(true, Ordering::Relaxed);
                    return Ok(resp);
                }
                let mut refuse =
                    tokio_tungstenite::tungstenite::handshake::server::ErrorResponse::new(Some(
                        "browsers can't connect here".into(),
                    ));
                *refuse.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;
                return Err(refuse);
            }
            Ok(resp)
        };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, no_browsers).await else {
        return;
    };
    let (mut sink, mut source) = ws.split();
    let first = match tokio::time::timeout(Duration::from_secs(10), source.next()).await {
        Ok(Some(Ok(WsMessage::Text(t)))) => t.to_string(),
        _ => return,
    };
    let reg: Value = serde_json::from_str(&first).unwrap_or_default();
    let uuid = reg["uuid"].as_str().unwrap_or_default().to_owned();
    if from_page.load(Ordering::Relaxed) {
        if reg["event"] == "registerPropertyInspector" {
            inspector::connection(inner, uuid, sink, source).await;
        }
        return;
    }
    if reg["event"] != "registerPlugin" {
        debug!("Stream Deck: ignoring a {} connection", reg["event"]);
        return;
    }
    let id = inner.connections.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    {
        let mut st = inner.state.lock().expect("state poisoned");
        let Some(p) = st.plugins.get_mut(&uuid) else {
            warn!("Stream Deck: unknown plugin {uuid} tried to register");
            return;
        };
        if p.out.is_some() {
            warn!("Stream Deck: {uuid} is already connected");
            return;
        }
        p.out = Some((id, tx.clone()));
        p.status = Status::Running;
        info!(plugin = %uuid, "Stream Deck plugin connected");
        // Catch it up: the tablets, then every key of its that's showing.
        for (dev, g) in &st.devices {
            let mut m = json!({ "event": "deviceDidConnect", "device": dev });
            m["deviceInfo"] = device_info(dev, *g);
            let _ = tx.send(m.to_string());
        }
        for inst in st.instances.values().filter(|i| i.key.plugin == uuid) {
            let _ = tx.send(action_event(&st, "willAppear", inst).to_string());
        }
    }
    let writer = tokio::spawn(async move {
        while let Some(text) = rx.recv().await {
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    while let Some(Ok(msg)) = source.next().await {
        if let WsMessage::Text(t) = msg {
            match serde_json::from_str::<Value>(&t) {
                Ok(v) => handle(&inner, &uuid, &v),
                Err(e) => debug!(plugin = %uuid, "unreadable message: {e}"),
            }
        }
    }
    writer.abort();
    let mut st = inner.state.lock().expect("state poisoned");
    if let Some(p) = st.plugins.get_mut(&uuid) {
        if p.out.as_ref().is_some_and(|(c, _)| *c == id) {
            p.out = None;
        }
    }
}

/// A message from a plugin.
fn handle(inner: &Inner, uuid: &str, v: &Value) {
    let event = v["event"].as_str().unwrap_or_default();
    let context = v["context"].as_str().unwrap_or_default().to_owned();
    let payload = &v["payload"];
    let mut st = inner.state.lock().expect("state poisoned");
    // Key commands only reach this plugin's own keys.
    let owns = st
        .instances
        .get(&context)
        .is_some_and(|i| i.key.plugin == uuid);
    match event {
        "setTitle" | "setImage" | "setState" if owns => {
            let folder = st
                .plugins
                .get(uuid)
                .map(|p| p.folder.clone())
                .unwrap_or_default();
            let inst = st.instances.get_mut(&context).expect("checked above");
            let target_state = payload["state"].as_u64().map(|s| s as usize);
            match event {
                "setTitle" => {
                    let title = payload["title"].as_str().map(str::to_owned);
                    let states: Vec<usize> =
                        target_state.map_or_else(|| vec![0, 1, inst.state], |s| vec![s]);
                    for s in states {
                        inst.titles.insert(s, title.clone());
                    }
                }
                "setImage" => {
                    let states: Vec<usize> =
                        target_state.map_or_else(|| vec![0, 1, inst.state], |s| vec![s]);
                    let image = payload["image"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .and_then(|s| images::load(s, &folder));
                    if payload["image"].as_str().is_some_and(|s| !s.is_empty()) && image.is_none() {
                        debug!(plugin = %uuid, "an image it set couldn't be read");
                    }
                    for s in states {
                        match &image {
                            Some(png) => {
                                inst.images.insert(s, png.clone());
                            }
                            None => {
                                inst.images.remove(&s);
                            }
                        }
                    }
                }
                _ => {
                    if let Some(s) = payload["state"].as_u64() {
                        inst.state = s as usize;
                    }
                }
            }
            let inst = &st.instances[&context];
            inner.redraw(&st, inst);
        }
        "showOk" | "showAlert" if owns => {
            let _ = inner.draws.send(Draw::Feedback {
                context,
                ok: event == "showOk",
            });
        }
        "setSettings" if owns => {
            let key = store_key(&st.instances[&context].key);
            st.store.contexts.insert(key, payload.clone());
            inner.save_store(&st);
        }
        "getSettings" if owns => {
            let inst = &st.instances[&context];
            let mut m = action_event(&st, "didReceiveSettings", inst);
            if let Some(id) = v.get("id") {
                m["id"] = id.clone();
            }
            drop(st);
            inner.send(uuid, m);
        }
        "setGlobalSettings" => {
            st.store.global.insert(uuid.to_owned(), payload.clone());
            inner.save_store(&st);
        }
        "getGlobalSettings" => {
            let settings = st
                .store
                .global
                .get(uuid)
                .cloned()
                .unwrap_or_else(|| json!({}));
            let mut m =
                json!({ "event": "didReceiveGlobalSettings", "payload": { "settings": settings } });
            if let Some(id) = v.get("id") {
                m["id"] = id.clone();
            }
            drop(st);
            inner.send(uuid, m);
        }
        "openUrl" => {
            let url = payload["url"].as_str().unwrap_or_default();
            if url.starts_with("https://") || url.starts_with("http://") {
                if let Err(e) = open::that_detached(url) {
                    warn!(plugin = %uuid, "couldn't open {url}: {e}");
                }
            }
        }
        "sendToPropertyInspector" => inspector::to_page(&st, uuid, &context, payload),
        "logMessage" => {
            info!(plugin = %uuid, "{}", payload["message"].as_str().unwrap_or_default())
        }
        other => debug!(plugin = %uuid, "Stream Deck: {other} isn't supported"),
    }
}

fn tempdir_in(dir: &Path) -> Result<PathBuf, String> {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let p = dir.join(format!(".install-{n}"));
    std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

/// Unpack a `.streamDeckPlugin` (a zip) into `dest`, refusing paths that leave it.
fn unzip(bytes: &[u8], dest: &Path) -> Result<(), String> {
    use std::io::Read;
    const MAX_TOTAL: u64 = 200 * 1024 * 1024;
    let bad = |e: zip::result::ZipError| format!("That file isn't a Stream Deck plugin: {e}");
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(bad)?;
    if z.len() > 10_000 {
        return Err("The plugin has too many files.".into());
    }
    let mut total = 0u64;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(bad)?;
        let Some(rel) = f.enclosed_name() else {
            return Err(format!("The plugin contains an unsafe path: {}", f.name()));
        };
        let out = dest.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if f.is_symlink() {
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut w = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        total += std::io::copy(
            &mut (&mut f).take(MAX_TOTAL - total.min(MAX_TOTAL) + 1),
            &mut w,
        )
        .map_err(|e| e.to_string())?;
        if total > MAX_TOTAL {
            return Err("The plugin is too big once unpacked.".into());
        }
        #[cfg(unix)]
        if let Some(mode) = f.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o755));
        }
    }
    Ok(())
}

/// The `*.sdPlugin` folder inside an unpacked `.streamDeckPlugin`.
fn find_plugin_folder(root: &Path) -> Result<PathBuf, String> {
    if root.join("manifest.json").is_file() {
        return Ok(root.to_owned());
    }
    std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir() && p.join("manifest.json").is_file())
        .ok_or_else(|| "That file has no plugin in it (no manifest.json).".into())
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let ty = e.file_type()?;
        let to = dest.join(e.file_name());
        if ty.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else if ty.is_file() {
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

/// The OS version as Stream Deck reports it in `-info`: `10.0.<build>` on Windows (10
/// and 11 both report 10.0, like `RtlGetVersion`), `15.1.1` on macOS. Plugins may parse
/// it to pick OS-specific behaviour.
fn platform_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        let v = if cfg!(windows) {
            sysinfo::System::kernel_version().map(|build| format!("10.0.{build}"))
        } else {
            sysinfo::System::os_version()
        };
        v.unwrap_or_else(|| std::env::consts::OS.to_owned())
    })
}
