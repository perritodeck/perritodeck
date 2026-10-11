//! The running host, as a library: shared by the `perritodeck-host` command line and
//! the PerritoDeck desktop app.
//!
//! [`Host::start`] loads the identity, paired devices, profile and integrations, starts
//! the TLS server and mDNS advertisement, and wires server events to the deck. The
//! returned [`Host`] answers the questions a UI asks (who's paired, open a pairing
//! window, what's on each page) and broadcasts [`HostEvent`]s.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use perritodeck_actions::{
    system::{EnigoInput, OpenSystem},
    Executor,
};
use perritodeck_core::{
    discovery::{self, Advertisement},
    display_fingerprint,
    pairing::{PairingStore, DEFAULT_OFFER_TTL},
    HostIdentity, Security, Server, ServerConfig, ServerEvent, ServerHandle,
};
use perritodeck_deck::{watch::watch_profile, Deck, Outbox};
pub use perritodeck_actions::ActionSpec;
pub use perritodeck_profile::edit::{Edit, KeyFields};
use perritodeck_profile::{NavSpec, Profile, SimpleNav};
use perritodeck_protocol::{Message, DEFAULT_PORT};
use serde::Serialize;
use tokio::{net::TcpListener, sync::broadcast, task::JoinHandle};
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct HostOptions {
    pub listen: IpAddr,
    pub port: u16,
    pub heartbeat: Duration,
    pub data_dir: PathBuf,
    pub profile: Option<PathBuf>,
    pub insecure: bool,
    pub mdns: bool,
    /// Fixed pairing codes (tests only).
    pub pair_codes: Vec<String>,
}

impl HostOptions {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            listen: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: DEFAULT_PORT,
            heartbeat: Duration::from_secs(2),
            data_dir,
            profile: None,
            insecure: false,
            mdns: true,
            pair_codes: Vec::new(),
        }
    }
}

/// The user's settings folder, e.g. `~/.config/PerritoDeck` on Linux.
pub fn default_data_dir() -> Result<PathBuf> {
    directories::ProjectDirs::from("dev", "perritodeck", "PerritoDeck")
        .map(|d| d.config_dir().to_owned())
        .context("could not find a settings folder for this user")
}

pub fn devices_path(dir: &Path) -> PathBuf {
    dir.join("devices.json")
}

pub fn integrations_path(dir: &Path) -> PathBuf {
    dir.join("integrations.toml")
}

pub fn default_profile_path(dir: &Path) -> PathBuf {
    dir.join("profiles").join("default.toml")
}

/// What a UI hears about.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HostEvent {
    #[serde(rename_all = "camelCase")]
    Connected {
        device_id: String,
        device_name: String,
    },
    #[serde(rename_all = "camelCase")]
    Disconnected { device_id: String },
    #[serde(rename_all = "camelCase")]
    Paired {
        device_id: String,
        device_name: String,
    },
    #[serde(rename_all = "camelCase")]
    KeyPressed { device_id: String, key_index: u32 },
    /// The profile file was reloaded, or an edit to it was rejected.
    ProfileChanged,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    /// Unix seconds.
    pub last_seen: Option<u64>,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingOffer {
    pub code: String,
    pub uri: String,
    /// Short fingerprint to compare by eye, e.g. `3FA9 07C2 D41E`.
    pub fingerprint: String,
    pub expires_in_secs: u64,
    /// The pairing link drawn as an SVG QR code.
    pub qr_svg: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub host_name: String,
    pub version: String,
    pub fingerprint: Option<String>,
    pub addresses: Vec<String>,
    pub insecure: bool,
    pub data_dir: String,
    pub profile_path: String,
    pub integrations: Vec<String>,
}

/// A read-only view of the profile for UIs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub name: String,
    pub cols: u32,
    pub rows: u32,
    /// True when the profile fixes the grid size; otherwise it follows each tablet.
    pub fixed_grid: bool,
    pub pages: Vec<PageView>,
    pub reload_error: Option<String>,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageView {
    pub name: String,
    pub keys: Vec<KeyView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyView {
    pub row: u32,
    pub col: u32,
    pub title: String,
    pub bg: String,
    pub fg: String,
    /// "action", "page", "back", "next", "previous", "toggle" or "label".
    pub kind: String,
    /// Short description of what it does, e.g. "hotkey Ctrl+C".
    pub detail: String,
    /// The key's own settings, as the editor changes them.
    pub fields: KeyFields,
    /// Toggle states (0 for an ordinary key). Toggles are edited in the file for now.
    pub states: usize,
    pub has_icon: bool,
}

/// Outbox that forwards to the server handle once it exists.
#[derive(Default)]
struct LateOutbox(std::sync::OnceLock<ServerHandle>);

impl Outbox for LateOutbox {
    fn send(&self, device_id: &str, msg: Message) -> bool {
        self.0.get().is_some_and(|h| h.send(device_id, msg))
    }
}

/// Edits made through the app, as (file before, file after) pairs.
#[derive(Default)]
struct History {
    undo: Vec<(String, String)>,
    redo: Vec<(String, String)>,
}

const HISTORY_LIMIT: usize = 200;

pub struct Host {
    pub host_name: String,
    pub addr: SocketAddr,
    pub lan: Vec<IpAddr>,
    pub profile_path: PathBuf,
    pub data_dir: PathBuf,
    pub identity: Option<HostIdentity>,
    pub store: Option<Arc<PairingStore>>,
    pub handle: ServerHandle,
    pub deck: Arc<Deck>,
    insecure: bool,
    integrations: Vec<String>,
    events: broadcast::Sender<HostEvent>,
    history: std::sync::Mutex<History>,
    executor: Executor,
    server: JoinHandle<std::io::Result<()>>,
    _watcher: Option<notify::RecommendedWatcher>,
    _mdns: Option<Advertisement>,
}

impl Host {
    pub async fn start(opts: HostOptions) -> Result<Self> {
        if opts.insecure && !opts.listen.is_loopback() {
            bail!("insecure mode only works on loopback (listen on 127.0.0.1)");
        }
        let dir = opts.data_dir.clone();
        let host_name = gethostname::gethostname().to_string_lossy().into_owned();
        let (security, store, identity) = if opts.insecure {
            warn!("insecure mode: no encryption, no pairing — development only");
            (Security::Insecure, None, None)
        } else {
            let identity = HostIdentity::load_or_create(&dir)
                .with_context(|| format!("loading the host certificate from {}", dir.display()))?;
            let store = Arc::new(PairingStore::open(&devices_path(&dir))?);
            for code in &opts.pair_codes {
                if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
                    bail!("pairing codes must be 6 digits");
                }
                warn!("fixed pairing code enabled for one hour (testing only)");
                store.add_offer(code.clone(), Duration::from_secs(3600));
            }
            let security = Security::Paired {
                tls: identity.tls_acceptor(),
                store: store.clone(),
                host_id: identity.fingerprint().to_owned(),
            };
            (security, Some(store), Some(identity))
        };

        let addr = SocketAddr::new(opts.listen, opts.port);
        let listener = TcpListener::bind(addr).await.with_context(|| {
            format!("could not listen on {addr} — is another PerritoDeck host already running?")
        })?;
        let addr = listener.local_addr().unwrap_or(addr);

        let profile_path = opts
            .profile
            .clone()
            .unwrap_or_else(|| default_profile_path(&dir));
        if perritodeck_profile::ensure_starter(&profile_path)? {
            info!("created a starter profile: {}", profile_path.display());
        }
        let profile = perritodeck_profile::load(&profile_path)?;
        let base_dir = profile_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        for m in profile.missing_icons(&base_dir) {
            warn!("icon not found: {}", m.display());
        }

        let input = Arc::new(EnigoInput::new());
        if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some() {
            // Ask for keyboard access now, so the desktop's permission prompt appears at
            // startup rather than on the first key press.
            let input = input.clone();
            tokio::task::spawn_blocking(move || {
                match input.connect_now() {
                Ok(()) => info!("keyboard access granted"),
                Err(e) => warn!("hotkeys and typing won't work until access is allowed and the host restarts: {e}"),
            }
            });
        }
        let integrations_config = perritodeck_integrations::Config::load(&integrations_path(&dir))
            .unwrap_or_else(|e| {
                warn!("ignoring integrations.toml: {e}");
                Default::default()
            });
        let builtins = perritodeck_integrations::Builtins::new(integrations_config);
        let integrations: Vec<String> = builtins
            .configured()
            .into_iter()
            .map(str::to_owned)
            .collect();
        if !integrations.is_empty() {
            info!("integrations: {}", integrations.join(", "));
        }
        let executor = Executor::new(input, Arc::new(OpenSystem)).with_integrations(builtins);
        let test_executor = executor.clone();

        let outbox = Arc::new(LateOutbox::default());
        let deck = Deck::new(profile, base_dir, outbox.clone(), executor);
        let layout_deck = deck.clone();
        let mut config = ServerConfig::new(
            host_name.clone(),
            security,
            Arc::new(move |hello: &perritodeck_protocol::Hello| layout_deck.initial_layout(hello)),
        );
        config.heartbeat_interval = opts.heartbeat;
        let (server, server_events) = Server::new(listener, config);
        let handle = server.handle();
        let _ = outbox.0.set(handle.clone());

        let watcher = watch_profile(profile_path.clone(), deck.clone())
            .map_err(|e| {
                warn!(
                    "won't notice edits to {} until restart: {e}",
                    profile_path.display()
                )
            })
            .ok();
        let lan = if opts.listen.is_loopback() {
            vec![]
        } else {
            discovery::lan_addresses()
        };
        let mdns = match (&identity, opts.mdns && !opts.listen.is_loopback()) {
            (Some(id), true) => discovery::advertise(&host_name, id.fingerprint(), addr.port())
                .map_err(|e| warn!("could not advertise on the network (tablets can still connect by address): {e}"))
                .ok(),
            _ => None,
        };

        let (events, _) = broadcast::channel(64);
        spawn_event_loop(deck.clone(), server_events, events.clone());
        let server = tokio::spawn(server.run());

        Ok(Self {
            host_name,
            addr,
            lan,
            profile_path,
            data_dir: dir,
            identity,
            store,
            handle,
            deck,
            insecure: opts.insecure,
            integrations,
            events,
            history: Default::default(),
            executor: test_executor,
            server,
            _watcher: watcher,
            _mdns: mdns,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HostEvent> {
        self.events.subscribe()
    }

    /// Resolves when the server stops (it normally runs until the process exits).
    pub async fn wait(&mut self) -> Result<()> {
        (&mut self.server)
            .await
            .context("server task failed")?
            .context("server stopped")
    }

    pub fn status(&self) -> HostStatus {
        let port = self.addr.port();
        let addresses = if self.addr.ip().is_loopback() {
            vec![format!("127.0.0.1:{port}")]
        } else {
            self.lan.iter().map(|ip| format!("{ip}:{port}")).collect()
        };
        HostStatus {
            host_name: self.host_name.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            fingerprint: self
                .identity
                .as_ref()
                .map(|i| display_fingerprint(i.fingerprint())),
            addresses,
            insecure: self.insecure,
            data_dir: self.data_dir.display().to_string(),
            profile_path: self.profile_path.display().to_string(),
            integrations: self.integrations.clone(),
        }
    }

    /// Addresses to put in pairing links.
    fn pairing_addrs(&self) -> Vec<IpAddr> {
        if self.addr.ip().is_loopback() {
            vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]
        } else {
            self.lan.clone()
        }
    }

    /// Open a pairing window. `None` in insecure mode (no pairing there).
    pub fn new_pairing_offer(&self) -> Option<PairingOffer> {
        let (store, id) = (self.store.as_ref()?, self.identity.as_ref()?);
        let code = store.new_offer(DEFAULT_OFFER_TTL);
        let uri = discovery::pairing_uri(
            &self.host_name,
            &self.pairing_addrs(),
            self.addr.port(),
            id.fingerprint(),
            &code,
        );
        let qr_svg = qrcode::QrCode::new(uri.as_bytes())
            .map(|qr| {
                qr.render::<qrcode::render::svg::Color>()
                    .min_dimensions(240, 240)
                    .quiet_zone(true)
                    .build()
            })
            .unwrap_or_default();
        Some(PairingOffer {
            code,
            uri,
            fingerprint: display_fingerprint(id.fingerprint()),
            expires_in_secs: DEFAULT_OFFER_TTL.as_secs(),
            qr_svg,
        })
    }

    pub fn devices(&self) -> Vec<DeviceInfo> {
        let online = self.handle.devices();
        let Some(store) = &self.store else {
            return online
                .into_iter()
                .map(|id| DeviceInfo {
                    name: id.clone(),
                    id,
                    last_seen: None,
                    connected: true,
                })
                .collect();
        };
        store
            .devices()
            .into_iter()
            .map(|(id, d)| DeviceInfo {
                connected: online.contains(&id),
                id,
                name: d.name,
                last_seen: d.last_seen,
            })
            .collect()
    }

    /// Forget a device and disconnect it now.
    pub fn revoke(&self, device_id: &str) -> bool {
        let removed = self.store.as_ref().is_some_and(|s| s.revoke(device_id));
        if removed {
            self.handle.kick(device_id);
        }
        removed
    }

    pub fn profile_view(&self) -> ProfileView {
        let profile = self.deck.profile();
        let mut view = view_of(&profile, self.deck.reload_error());
        let history = self.history.lock().expect("history poisoned");
        view.can_undo = !history.undo.is_empty();
        view.can_redo = !history.redo.is_empty();
        view
    }

    /// Change the profile file and show the result on the tablets right away.
    pub fn edit_profile(&self, edits: &[Edit]) -> Result<ProfileView, String> {
        let before = self.read_profile_text()?;
        let after = perritodeck_profile::edit::apply(&before, edits).map_err(|e| e.to_string())?;
        if after != before {
            self.write_profile_text(&after)?;
            let mut history = self.history.lock().expect("history poisoned");
            history.undo.push((before, after));
            if history.undo.len() > HISTORY_LIMIT {
                history.undo.remove(0);
            }
            history.redo.clear();
        }
        Ok(self.profile_view())
    }

    /// Run an action now, as if its key were pressed. `Ok` carries a note to show when
    /// the action ran but reported a problem (a command that failed, say).
    pub async fn test_action(
        &self,
        spec: &perritodeck_actions::ActionSpec,
    ) -> Result<Option<String>, String> {
        match self.executor.run(spec).await {
            Ok(perritodeck_actions::Outcome::Ok) => Ok(None),
            Ok(perritodeck_actions::Outcome::Alert(note)) => Ok(Some(note)),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Step back through the app's edits. Edits made in a text editor aren't undone here.
    pub fn undo(&self) -> Result<ProfileView, String> {
        self.step(true)
    }

    pub fn redo(&self) -> Result<ProfileView, String> {
        self.step(false)
    }

    fn step(&self, back: bool) -> Result<ProfileView, String> {
        let mut history = self.history.lock().expect("history poisoned");
        let entry = if back {
            history.undo.pop()
        } else {
            history.redo.pop()
        };
        let Some((before, after)) = entry else {
            return Err(format!(
                "Nothing to {}.",
                if back { "undo" } else { "redo" }
            ));
        };
        let (expected, target) = if back {
            (&after, &before)
        } else {
            (&before, &after)
        };
        if self.read_profile_text()? != *expected {
            history.undo.clear();
            history.redo.clear();
            return Err("The profile file was changed outside PerritoDeck, so earlier edits can't be undone.".into());
        }
        self.write_profile_text(target)?;
        if back {
            history.redo.push((before, after));
        } else {
            history.undo.push((before, after));
        }
        drop(history);
        Ok(self.profile_view())
    }

    fn read_profile_text(&self) -> Result<String, String> {
        std::fs::read_to_string(&self.profile_path)
            .map_err(|e| format!("Couldn't read {}: {e}", self.profile_path.display()))
    }

    /// Save atomically (so a crash never leaves half a file), then reload the deck now
    /// rather than waiting for the file watcher.
    fn write_profile_text(&self, text: &str) -> Result<(), String> {
        let path = &self.profile_path;
        let profile = Profile::parse(text).map_err(|e| e.to_string())?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = path.with_file_name(format!(".{name}.saving"));
        std::fs::write(&tmp, text)
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("Couldn't save {}: {e}", path.display())
            })?;
        self.deck.reload(profile);
        Ok(())
    }
}

pub fn view_of(profile: &Profile, reload_error: Option<String>) -> ProfileView {
    let (cols, rows) = profile.grid.map(|g| (g.cols, g.rows)).unwrap_or_else(|| {
        // Sized per tablet; preview the most common tablet grid.
        let max_c = profile
            .pages
            .iter()
            .flat_map(|p| p.keys.iter().map(|k| k.at[1] + 1))
            .max()
            .unwrap_or(1);
        let max_r = profile
            .pages
            .iter()
            .flat_map(|p| p.keys.iter().map(|k| k.at[0] + 1))
            .max()
            .unwrap_or(1);
        (max_c.max(5), max_r.max(3))
    });
    let pages = profile
        .pages
        .iter()
        .map(|p| PageView {
            name: p.name.clone(),
            keys: p
                .keys
                .iter()
                .map(|k| {
                    let look = k.look(0);
                    let (kind, detail) = match (&k.nav, k.states.len() >= 2, k.action(0)) {
                        (Some(NavSpec::Page { page }), _, _) => ("page", format!("opens {page}")),
                        (Some(NavSpec::Simple(SimpleNav::Back)), _, _) => {
                            ("back", "back".to_owned())
                        }
                        (Some(NavSpec::Simple(SimpleNav::Next)), _, _) => {
                            ("next", "next page".to_owned())
                        }
                        (Some(NavSpec::Simple(SimpleNav::Previous)), _, _) => {
                            ("previous", "previous page".to_owned())
                        }
                        (None, true, a) => (
                            "toggle",
                            format!(
                                "{} states{}",
                                k.states.len(),
                                a.map(|a| format!(", {}", describe(a))).unwrap_or_default()
                            ),
                        ),
                        (None, false, Some(a)) => ("action", describe(a)),
                        (None, false, None) => ("label", "no action".to_owned()),
                    };
                    KeyView {
                        row: k.at[0],
                        col: k.at[1],
                        title: look.title,
                        bg: look.bg,
                        fg: look.fg,
                        kind: kind.to_owned(),
                        detail,
                        fields: KeyFields {
                            title: k.title.clone(),
                            bg: k.bg.clone(),
                            fg: k.fg.clone(),
                            action: k.action.clone(),
                            nav: k.nav.clone(),
                        },
                        states: k.states.len(),
                        has_icon: k.icon.is_some() || k.states.iter().any(|s| s.icon.is_some()),
                    }
                })
                .collect(),
        })
        .collect();
    ProfileView {
        name: profile.name.clone(),
        cols,
        rows,
        fixed_grid: profile.grid.is_some(),
        pages,
        reload_error,
        can_undo: false,
        can_redo: false,
    }
}

fn describe(a: &perritodeck_actions::ActionSpec) -> String {
    use perritodeck_actions::ActionSpec as A;
    match a {
        A::Hotkey { keys } => format!("hotkey {keys}"),
        A::Text { text } => format!("types {:?}", text.chars().take(24).collect::<String>()),
        A::Media { key } => format!("media {key:?}"),
        A::OpenUrl { url } => format!("opens {url}"),
        A::OpenPath { path } => format!("opens {}", path.display()),
        A::Launch { program, .. } => format!("starts {program}"),
        A::Command { program, .. } => format!("runs {program}"),
        A::Multi { steps } => format!("{} steps", steps.len()),
        A::Obs(o) => format!("OBS {o:?}"),
        A::HomeAssistant {
            service, entity, ..
        } => {
            format!(
                "Home Assistant {service}{}",
                entity
                    .as_deref()
                    .map(|e| format!(" {e}"))
                    .unwrap_or_default()
            )
        }
        A::None => "nothing".to_owned(),
    }
}

/// Feed server events to the deck, keep live titles ticking, and tell subscribers.
fn spawn_event_loop(
    deck: Arc<Deck>,
    mut server_events: tokio::sync::mpsc::UnboundedReceiver<ServerEvent>,
    out: broadcast::Sender<HostEvent>,
) {
    let ticker = deck.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            ticker.tick();
        }
    });
    let (watched, notify) = (deck.clone(), out.clone());
    tokio::spawn(async move {
        let mut seen = watched.revision();
        let mut every = tokio::time::interval(Duration::from_millis(200));
        loop {
            every.tick().await;
            let now = watched.revision();
            if now != seen {
                seen = now;
                let _ = notify.send(HostEvent::ProfileChanged);
            }
        }
    });
    tokio::spawn(async move {
        while let Some(event) = server_events.recv().await {
            let ui = match event {
                ServerEvent::Connected {
                    device_id,
                    device_name,
                    grid,
                    screen,
                    capabilities,
                    ..
                } => {
                    deck.connected(&device_id, grid, screen, &capabilities);
                    Some(HostEvent::Connected {
                        device_id,
                        device_name,
                    })
                }
                ServerEvent::KeyDown {
                    device_id,
                    key_index,
                } => {
                    info!(device = %device_id, "key {} pressed", key_index + 1);
                    deck.key_down(&device_id, key_index);
                    Some(HostEvent::KeyPressed {
                        device_id,
                        key_index,
                    })
                }
                ServerEvent::Paired {
                    device_id,
                    device_name,
                } => Some(HostEvent::Paired {
                    device_id,
                    device_name,
                }),
                ServerEvent::Disconnected { device_id, .. } => {
                    deck.disconnected(&device_id);
                    Some(HostEvent::Disconnected { device_id })
                }
                _ => None,
            };
            if let Some(e) = ui {
                let _ = out.send(e);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn starts_and_answers_ui_questions() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = HostOptions::new(dir.path().to_owned());
        opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
        opts.port = 0;
        opts.mdns = false;
        let host = Host::start(opts).await.unwrap();

        let status = host.status();
        assert!(status.fingerprint.is_some());
        assert_eq!(
            status.addresses,
            vec![format!("127.0.0.1:{}", host.addr.port())]
        );
        assert!(
            Path::new(&status.profile_path).exists(),
            "starter profile written"
        );

        let offer = host.new_pairing_offer().unwrap();
        assert_eq!(offer.code.len(), 6);
        assert!(offer.uri.starts_with("perritodeck://pair?"));
        assert!(offer.qr_svg.contains("<svg"));

        assert!(host.devices().is_empty());
        assert!(!host.revoke("nobody"));

        let view = host.profile_view();
        assert!(!view.pages.is_empty());
        assert!(view.pages[0].keys.iter().any(|k| k.kind == "page"));
        assert!(view.reload_error.is_none());

        // A broken edit is reported to subscribers and in the view.
        let mut events = host.subscribe();
        std::fs::write(&status.profile_path, "this is not toml [").unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(HostEvent::ProfileChanged) = events.recv().await {
                    break;
                }
            }
        })
        .await;
        assert!(got.is_ok(), "no ProfileChanged event");
        assert!(host.profile_view().reload_error.is_some());
    }

    #[tokio::test]
    async fn edits_save_reload_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = HostOptions::new(dir.path().to_owned());
        opts.listen = IpAddr::V4(Ipv4Addr::LOCALHOST);
        opts.port = 0;
        opts.mdns = false;
        let host = Host::start(opts).await.unwrap();
        let original = std::fs::read_to_string(&host.profile_path).unwrap();
        let title = |v: &ProfileView| {
            v.pages[0]
                .keys
                .iter()
                .find(|k| k.row == 1 && k.col == 1)
                .map(|k| k.title.clone())
        };

        let edit = Edit::SetKey {
            page: "Main".into(),
            at: [1, 1],
            key: KeyFields {
                title: Some("Copied".into()),
                ..Default::default()
            },
        };
        let view = host.edit_profile(std::slice::from_ref(&edit)).unwrap();
        assert_eq!(title(&view).as_deref(), Some("Copied"));
        assert!(view.can_undo && !view.can_redo);
        // Saved to disk and already live in the deck (no waiting for the watcher).
        assert!(std::fs::read_to_string(&host.profile_path)
            .unwrap()
            .contains("\"Copied\""));
        assert_eq!(
            title(&view_of(&host.deck.profile(), None)).as_deref(),
            Some("Copied")
        );

        let view = host.undo().unwrap();
        assert_eq!(title(&view).as_deref(), Some("Copy"));
        assert_eq!(
            std::fs::read_to_string(&host.profile_path).unwrap(),
            original
        );
        assert!(!view.can_undo && view.can_redo);
        let view = host.redo().unwrap();
        assert_eq!(title(&view).as_deref(), Some("Copied"));

        // A bad edit changes nothing.
        let bad = Edit::SetKey {
            page: "Main".into(),
            at: [0, 0],
            key: KeyFields {
                bg: Some("red".into()),
                ..Default::default()
            },
        };
        assert!(host.edit_profile(&[bad]).is_err());
        assert_eq!(title(&host.profile_view()).as_deref(), Some("Copied"));

        // Actions can be tried from the editor.
        let ok = perritodeck_actions::ActionSpec::Command {
            program: "true".into(),
            args: vec![],
            cwd: None,
            timeout_ms: None,
        };
        if cfg!(unix) {
            assert_eq!(host.test_action(&ok).await, Ok(None));
            let fails = perritodeck_actions::ActionSpec::Command {
                program: "false".into(),
                args: vec![],
                cwd: None,
                timeout_ms: None,
            };
            assert!(host.test_action(&fails).await.unwrap().is_some());
        }
        let bad = perritodeck_actions::ActionSpec::OpenUrl {
            url: "file:///etc/passwd".into(),
        };
        assert!(host.test_action(&bad).await.is_err());

        // After a hand edit, undo refuses rather than overwrite it.
        std::fs::write(&host.profile_path, &original).unwrap();
        assert!(host.undo().unwrap_err().contains("outside"));
        assert!(!host.profile_view().can_undo);
    }

    #[tokio::test]
    async fn insecure_needs_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = HostOptions::new(dir.path().to_owned());
        opts.insecure = true;
        assert!(Host::start(opts).await.is_err());
    }

    #[test]
    fn events_serialize_for_the_ui() {
        let e = HostEvent::KeyPressed {
            device_id: "d".into(),
            key_index: 3,
        };
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"kind":"keyPressed","deviceId":"d","keyIndex":3}"#
        );
    }
}
