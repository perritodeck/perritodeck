//! The running host, as a library: shared by the `perritodeck-host` command line and
//! the PerritoDeck desktop app.
//!
//! [`Host::start`] loads the identity, paired devices, profiles and integrations, starts
//! the TLS server and mDNS advertisement, and wires server events to the deck. The
//! returned [`Host`] answers the questions a UI asks (who's paired, open a pairing
//! window, what's on each page), edits profiles, and broadcasts [`HostEvent`]s.

pub mod focus;
mod live;
pub mod settings;
pub mod usb;
pub mod views;

use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
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
use perritodeck_deck::{watch, Deck, Outbox};
use perritodeck_profile::Profile;
use perritodeck_protocol::{Message, DEFAULT_PORT};
use serde::Serialize;
use tokio::{net::TcpListener, sync::broadcast, task::JoinHandle};
use tracing::{info, warn};

pub use perritodeck_actions::ActionSpec;
pub use perritodeck_plugins::{
    BrokenPlugin, Manifest as PluginManifest, PluginInfo, Status as PluginStatus,
};
pub use perritodeck_profile::edit::{Edit, KeyFields};
pub use settings::{AppSettings, AutoSwitch, Network, SwitchRule};
pub mod companion;
pub mod online;
pub mod updates;
pub use views::{view_of, view_with_icons, KeyView, PageView, ProfileSummary, ProfileView};

#[derive(Debug, Clone)]
pub struct HostOptions {
    pub listen: IpAddr,
    pub port: u16,
    pub heartbeat: Duration,
    pub data_dir: PathBuf,
    /// The default profile's file. Other profiles live next to it.
    pub profile: Option<PathBuf>,
    pub insecure: bool,
    pub mdns: bool,
    /// Watch which app is in front, for auto-switching profiles.
    pub watch_focus: bool,
    /// Forward the port to tablets on USB with `adb reverse`, when adb is installed.
    pub usb: bool,
    /// Folder of plugins that ship with the app (one folder each), offered for install.
    pub plugin_catalog: Option<PathBuf>,
    /// The online plugin catalogue's `index.json`.
    pub plugin_index: String,
    /// Fixed pairing codes (tests only).
    pub pair_codes: Vec<String>,
}

impl HostOptions {
    /// Use the port and Wi-Fi choice saved in the app's settings.
    pub fn with_saved_network(mut self) -> Self {
        let n = AppSettings::load(&settings_path(&self.data_dir)).network;
        self.port = n.port;
        self.listen = if n.lan {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        self
    }

    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            listen: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: DEFAULT_PORT,
            heartbeat: Duration::from_secs(2),
            data_dir,
            profile: None,
            insecure: false,
            mdns: true,
            watch_focus: true,
            usb: true,
            plugin_catalog: None,
            plugin_index: online::DEFAULT_INDEX.to_owned(),
            pair_codes: Vec::new(),
        }
    }
}

/// The user's settings folder: `~/.config/perritodeck` on Linux,
/// `%APPDATA%\perritodeck\PerritoDeck\config` on Windows,
/// `~/Library/Application Support/dev.perritodeck.PerritoDeck` on macOS.
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

pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join("app-settings.json")
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
    /// A profile was loaded, reloaded, removed or rejected, a tablet switched profile,
    /// or keys were paused or resumed.
    ProfileChanged,
    /// A different app came to the front.
    #[serde(rename_all = "camelCase")]
    ForegroundApp { app: Option<String> },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    /// Unix seconds.
    pub last_seen: Option<u64>,
    pub connected: bool,
    /// The profile it shows now (or will show when it connects).
    pub profile: String,
    /// The profile chosen for it; `None` means the default.
    pub assigned_profile: Option<String>,
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
    pub profiles_dir: String,
    pub integrations: Vec<String>,
    pub paused: bool,
    /// False where the desktop doesn't tell apps which app is in front (Wayland).
    pub auto_switch_supported: bool,
    /// Tablets on a USB cable that can reach this computer (needs `adb`).
    pub usb_devices: Vec<String>,
    /// Whether `adb` was found, so USB tablets work.
    pub usb_available: bool,
    /// How the connection to Bitfocus Companion is doing, if it's set up.
    pub companion: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginsView {
    pub installed: Vec<PluginInfo>,
    /// Plugin folders whose manifest couldn't be read, and why.
    pub broken: Vec<BrokenPlugin>,
    /// Where plugins are installed.
    pub folder: String,
    /// Plugins that ship with the app.
    pub catalog: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub manifest: PluginManifest,
    pub installed: bool,
}

/// Stream Deck plugins on this computer.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SdPluginsView {
    pub installed: Vec<perritodeck_sdcompat::PluginInfo>,
    pub broken: Vec<perritodeck_sdcompat::BrokenPlugin>,
    pub folder: String,
}

/// A plugin in the online catalogue, and whether this computer has it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineEntry {
    #[serde(flatten)]
    pub plugin: online::OnlinePlugin,
    /// The installed version, if any.
    pub installed: Option<String>,
    /// The catalogue has a newer version than the installed one.
    pub update: bool,
}

/// Outbox that forwards to the server handle once it exists.
#[derive(Default)]
struct LateOutbox(std::sync::OnceLock<ServerHandle>);

impl Outbox for LateOutbox {
    fn send(&self, device_id: &str, msg: Message) -> bool {
        self.0.get().is_some_and(|h| h.send(device_id, msg))
    }
}

/// One edit made through the app: the file, and its text before and after.
struct Change {
    path: PathBuf,
    before: String,
    after: String,
}

/// Edits made through the app.
#[derive(Default)]
struct History {
    undo: Vec<Change>,
    redo: Vec<Change>,
}

const HISTORY_LIMIT: usize = 200;
const RECENT_APPS: usize = 12;

/// State the background tasks share with [`Host`].
struct Shared {
    deck: Arc<Deck>,
    settings: Mutex<AppSettings>,
    settings_path: PathBuf,
    foreground: Mutex<Option<String>>,
    recent_apps: Mutex<VecDeque<String>>,
    events: broadcast::Sender<HostEvent>,
}

impl Shared {
    /// Show each connected tablet the profile it should have right now: a matching
    /// auto-switch rule's, else its assigned one, else the default.
    fn apply_auto_switch(&self) {
        let settings = self.settings.lock().expect("settings poisoned").clone();
        let app = self.foreground.lock().expect("focus poisoned").clone();
        let loaded = self.deck.profile_ids();
        let default = self.deck.default_profile_id();
        for (device, showing) in self.deck.connected_devices() {
            let wanted = settings
                .auto_switch
                .profile_for(app.as_deref(), &device)
                .filter(|p| loaded.iter().any(|l| l == p))
                .map(str::to_owned)
                .or_else(|| {
                    settings
                        .device_profiles
                        .get(&device)
                        .filter(|p| loaded.contains(p))
                        .cloned()
                })
                .unwrap_or_else(|| default.clone());
            if wanted != showing {
                self.deck.show_profile(&device, &wanted);
            }
        }
    }

    fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        settings
            .save(&self.settings_path)
            .map_err(|e| format!("Couldn't save {}: {e}", self.settings_path.display()))
    }
}

pub struct Host {
    pub host_name: String,
    pub addr: SocketAddr,
    pub lan: Vec<IpAddr>,
    /// The default profile's file.
    pub profile_path: PathBuf,
    /// Where every profile lives (the default profile's folder).
    pub profiles_dir: PathBuf,
    pub data_dir: PathBuf,
    pub identity: Option<HostIdentity>,
    pub store: Option<Arc<PairingStore>>,
    pub handle: ServerHandle,
    pub deck: Arc<Deck>,
    insecure: bool,
    integrations: Vec<String>,
    shared: Arc<Shared>,
    plugins: Arc<perritodeck_plugins::PluginHost>,
    plugin_catalog: Option<PathBuf>,
    plugin_index: String,
    sd_plugins: Option<perritodeck_sdcompat::SdHost>,
    companion: companion::CompanionState,
    usb: usb::UsbDevices,
    usb_available: bool,
    history: Mutex<History>,
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
        let profiles_dir = profile_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let default_id = watch::profile_id(&profile_path).unwrap_or_else(|| "default".into());
        for m in profile.missing_icons(&profiles_dir) {
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
        let companion_config = integrations_config.companion.clone();
        let builtins = perritodeck_integrations::Builtins::new(integrations_config);
        let mut integrations: Vec<String> = builtins
            .configured()
            .into_iter()
            .map(str::to_owned)
            .collect();
        if companion_config.is_some() {
            integrations.push("Companion".into());
        }
        if !integrations.is_empty() {
            info!("integrations: {}", integrations.join(", "));
        }
        // Plugins publish live values into the deck, which exists only below; a shim
        // forwards them once it does.
        let live_sink: Arc<std::sync::OnceLock<Arc<Deck>>> = Arc::default();
        let sink = live_sink.clone();
        let plugins = perritodeck_plugins::PluginHost::start(
            dir.join("plugins"),
            env!("CARGO_PKG_VERSION"),
            Arc::new(move |k, v| {
                if let Some(deck) = sink.get() {
                    deck.set_live([(k, v)]);
                }
            }),
        );
        let executor = Executor::new(input, Arc::new(OpenSystem))
            .with_integrations(builtins)
            .with_plugins(plugins.clone());
        let test_executor = executor.clone();

        let outbox = Arc::new(LateOutbox::default());
        let deck = Deck::with_default(
            &default_id,
            profile,
            profiles_dir.clone(),
            outbox.clone(),
            executor,
        );
        watch::load_all(&profiles_dir, &deck);
        let _ = live_sink.set(deck.clone());
        // Stream Deck plugins run beside PerritoDeck's own.
        let sd_plugins = match perritodeck_sdcompat::SdHost::start(dir.join("sdplugins")).await {
            Ok(sd) => {
                sd.attach(&deck);
                Some(sd)
            }
            Err(e) => {
                warn!("Stream Deck plugins are off: {e}");
                None
            }
        };
        let companion_state = companion::CompanionState::default();
        if let Some(c) = companion_config {
            info!("Companion: connecting to {}:{}", c.host, c.port);
            let names = store.clone();
            let bridge = companion::Bridge::start(
                c.host,
                c.port,
                Arc::downgrade(&deck),
                Box::new(move |id| {
                    names
                        .as_ref()
                        .and_then(|s| s.devices().into_iter().find(|(d, _)| d == id))
                        .map(|(_, d)| d.name)
                }),
                companion_state.clone(),
            );
            deck.set_remote(bridge);
        }

        let settings_file = settings_path(&dir);
        let settings = AppSettings::load(&settings_file);
        for (device, profile) in &settings.device_profiles {
            if !deck.assign(device, Some(profile)) {
                warn!("{device} was set to show profile {profile}, which isn't there; showing the default");
            }
        }

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

        let watcher = watch::watch_profiles(profiles_dir.clone(), deck.clone())
            .map_err(|e| {
                warn!(
                    "won't notice edits to {} until restart: {e}",
                    profiles_dir.display()
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
        let shared = Arc::new(Shared {
            deck: deck.clone(),
            settings: Mutex::new(settings),
            settings_path: settings_file,
            foreground: Mutex::new(None),
            recent_apps: Mutex::new(VecDeque::new()),
            events: events.clone(),
        });
        spawn_event_loop(shared.clone(), server_events);
        live::spawn(deck.clone(), test_executor.integrations_handle());
        {
            // Plugins hear which of their values profiles show (e.g. `up:<host>`).
            let (deck, plugins) = (deck.clone(), plugins.clone());
            tokio::spawn(async move {
                let mut every = tokio::time::interval(Duration::from_secs(2));
                loop {
                    every.tick().await;
                    plugins.watch(&deck.live_tokens());
                }
            });
        }
        if opts.watch_focus && focus::supported() {
            spawn_focus_watch(shared.clone());
        }
        let usb_devices = usb::UsbDevices::default();
        let usb_available = opts.usb && !opts.insecure && usb::find_adb().is_some();
        if usb_available {
            usb::spawn(addr.port(), usb_devices.clone());
        }
        let server = tokio::spawn(server.run());

        Ok(Self {
            plugins,
            plugin_catalog: opts.plugin_catalog.clone(),
            plugin_index: opts.plugin_index.clone(),
            sd_plugins,
            companion: companion_state,
            usb: usb_devices,
            usb_available,
            host_name,
            addr,
            lan,
            profile_path,
            profiles_dir,
            data_dir: dir,
            identity,
            store,
            handle,
            deck,
            insecure: opts.insecure,
            integrations,
            shared,
            history: Default::default(),
            executor: test_executor,
            server,
            _watcher: watcher,
            _mdns: mdns,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HostEvent> {
        self.shared.events.subscribe()
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
            profiles_dir: self.profiles_dir.display().to_string(),
            integrations: self.integrations.clone(),
            paused: self.deck.paused(),
            auto_switch_supported: focus::supported(),
            usb_devices: self
                .usb
                .lock()
                .expect("usb poisoned")
                .iter()
                .cloned()
                .collect(),
            usb_available: self.usb_available,
            companion: self
                .companion
                .lock()
                .ok()
                .map(|s| s.clone())
                .filter(|_| self.integrations.iter().any(|i| i == "Companion")),
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
        let assigned = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .device_profiles
            .clone();
        let info = |id: String, name: String, last_seen, connected| DeviceInfo {
            profile: self.deck.device_profile(&id),
            assigned_profile: assigned.get(&id).cloned(),
            id,
            name,
            last_seen,
            connected,
        };
        let Some(store) = &self.store else {
            return online
                .into_iter()
                .map(|id| info(id.clone(), id, None, true))
                .collect();
        };
        let mut list: Vec<DeviceInfo> = store
            .devices()
            .into_iter()
            .map(|(id, d)| {
                let connected = online.contains(&id);
                info(id, d.name, d.last_seen, connected)
            })
            .collect();
        list.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.cmp(&b.name)));
        list
    }

    /// Forget a device and disconnect it now.
    pub fn revoke(&self, device_id: &str) -> bool {
        let removed = self.store.as_ref().is_some_and(|s| s.revoke(device_id));
        if removed {
            self.handle.kick(device_id);
            let mut settings = self.shared.settings.lock().expect("settings poisoned");
            if settings.device_profiles.remove(device_id).is_some() {
                let _ = self.shared.save_settings(&settings);
            }
        }
        removed
    }

    pub fn rename_device(&self, device_id: &str, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A tablet needs a name.".into());
        }
        let store = self
            .store
            .as_ref()
            .ok_or("Pairing is off in development mode.")?;
        if !store.rename(device_id, name) {
            return Err("That tablet isn't paired.".into());
        }
        Ok(())
    }

    /// Choose the profile a tablet shows; `None` for the default. Remembered.
    pub fn assign_profile(&self, device_id: &str, profile_id: Option<&str>) -> Result<(), String> {
        if !self.deck.assign(device_id, profile_id) {
            return Err(format!(
                "There's no profile called {}.",
                profile_id.unwrap_or_default()
            ));
        }
        let mut settings = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .clone();
        match profile_id {
            Some(p) if p != self.deck.default_profile_id() => {
                settings
                    .device_profiles
                    .insert(device_id.to_owned(), p.to_owned());
            }
            _ => {
                settings.device_profiles.remove(device_id);
            }
        }
        self.shared.save_settings(&settings)?;
        *self.shared.settings.lock().expect("settings poisoned") = settings;
        self.shared.apply_auto_switch();
        Ok(())
    }

    /// Stop running actions (keys show a "paused" warning) until resumed.
    pub fn set_paused(&self, paused: bool) {
        self.deck.set_paused(paused);
        info!("keys {}", if paused { "paused" } else { "resumed" });
    }

    pub fn paused(&self) -> bool {
        self.deck.paused()
    }

    pub fn auto_switch(&self) -> AutoSwitch {
        self.shared
            .settings
            .lock()
            .expect("settings poisoned")
            .auto_switch
            .clone()
    }

    pub fn set_auto_switch(&self, auto: AutoSwitch) -> Result<(), String> {
        let mut settings = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .clone();
        settings.auto_switch = auto;
        self.shared.save_settings(&settings)?;
        *self.shared.settings.lock().expect("settings poisoned") = settings;
        self.shared.apply_auto_switch();
        Ok(())
    }

    pub fn network(&self) -> settings::Network {
        self.shared
            .settings
            .lock()
            .expect("settings poisoned")
            .network
            .clone()
    }

    /// Save the port and whether Wi-Fi tablets may connect; used from the next start.
    pub fn set_network(&self, network: settings::Network) -> Result<(), String> {
        if network.port < 1024 {
            return Err("Use a port from 1024 up.".into());
        }
        let mut settings = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .clone();
        settings.network = network;
        self.shared.save_settings(&settings)?;
        *self.shared.settings.lock().expect("settings poisoned") = settings;
        Ok(())
    }

    pub fn check_updates_enabled(&self) -> bool {
        self.shared
            .settings
            .lock()
            .expect("settings poisoned")
            .check_updates
    }

    pub fn set_check_updates(&self, on: bool) -> Result<(), String> {
        let mut settings = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .clone();
        settings.check_updates = on;
        self.shared.save_settings(&settings)?;
        *self.shared.settings.lock().expect("settings poisoned") = settings;
        Ok(())
    }

    /// Apps seen in front recently, newest first, for making auto-switch rules.
    pub fn recent_apps(&self) -> Vec<String> {
        self.shared
            .recent_apps
            .lock()
            .expect("apps poisoned")
            .iter()
            .cloned()
            .collect()
    }

    pub fn foreground_app(&self) -> Option<String> {
        self.shared
            .foreground
            .lock()
            .expect("focus poisoned")
            .clone()
    }

    // ---- Plugins ----

    pub fn plugins(&self) -> PluginsView {
        let installed = self.plugins.list();
        let catalog = self
            .catalog_entries()
            .into_iter()
            .map(|(manifest, _)| CatalogEntry {
                installed: installed.iter().any(|p| p.manifest.id == manifest.id),
                manifest,
            })
            .collect();
        PluginsView {
            installed,
            broken: self.plugins.broken(),
            folder: self.plugins.folder().display().to_string(),
            catalog,
        }
    }

    fn catalog_entries(&self) -> Vec<(PluginManifest, PathBuf)> {
        let Some(dir) = &self.plugin_catalog else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out: Vec<(PluginManifest, PathBuf)> = entries
            .flatten()
            .filter_map(|e| {
                let text =
                    std::fs::read_to_string(e.path().join(perritodeck_plugins::MANIFEST)).ok()?;
                Some((PluginManifest::parse(&text).ok()?, e.path()))
            })
            .collect();
        out.sort_by(|a, b| a.0.name.cmp(&b.0.name));
        out
    }

    /// Install (or update) a plugin that ships with the app.
    pub fn install_from_catalog(&self, id: &str) -> Result<String, String> {
        let (_, folder) = self
            .catalog_entries()
            .into_iter()
            .find(|(m, _)| m.id == id)
            .ok_or_else(|| format!("{id} doesn't ship with this version."))?;
        self.plugins.install_from(&folder)
    }

    /// What the online catalogue offers. Blocks on the network; call it off the UI thread.
    pub fn online_plugins(&self) -> Result<Vec<OnlineEntry>, String> {
        let installed: HashMap<String, String> = self
            .plugins
            .list()
            .into_iter()
            .map(|p| (p.manifest.id, p.manifest.version))
            .collect();
        let mut out: Vec<OnlineEntry> = online::fetch_index(&self.plugin_index)?
            .into_iter()
            .map(|plugin| {
                let have = installed.get(&plugin.id).cloned();
                let update = have
                    .as_deref()
                    .is_some_and(|v| updates::newer(&plugin.version, v));
                OnlineEntry {
                    plugin,
                    installed: have,
                    update,
                }
            })
            .collect();
        out.sort_by(|a, b| a.plugin.name.cmp(&b.plugin.name));
        Ok(out)
    }

    /// Download a plugin from the online catalogue (checking its SHA-256), then install
    /// or update it. Blocks on the network.
    pub fn install_online_plugin(&self, id: &str) -> Result<String, String> {
        let entry = online::fetch_index(&self.plugin_index)?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("{id} isn't in the plugin catalogue."))?;
        let zip = online::download(&self.plugin_index, &entry)?;
        let tmp = tempfile::Builder::new()
            .prefix("plugin-download-")
            .tempdir_in(&self.data_dir)
            .map_err(|e| format!("Couldn't unpack the plugin: {e}"))?;
        let folder = online::unpack(&zip, &tmp.path().join("p"))?;
        let text = std::fs::read_to_string(folder.join(perritodeck_plugins::MANIFEST))
            .map_err(|e| e.to_string())?;
        let manifest = PluginManifest::parse(&text)?;
        if manifest.id != entry.id {
            return Err(format!(
                "The download is {}, not {}, so it wasn't installed.",
                manifest.id, entry.id
            ));
        }
        self.plugins.install_from(&folder)
    }

    // ---- Stream Deck plugins ----

    fn sd(&self) -> Result<&perritodeck_sdcompat::SdHost, String> {
        self.sd_plugins
            .as_ref()
            .ok_or_else(|| "Stream Deck plugins couldn't start on this computer.".to_owned())
    }

    /// Installed Stream Deck plugins, for the Plugins page and the key editor.
    pub fn sd_plugins(&self) -> SdPluginsView {
        match &self.sd_plugins {
            Some(sd) => SdPluginsView {
                installed: sd.list(),
                broken: sd.broken(),
                folder: sd.folder().display().to_string(),
            },
            None => SdPluginsView::default(),
        }
    }

    /// Install a `.streamDeckPlugin` file or a `.sdPlugin` folder.
    pub fn install_sd_plugin(&self, src: &Path) -> Result<String, String> {
        self.sd()?.install(src)
    }

    /// The address of the Stream Deck plugin settings page for the key at `row`/`col` of
    /// a page, for the app to show in a frame. Save the key's action first.
    pub fn sd_settings_page(
        &self,
        profile: &str,
        page: &str,
        row: u32,
        col: u32,
    ) -> Result<String, String> {
        let key = self
            .deck
            .agent_key_for(profile, page, row, col)
            .ok_or_else(|| "That key doesn't run a Stream Deck plugin action yet.".to_owned())?;
        self.sd()?.settings_page(&key)
    }

    pub fn remove_sd_plugin(&self, uuid: &str) -> Result<(), String> {
        self.sd()?.remove(uuid)
    }

    pub fn restart_sd_plugin(&self, uuid: &str) -> Result<(), String> {
        self.sd()?.restart(uuid)
    }

    /// Copy a plugin's folder into the plugins folder and start it.
    pub fn install_plugin(&self, folder: &Path) -> Result<String, String> {
        self.plugins.install_from(folder)
    }

    /// A plugin's own settings (server address, token…), as saved on the Plugins page.
    pub fn plugin_settings(&self, id: &str) -> Result<serde_json::Value, String> {
        self.plugins.settings(id)
    }

    pub fn set_plugin_settings(&self, id: &str, settings: serde_json::Value) -> Result<(), String> {
        self.plugins.set_settings(id, settings)
    }

    pub fn remove_plugin(&self, id: &str) -> Result<(), String> {
        self.plugins.remove(id)
    }

    pub fn restart_plugin(&self, id: &str) -> Result<(), String> {
        self.plugins.restart(id)
    }

    /// Look again for plugins added or removed by hand.
    pub fn rescan_plugins(&self) {
        self.plugins.scan();
    }

    // ---- Profiles ----

    fn path_of(&self, id: &str) -> PathBuf {
        self.profiles_dir.join(format!("{id}.toml"))
    }

    pub fn profiles(&self) -> Vec<ProfileSummary> {
        let default = self.deck.default_profile_id();
        let mut list: Vec<ProfileSummary> = self
            .deck
            .profile_ids()
            .into_iter()
            .filter_map(|id| {
                let p = self.deck.profile_by_id(&id)?;
                Some(ProfileSummary {
                    is_default: id == default,
                    has_error: self.deck.reload_error_of(&id).is_some(),
                    name: p.name,
                    pages: p.pages.len(),
                    id,
                })
            })
            .collect();
        list.sort_by(|a, b| {
            b.is_default
                .cmp(&a.is_default)
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        list
    }

    /// A profile as the editor shows it; `None` for the default.
    pub fn profile_view(&self, id: Option<&str>) -> Result<ProfileView, String> {
        let id = id
            .map(str::to_owned)
            .unwrap_or_else(|| self.deck.default_profile_id());
        let profile = self
            .deck
            .profile_by_id(&id)
            .ok_or_else(|| format!("There's no profile called {id}."))?;
        let mut view = view_with_icons(
            &id,
            &profile,
            self.deck.reload_error_of(&id),
            Some(&self.profiles_dir),
        );
        let history = self.history.lock().expect("history poisoned");
        view.can_undo = !history.undo.is_empty();
        view.can_redo = !history.redo.is_empty();
        Ok(view)
    }

    /// Change a profile's file and show the result on the tablets right away.
    pub fn edit_profile(&self, id: &str, edits: &[Edit]) -> Result<ProfileView, String> {
        if self.deck.profile_by_id(id).is_none() {
            return Err(format!("There's no profile called {id}."));
        }
        let path = self.path_of(id);
        let before = read_text(&path)?;
        let after = perritodeck_profile::edit::apply(&before, edits).map_err(|e| e.to_string())?;
        if after != before {
            self.write_profile(&path, &after)?;
            self.record(Change {
                path,
                before,
                after,
            });
        }
        self.profile_view(Some(id))
    }

    fn record(&self, change: Change) {
        let mut history = self.history.lock().expect("history poisoned");
        history.undo.push(change);
        if history.undo.len() > HISTORY_LIMIT {
            history.undo.remove(0);
        }
        history.redo.clear();
    }

    /// Make a new profile, empty or as a copy of another. Returns its id.
    pub fn create_profile(&self, name: &str, copy_of: Option<&str>) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A profile needs a name.".into());
        }
        let text = match copy_of {
            Some(src) => {
                let text = read_text(&self.path_of(src))?;
                perritodeck_profile::edit::apply(
                    &text,
                    &[Edit::SetName {
                        name: name.to_owned(),
                    }],
                )
                .map_err(|e| e.to_string())?
            }
            None => blank_profile(name),
        };
        let id = unique_id(&slug(name), |id| self.path_of(id).exists());
        self.write_profile(&self.path_of(&id), &text)?;
        Ok(id)
    }

    /// A new profile that shows Bitfocus Companion's buttons.
    pub fn create_companion_profile(&self, name: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A profile needs a name.".into());
        }
        let mut doc = toml_edit::DocumentMut::new();
        doc["schema_version"] = toml_edit::value(i64::from(perritodeck_profile::SCHEMA_VERSION));
        doc["name"] = toml_edit::value(name);
        doc["companion"] = toml_edit::value(true);
        let text = format!(
            "# Shows the buttons from Bitfocus Companion. Tell PerritoDeck where Companion runs in\n\
             # integrations.toml:\n#   [companion]\n#   host = \"127.0.0.1\"\n{doc}"
        );
        let id = unique_id(&slug(name), |id| self.path_of(id).exists());
        self.write_profile(&self.path_of(&id), &text)?;
        Ok(id)
    }

    /// Copy an image into the profiles folder for use as a key icon: PNG or JPEG,
    /// scaled down to at most 288 px. Returns its path relative to the folder.
    pub fn import_icon(&self, source: &Path) -> Result<String, String> {
        let bytes = std::fs::read(source)
            .map_err(|e| format!("Couldn't read {}: {e}", source.display()))?;
        self.import_icon_bytes(&bytes)
    }

    /// Like [`Host::import_icon`], from the image's bytes (PNG, JPEG or GIF). Animated
    /// GIFs are kept as they are, so the key animates.
    pub fn import_icon_bytes(&self, bytes: &[u8]) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        const MAX: u32 = 288;
        const MAX_GIF_BYTES: usize = 4 * 1024 * 1024;
        if perritodeck_render::decode_animation(bytes).is_some() {
            if bytes.len() > MAX_GIF_BYTES {
                return Err("That animation is too big; use a GIF under 4 MB.".into());
            }
            let hash = Sha256::digest(bytes);
            let name: String = hash.iter().take(6).map(|b| format!("{b:02x}")).collect();
            let dir = self.profiles_dir.join("icons");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(format!("{name}.gif")), bytes).map_err(|e| e.to_string())?;
            return Ok(format!("icons/{name}.gif"));
        }
        let img = image::load_from_memory(bytes)
            .map_err(|_| "That file isn't a PNG, JPEG or GIF image.".to_owned())?;
        let img = if img.width() > MAX || img.height() > MAX {
            img.resize(MAX, MAX, image::imageops::FilterType::Lanczos3)
        } else {
            img
        };
        let mut png = Vec::new();
        img.to_rgba8()
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| format!("Couldn't convert the image: {e}"))?;
        let hash = Sha256::digest(&png);
        let name: String = hash.iter().take(6).map(|b| format!("{b:02x}")).collect();
        let dir = self.profiles_dir.join("icons");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("{name}.png")), &png).map_err(|e| e.to_string())?;
        Ok(format!("icons/{name}.png"))
    }

    /// Save a profile and the images it uses as one `.perritoprofile` file (a zip) to
    /// share or keep.
    pub fn export_profile(&self, id: &str, dest: &Path) -> Result<(), String> {
        use std::io::Write;
        let profile = self
            .deck
            .profile_by_id(id)
            .ok_or_else(|| format!("There's no profile called {id}."))?;
        let text = read_text(&self.path_of(id))?;
        let file = std::fs::File::create(dest)
            .map_err(|e| format!("Couldn't create {}: {e}", dest.display()))?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        let fail = |e: &dyn std::fmt::Display| format!("Couldn't write {}: {e}", dest.display());
        zip.start_file("profile.toml", opts).map_err(|e| fail(&e))?;
        zip.write_all(text.as_bytes()).map_err(|e| fail(&e))?;
        let mut icons: Vec<PathBuf> = profile
            .pages
            .iter()
            .flat_map(|p| p.keys.iter())
            .flat_map(|k| {
                k.icon
                    .iter()
                    .chain(k.states.iter().filter_map(|s| s.icon.as_ref()))
            })
            .cloned()
            .collect();
        icons.sort();
        icons.dedup();
        for icon in icons {
            // Only images inside the profiles folder travel with it.
            let rel = icon.to_string_lossy().replace('\\', "/");
            if icon.is_absolute() || rel.split('/').any(|part| part == "..") {
                continue;
            }
            if let Ok(bytes) = std::fs::read(self.profiles_dir.join(&icon)) {
                zip.start_file(format!("images/{rel}"), opts)
                    .map_err(|e| fail(&e))?;
                zip.write_all(&bytes).map_err(|e| fail(&e))?;
            }
        }
        zip.finish().map_err(|e| fail(&e))?;
        Ok(())
    }

    /// Add a profile from a `.perritoprofile` file. Returns its id.
    pub fn import_profile(&self, src: &Path) -> Result<String, String> {
        use std::io::Read;
        const MAX: u64 = 20 * 1024 * 1024;
        let bad = || format!("{} isn't a PerritoDeck profile file.", src.display());
        let file = std::fs::File::open(src)
            .map_err(|e| format!("Couldn't open {}: {e}", src.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|_| bad())?;
        let mut text = String::new();
        let mut images: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|_| bad())?;
            if entry.size() > MAX {
                return Err(bad());
            }
            let name = entry.name().to_owned();
            if name == "profile.toml" {
                entry.read_to_string(&mut text).map_err(|_| bad())?;
            } else if let Some(rel) = name.strip_prefix("images/") {
                // Never write outside the profiles folder.
                let safe = !rel.is_empty()
                    && !rel.starts_with('/')
                    && !rel.contains('\\')
                    && rel.split('/').all(|p| !p.is_empty() && p != "..");
                if safe && !entry.is_dir() {
                    let mut bytes = Vec::new();
                    entry.read_to_end(&mut bytes).map_err(|_| bad())?;
                    images.push((rel.to_owned(), bytes));
                }
            }
        }
        let profile = Profile::parse(&text).map_err(|e| format!("{}: {e}", bad()))?;
        let problems = profile.problems();
        if !problems.is_empty() {
            return Err(format!(
                "{} has problems: {}",
                src.display(),
                problems.join("; ")
            ));
        }
        for (rel, bytes) in images {
            let path = self.profiles_dir.join(&rel);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            // Images are named by content, so an existing file is the same image.
            if !path.exists() {
                std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            }
        }
        let id = unique_id(&slug(&profile.name), |id| self.path_of(id).exists());
        self.write_profile(&self.path_of(&id), &text)?;
        Ok(id)
    }

    /// Delete a profile (the default can't be). Its file is kept in the settings
    /// folder's `deleted-profiles`, in case.
    pub fn delete_profile(&self, id: &str) -> Result<(), String> {
        if id == self.deck.default_profile_id() {
            return Err("The default profile can't be deleted.".into());
        }
        let path = self.path_of(id);
        let keep = self.data_dir.join("deleted-profiles");
        std::fs::create_dir_all(&keep).map_err(|e| e.to_string())?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        std::fs::rename(&path, keep.join(format!("{id}-{stamp}.toml")))
            .or_else(|_| std::fs::remove_file(&path))
            .map_err(|e| format!("Couldn't delete {}: {e}", path.display()))?;
        self.deck.remove_profile(id);
        // Rules and tablets that used it fall back to the default.
        let mut settings = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .clone();
        settings.device_profiles.retain(|_, p| p != id);
        settings.auto_switch.rules.retain(|r| r.profile != id);
        self.shared.save_settings(&settings)?;
        *self.shared.settings.lock().expect("settings poisoned") = settings;
        let mut history = self.history.lock().expect("history poisoned");
        history.undo.retain(|c| c.path != path);
        history.redo.retain(|c| c.path != path);
        Ok(())
    }

    /// Run an action now, as if its key were pressed. `Ok` carries a note to show when
    /// the action ran but reported a problem (a command that failed, say).
    pub async fn test_action(&self, spec: &ActionSpec) -> Result<Option<String>, String> {
        match self.executor.run(spec).await {
            Ok(perritodeck_actions::Outcome::Ok) => Ok(None),
            Ok(perritodeck_actions::Outcome::Alert(note)) => Ok(Some(note)),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Step back through the app's edits (across profiles). Edits made in a text
    /// editor aren't undone here. Returns the profile that changed.
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
        let Some(change) = entry else {
            return Err(format!(
                "Nothing to {}.",
                if back { "undo" } else { "redo" }
            ));
        };
        let (expected, target) = if back {
            (&change.after, &change.before)
        } else {
            (&change.before, &change.after)
        };
        if read_text(&change.path)? != *expected {
            history.undo.clear();
            history.redo.clear();
            return Err("The profile file was changed outside PerritoDeck, so earlier edits can't be undone.".into());
        }
        self.write_profile(&change.path, target)?;
        let id = watch::profile_id(&change.path).unwrap_or_default();
        if back {
            history.redo.push(change);
        } else {
            history.undo.push(change);
        }
        drop(history);
        self.profile_view(Some(&id))
    }

    /// Save atomically (so a crash never leaves half a file), then load it into the
    /// deck now rather than waiting for the file watcher.
    fn write_profile(&self, path: &Path, text: &str) -> Result<(), String> {
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
        if let Some(id) = watch::profile_id(path) {
            self.deck
                .load_profile(&id, profile, Some(self.profiles_dir.clone()));
        }
        Ok(())
    }
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))
}

fn blank_profile(name: &str) -> String {
    let mut doc = toml_edit::DocumentMut::new();
    doc["schema_version"] = toml_edit::value(i64::from(perritodeck_profile::SCHEMA_VERSION));
    doc["name"] = toml_edit::value(name);
    let mut page = toml_edit::Table::new();
    page["name"] = toml_edit::value("Main");
    let mut pages = toml_edit::ArrayOfTables::new();
    pages.push(page);
    doc["pages"] = toml_edit::Item::ArrayOfTables(pages);
    format!("# PerritoDeck profile. Edit it in the app, or by hand; see the default profile for every option.\n{doc}")
}

/// A file-name-safe id from a name: "Streaming (OBS)" → "streaming-obs".
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "profile".into()
    } else {
        out
    }
}

fn unique_id(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .expect("unbounded")
}

/// Feed server events to the deck, keep live titles ticking, and tell subscribers.
fn spawn_event_loop(
    shared: Arc<Shared>,
    mut server_events: tokio::sync::mpsc::UnboundedReceiver<ServerEvent>,
) {
    let animator = shared.deck.clone();
    tokio::spawn(async move {
        // Animated (GIF) keys; cheap when there are none.
        let mut every = tokio::time::interval(Duration::from_millis(40));
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            every.tick().await;
            animator.animate();
        }
    });
    let ticker = shared.deck.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            ticker.tick();
        }
    });
    let sliders = shared.deck.clone();
    tokio::spawn(async move {
        // Keep sliders in step with changes made on the computer.
        let mut every = tokio::time::interval(Duration::from_secs(3));
        loop {
            every.tick().await;
            sliders.poll_sliders().await;
        }
    });
    let watched = shared.clone();
    tokio::spawn(async move {
        let mut seen = watched.deck.revision();
        let mut every = tokio::time::interval(Duration::from_millis(200));
        loop {
            every.tick().await;
            let now = watched.deck.revision();
            if now != seen {
                seen = now;
                let _ = watched.events.send(HostEvent::ProfileChanged);
            }
        }
    });
    tokio::spawn(async move {
        let deck = shared.deck.clone();
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
                    shared.apply_auto_switch();
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
                ServerEvent::KeyUp {
                    device_id,
                    key_index,
                } => {
                    deck.key_up(&device_id, key_index);
                    None
                }
                ServerEvent::SliderChange {
                    device_id,
                    key_index,
                    value,
                } => {
                    deck.slider_change(&device_id, key_index, value);
                    None
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
                let _ = shared.events.send(e);
            }
        }
    });
}

/// Names our own windows go by; they never count as "the app in front".
fn is_us(app: &str) -> bool {
    let a = app.to_ascii_lowercase();
    a.starts_with("perritodeck")
}

/// Poll which app is in front and switch profiles when it changes.
fn spawn_focus_watch(shared: Arc<Shared>) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_millis(700));
        let mut last: Option<String> = None;
        loop {
            every.tick().await;
            let Ok(app) = tokio::task::spawn_blocking(focus::foreground_app).await else {
                continue;
            };
            // Clicking over to PerritoDeck itself shouldn't switch the tablets away.
            if app.as_deref().is_some_and(is_us) || app == last {
                continue;
            }
            last.clone_from(&app);
            if let Some(a) = &app {
                let mut recent = shared.recent_apps.lock().expect("apps poisoned");
                recent.retain(|r| !r.eq_ignore_ascii_case(a));
                recent.push_front(a.clone());
                recent.truncate(RECENT_APPS);
            }
            *shared.foreground.lock().expect("focus poisoned") = app.clone();
            shared.apply_auto_switch();
            let _ = shared.events.send(HostEvent::ForegroundApp { app });
        }
    });
}

#[cfg(test)]
mod tests;
