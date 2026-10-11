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
//! Supported: Node.js plugins (the current SDK), and native plugins on Windows and macOS.
//! Not supported: property inspectors (settings are edited as JSON in the app), dials and
//! touch strips, multi-actions, and the old HTML plugins.

pub mod images;
pub mod manifest;

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    time::Duration,
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
    broken: Vec<BrokenPlugin>,
    devices: BTreeMap<String, Grid>,
    instances: HashMap<String, Instance>,
    store: Store,
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
    dir: PathBuf,
    port: u16,
    state: Mutex<State>,
    deck: OnceLock<Weak<Deck>>,
    draws: mpsc::UnboundedSender<Draw>,
    node: String,
    connections: AtomicU64,
}

/// The Stream Deck plugin host. Cheap to clone.
#[derive(Clone)]
pub struct SdHost(Arc<Inner>);

impl SdHost {
    /// Start the WebSocket server on 127.0.0.1 and every plugin in `dir`.
    pub async fn start(dir: PathBuf) -> std::io::Result<Self> {
        Self::start_with_node(dir, "node".into()).await
    }

    /// Like [`SdHost::start`], with a different Node.js program (tests).
    pub async fn start_with_node(dir: PathBuf, node: String) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let store = std::fs::read(dir.join(STORE_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let (draws, mut draw_rx) = mpsc::unbounded_channel::<Draw>();
        let inner = Arc::new(Inner {
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
        });
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
                    },
                );
                start.push(uuid);
            }
        }
        for uuid in start {
            tokio::spawn(supervise(self.0.clone(), uuid));
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
        if dest.exists() {
            std::fs::remove_dir_all(&dest)
                .map_err(|e| format!("Couldn't replace the old version: {e}"))?;
        }
        copy_dir(&folder, &dest).map_err(|e| format!("Couldn't copy the plugin: {e}"))?;
        self.scan();
        Ok(manifest.uuid)
    }

    pub fn remove(&self, uuid: &str) -> Result<(), String> {
        let folder = {
            let mut st = self.0.state.lock().expect("state poisoned");
            let p = st
                .plugins
                .remove(uuid)
                .ok_or_else(|| format!("There's no Stream Deck plugin {uuid}."))?;
            stop(&p);
            p.folder
        };
        std::fs::remove_dir_all(&folder)
            .map_err(|e| format!("Couldn't remove {}: {e}", folder.display()))?;
        Ok(())
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

    /// Stop every plugin.
    pub fn shutdown(&self) {
        let st = self.0.state.lock().expect("state poisoned");
        for p in st.plugins.values() {
            stop(p);
        }
    }

    /// The WebSocket port plugins connect to.
    pub fn port(&self) -> u16 {
        self.0.port
    }
}

fn stop(p: &Plugin) {
    p.stop.notify_one();
}

impl Inner {
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
                if std::fs::write(&tmp, b)
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
                "platformVersion": std::env::consts::OS,
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
        .get(&key.context)
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
    let mut crashes = 0;
    loop {
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
        let mut cmd = match &code {
            Code::Node(script) => {
                let mut c = tokio::process::Command::new(&inner.node);
                c.arg(script);
                c
            }
            Code::Native(program) => tokio::process::Command::new(program),
            Code::Unsupported(reason) => {
                set_status(Status::Unsupported {
                    reason: reason.clone(),
                });
                return;
            }
        };
        cmd.args(&args)
            .current_dir(&folder)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && matches!(code, Code::Node(_)) => {
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
                return;
            }
        };
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

/// One plugin's WebSocket.
async fn connection(inner: Arc<Inner>, stream: tokio::net::TcpStream) {
    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let (mut sink, mut source) = ws.split();
    let first = match tokio::time::timeout(Duration::from_secs(10), source.next()).await {
        Ok(Some(Ok(WsMessage::Text(t)))) => t.to_string(),
        _ => return,
    };
    let reg: Value = serde_json::from_str(&first).unwrap_or_default();
    let uuid = reg["uuid"].as_str().unwrap_or_default().to_owned();
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
            st.store.contexts.insert(context, payload.clone());
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
