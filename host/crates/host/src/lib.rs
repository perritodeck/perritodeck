//! The running host, as a library: shared by the `perritodeck-host` command line and
//! the PerritoDeck desktop app.
//!
//! [`Host::start`] loads the identity, paired devices, profiles and integrations, starts
//! the TLS server and mDNS advertisement, and wires server events to the deck. The
//! returned [`Host`] answers the questions a UI asks (who's paired, open a pairing
//! window, what's on each page), edits profiles, and broadcasts [`HostEvent`]s.

pub mod diagnostics;
pub mod focus;
mod import;
mod live;
pub mod logging;
pub mod mcp;
pub mod settings;
pub mod usb;
pub mod views;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
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

pub use import::{ImportChoice, ImportPreview, ImportStep, Imported};
pub use mcp::{McpSnippets, McpState, McpView};
pub use perritodeck_actions::ActionSpec;
pub use perritodeck_mcp::bridge_stdio_to_http;
pub use perritodeck_plugins::{
    BrokenPlugin, Manifest as PluginManifest, PluginInfo, Status as PluginStatus,
};
pub use perritodeck_profile::edit::{Edit, KeyFields};
pub use settings::{AppSettings, AutoSwitch, Mcp as McpSettings, Network, SwitchRule};
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
    /// The stdio bridge for AI assistants that ships with this program
    /// ([`mcp::shipped_bridge`]), copied into the data folder at start.
    pub mcp_bridge: Option<PathBuf>,
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
            mcp_bridge: None,
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

/// Whether tablets on a USB cable reach a host listening on `listen`: adb's tunnel
/// arrives at 127.0.0.1, so it takes every address or the loopback one.
pub fn usb_reaches(listen: IpAddr) -> bool {
    listen.is_unspecified() || listen.is_loopback()
}

/// The live values installed plugins declare, as the deck matches them:
/// `plugin:<id>:<value>`, with a trailing `:` for values that take an argument.
pub fn declared_plugin_values(plugins: &[PluginInfo]) -> HashSet<String> {
    plugins
        .iter()
        .flat_map(|p| {
            p.manifest.values.iter().map(move |v| {
                let tail = if v.arg.is_some() { ":" } else { "" };
                format!("plugin:{}:{}{tail}", p.manifest.id, v.id)
            })
        })
        .collect()
}

/// Where a file or folder really is, or `path` itself if that can't be found out.
///
/// A program started from inside a packaged Windows app (MSIX, such as Claude's desktop
/// app, or a terminal it opened) sees AppData through a redirection into that app's own
/// folder (`…\Packages\<app>\LocalCache\…`); only the final path shows it. Windows
/// Firewall and every program started normally see the real place.
pub fn real_path(path: &Path) -> PathBuf {
    let Ok(real) = std::fs::canonicalize(path) else {
        return path.to_owned();
    };
    let text = real.to_string_lossy();
    let plain = match (text.strip_prefix(r"\\?\UNC\"), text.strip_prefix(r"\\?\")) {
        (Some(unc), _) => PathBuf::from(format!(r"\\{unc}")),
        (None, Some(local)) => PathBuf::from(local),
        _ => return real,
    };
    // The same place, written differently (case, short names): keep the original.
    if plain
        .to_string_lossy()
        .eq_ignore_ascii_case(&path.to_string_lossy())
    {
        path.to_owned()
    } else {
        plain
    }
}

/// Where the data folder really is (see [`real_path`]). Asked of a file in it: once the
/// folder exists in both places, the folder itself resolves to the real one while the
/// files in it are still the redirected copies.
pub fn real_data_dir(dir: &Path) -> PathBuf {
    let (cert, _) = HostIdentity::paths(dir);
    if cert.is_file() {
        if let Some(parent) = real_path(&cert).parent() {
            if parent != dir {
                return parent.to_owned();
            }
        }
    }
    real_path(dir)
}

/// Whether Windows redirected `path` into a packaged app's own folder (see [`real_path`]).
pub fn redirected_into_package(path: &Path) -> bool {
    in_package_cache(&real_path(path))
}

fn in_package_cache(real: &Path) -> bool {
    let real = real.to_string_lossy().to_ascii_lowercase();
    cfg!(windows) && real.contains(r"\packages\") && real.contains(r"\localcache\")
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

/// The host's own files that sit among the profiles when `--profile` puts them in the
/// data folder `dir`: its integrations file, which holds passwords. They look like
/// profiles but are never read as one.
fn not_profiles(profiles_dir: &Path, dir: &Path) -> Vec<String> {
    [integrations_path(dir)]
        .iter()
        .filter(|f| {
            f.parent()
                .is_some_and(|folder| same_place(folder, profiles_dir))
        })
        .filter_map(|f| Some(f.file_name()?.to_string_lossy().into_owned()))
        .collect()
}

/// Whether two paths name the same file or folder, however they're written.
fn same_place(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
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
    /// A Stream Deck plugin drew keys of a profile differently (at most a few times a
    /// second), so the editor can refresh its preview.
    KeyLooksChanged { profile: String },
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
    /// Where the log files are.
    pub logs_dir: String,
    pub profile_path: String,
    pub profiles_dir: String,
    pub integrations: Vec<String>,
    /// The mistake in `integrations.toml`, if it has one; the file is then ignored, so
    /// OBS, Home Assistant and Companion are all off until it's fixed and the app
    /// restarts.
    pub integrations_error: Option<String>,
    pub paused: bool,
    /// False where the desktop doesn't tell apps which app is in front (Wayland).
    pub auto_switch_supported: bool,
    /// Tablets on a USB cable that can reach this computer (needs `adb`): their port
    /// is forwarded.
    pub usb_devices: Vec<String>,
    /// Tablets on a cable that can't connect yet, or adb not answering, one line each.
    pub usb_problems: Vec<String>,
    /// Whether `adb` was found, so USB tablets work.
    pub usb_available: bool,
    /// How the connection to Bitfocus Companion is doing, if it's set up.
    pub companion: Option<String>,
}

/// Why tablets may not get through to this computer over Wi-Fi, for the Tablets and
/// Pair pages: most often the firewall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkHint {
    /// Tablets (by name) that tried the network, got no answer and came over the USB
    /// cable instead.
    pub fell_back: Vec<String>,
    /// Windows Firewall blocks PerritoDeck on the networks in use now, or has no rule
    /// letting it in (so it turns tablets away by default).
    pub firewall_blocks: bool,
    /// A network in use is one Windows calls Public.
    pub public_network: bool,
    /// The port tablets connect to.
    pub port: u16,
    /// `windows`, `macos` or `linux`: whose firewall to explain.
    pub os: String,
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

/// What a tablet said about itself when it connected, for diagnostics.
#[derive(Debug, Clone)]
struct TabletDetails {
    platform: perritodeck_protocol::Platform,
    app_version: String,
    os_version: String,
}

/// The MCP endpoint, when it runs, and what it was started with.
#[derive(Default)]
struct McpRuntime {
    server: Option<perritodeck_mcp::HttpServer>,
    port: u16,
    token: String,
}

/// State the background tasks share with [`Host`].
struct Shared {
    deck: Arc<Deck>,
    settings: Mutex<AppSettings>,
    settings_path: PathBuf,
    foreground: Mutex<Option<String>>,
    recent_apps: Mutex<VecDeque<String>>,
    events: broadcast::Sender<HostEvent>,
    /// What each tablet said about itself when it last connected.
    tablet_details: Mutex<HashMap<String, TabletDetails>>,
    /// Tablets that last came over the USB cable because this computer's network
    /// addresses didn't answer them: its firewall may be keeping tablets on Wi-Fi out.
    fell_back: Mutex<BTreeSet<String>>,
    /// Tablets switched by a "switch profile" key since the app in front last changed.
    /// Auto-switch rules leave them be until it changes again, or the key would undo
    /// itself while the rule's app is in front.
    key_switched: Mutex<HashSet<String>>,
}

impl Shared {
    /// Show each connected tablet the profile it should have right now: a matching
    /// auto-switch rule's, else its assigned one, else the default.
    fn apply_auto_switch(&self) {
        let settings = self.settings.lock().expect("settings poisoned").clone();
        let app = self.foreground.lock().expect("focus poisoned").clone();
        let loaded = self.deck.profile_ids();
        let default = self.deck.default_profile_id();
        let key_switched = self.key_switched.lock().expect("switches poisoned").clone();
        for (device, showing) in self.deck.connected_devices() {
            let wanted = settings
                .auto_switch
                .profile_for(app.as_deref(), &device)
                .filter(|_| !key_switched.contains(&device))
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

    /// A key switched `device` to `profile`: remember it like a choice made in the app.
    fn remember_key_switch(&self, device: &str, profile: &str) {
        let is_default = profile == self.deck.default_profile_id();
        let mut settings = self.settings.lock().expect("settings poisoned");
        let mut next = settings.clone();
        if is_default {
            next.device_profiles.remove(device);
        } else {
            next.device_profiles
                .insert(device.to_owned(), profile.to_owned());
        }
        if next != *settings {
            if let Err(e) = self.save_settings(&next) {
                warn!("{e}");
            }
            *settings = next;
        }
        drop(settings);
        self.key_switched
            .lock()
            .expect("switches poisoned")
            .insert(device.to_owned());
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
    integrations_error: Option<String>,
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
    mcp: tokio::sync::Mutex<McpRuntime>,
    mcp_state: Mutex<McpState>,
    mcp_bridges: mcp::BridgesSeen,
    /// What Windows Firewall does with tablets, and when that was asked.
    firewall: Mutex<Option<(std::time::Instant, Option<discovery::Firewall>)>>,
    /// A profile import waiting for the person's choice.
    import_staging: Mutex<import::Staging>,
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
            format!("could not listen on {addr} — is PerritoDeck already running somewhere else?")
        })?;
        let addr = listener.local_addr().unwrap_or(addr);

        let real_dir = real_data_dir(&dir);
        if in_package_cache(&real_dir) {
            warn!(
                "started from inside a packaged app, so Windows keeps PerritoDeck's data in {} \
                 instead of {}; a PerritoDeck started normally won't see it",
                real_dir.display(),
                dir.display()
            );
        }

        if let Some(shipped) = opts.mcp_bridge.clone() {
            let data = dir.clone();
            let installed =
                tokio::task::spawn_blocking(move || mcp::install_bridge(&shipped, &data)).await;
            let at = mcp::bridge_path(&dir);
            match installed {
                Ok(Ok(mcp::BridgeInstall::Copied)) => {
                    info!("copied the bridge for AI assistants to {}", at.display())
                }
                Ok(Ok(mcp::BridgeInstall::Unchanged)) => {}
                Ok(Ok(mcp::BridgeInstall::InUse)) => info!(
                    "an AI assistant is using {}, so it stays as it is for now",
                    at.display()
                ),
                Ok(Err(e)) => warn!(
                    "couldn't copy the bridge for AI assistants to {}: {e}",
                    at.display()
                ),
                Err(e) => warn!("couldn't copy the bridge for AI assistants: {e}"),
            }
        }

        let profile_path = opts
            .profile
            .clone()
            .unwrap_or_else(|| default_profile_path(&dir));
        // It can hold passwords, which a profile's load error would quote.
        if same_place(&profile_path, &integrations_path(&dir)) {
            bail!(
                "{} holds the integrations (OBS, Home Assistant, Companion), not a profile",
                profile_path.display()
            );
        }
        if perritodeck_profile::ensure_starter(&profile_path)? {
            info!("created a starter profile: {}", profile_path.display());
        }
        let profile = perritodeck_profile::load(&profile_path)?;
        let profiles_dir = profile_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let not_profiles = not_profiles(&profiles_dir, &dir);
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
        let (integrations_config, integrations_error) =
            match perritodeck_integrations::Config::load(&integrations_path(&dir)) {
                Ok(config) => (config, None),
                Err(e) => {
                    warn!("ignoring integrations.toml: {e}");
                    (Default::default(), Some(e))
                }
            };
        let companion_config = integrations_config.companion.clone();
        let builtins = perritodeck_integrations::Builtins::new(
            integrations_config,
            integrations_error.clone(),
        );
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
        // What plugins ask to open (a browser for a sign-in, an app) is opened here, by
        // the host, like an Open or Launch key: outside the plugin's process tree, so
        // it outlives the plugin.
        let opens = Executor::new(input.clone(), Arc::new(OpenSystem));
        let opener: perritodeck_plugins::Opener = Arc::new(move |request| {
            let opens = opens.clone();
            Box::pin(async move {
                let spec = match request {
                    perritodeck_plugins::OpenRequest::Target(target) => ActionSpec::OpenPath {
                        path: target.into(),
                    },
                    perritodeck_plugins::OpenRequest::Program { program, args } => {
                        ActionSpec::Launch { program, args }
                    }
                };
                opens.run(&spec).await.map(drop).map_err(|e| e.to_string())
            })
        });
        let plugins = perritodeck_plugins::PluginHost::start_with_opener(
            dir.join("plugins"),
            env!("CARGO_PKG_VERSION"),
            Arc::new(move |k, v| {
                if let Some(deck) = sink.get() {
                    deck.set_live([(k, v)]);
                }
            }),
            opener,
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
        watch::load_all(&profiles_dir, &deck, &not_profiles);
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
        if settings.detailed_logs {
            logging::set_detailed(true);
        }
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

        let watcher = watch::watch_profiles(profiles_dir.clone(), deck.clone(), not_profiles)
            .map_err(|e| {
                warn!(
                    "won't notice edits to {} until restart: {e}",
                    profiles_dir.display()
                )
            })
            .ok();
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
            tablet_details: Mutex::new(HashMap::new()),
            fell_back: Mutex::new(BTreeSet::new()),
            key_switched: Mutex::new(HashSet::new()),
        });
        let remember = Arc::downgrade(&shared);
        deck.on_profile_switch(Arc::new(move |device: &str, profile: &str| {
            if let Some(shared) = remember.upgrade() {
                shared.remember_key_switch(device, profile);
            }
        }));
        spawn_event_loop(shared.clone(), server_events);
        live::spawn(deck.clone(), test_executor.integrations_handle());
        {
            // Plugins hear which of their values profiles show (e.g. `up:<host>`).
            let (deck, plugins) = (deck.clone(), plugins.clone());
            tokio::spawn(async move {
                let mut every = poll_timer(Duration::from_secs(2));
                loop {
                    every.tick().await;
                    plugins.watch(&deck.live_tokens());
                    // Titles show "–" for these until the plugin reports them.
                    deck.set_declared(declared_plugin_values(&plugins.list()));
                }
            });
        }
        if opts.watch_focus && focus::supported() {
            spawn_focus_watch(shared.clone());
        }
        let usb_devices = usb::UsbDevices::default();
        // adb reverse leads the tablet to 127.0.0.1 on this computer, which a host
        // listening on one other address (the CLI's --listen) doesn't answer.
        let usb_reachable = usb_reaches(opts.listen);
        if opts.usb && !opts.insecure && !usb_reachable {
            info!(
                "tablets on USB can't connect: the host listens on {} only, and adb's \
                 tunnel arrives at 127.0.0.1",
                opts.listen
            );
        }
        let usb_available =
            opts.usb && !opts.insecure && usb_reachable && usb::find_adb().is_some();
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
            profile_path,
            profiles_dir,
            data_dir: dir,
            identity,
            store,
            handle,
            deck,
            insecure: opts.insecure,
            integrations,
            integrations_error,
            shared,
            history: Default::default(),
            executor: test_executor,
            mcp: Default::default(),
            mcp_state: Mutex::new(McpState::Off),
            mcp_bridges: Default::default(),
            firewall: Mutex::new(None),
            import_staging: Default::default(),
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
        let usb = self.usb.lock().expect("usb poisoned").clone();
        let port = self.addr.port();
        let addresses = if self.addr.ip().is_loopback() {
            vec![format!("127.0.0.1:{port}")]
        } else {
            self.lan_addrs()
                .iter()
                .map(|ip| format!("{ip}:{port}"))
                .collect()
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
            logs_dir: self.logs_dir().display().to_string(),
            profile_path: self.profile_path.display().to_string(),
            profiles_dir: self.profiles_dir.display().to_string(),
            integrations: self.integrations.clone(),
            integrations_error: self.integrations_error.clone(),
            paused: self.deck.paused(),
            auto_switch_supported: focus::supported(),
            usb_devices: usb.forwarded.iter().cloned().collect(),
            usb_problems: usb.problems(),
            usb_available: self.usb_available,
            companion: self
                .companion
                .lock()
                .ok()
                .map(|s| s.clone())
                .filter(|_| self.integrations.iter().any(|i| i == "Companion")),
        }
    }

    /// The addresses tablets on the network can reach this host at. Looked up each
    /// time rather than once at start-up: the app starts at logon, often before Wi-Fi
    /// or DHCP is up, and a laptop changes networks while it runs.
    fn lan_addrs(&self) -> Vec<IpAddr> {
        lan_addrs_for(self.addr.ip(), discovery::lan_addresses)
    }

    /// Addresses to put in pairing links.
    fn pairing_addrs(&self) -> Vec<IpAddr> {
        pairing_addrs_for(self.addr.ip(), self.lan_addrs(), self.usb_available)
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
            self.shared
                .fell_back
                .lock()
                .expect("fell back poisoned")
                .remove(device_id);
            let mut settings = self.shared.settings.lock().expect("settings poisoned");
            if settings.device_profiles.remove(device_id).is_some() {
                let _ = self.shared.save_settings(&settings);
            }
        }
        removed
    }

    /// Whether to tell people their firewall may be keeping tablets on Wi-Fi out, and
    /// why. `None` when nothing says so, or this host only takes tablets on a USB cable
    /// anyway. May take a second on Windows (it reads the firewall's rules), so don't
    /// call it on the UI thread.
    pub fn network_hint(&self) -> Option<NetworkHint> {
        self.network_hint_with(!self.addr.ip().is_loopback(), || self.firewall())
    }

    /// [`Host::network_hint`], told whether tablets can come over the network at all and
    /// how to find out what Windows Firewall does with them.
    fn network_hint_with(
        &self,
        on_network: bool,
        firewall: impl FnOnce() -> Option<discovery::Firewall>,
    ) -> Option<NetworkHint> {
        let store = self.store.as_ref()?;
        if !on_network {
            return None;
        }
        let names: BTreeMap<String, String> = store
            .devices()
            .into_iter()
            .map(|(id, d)| (id, d.name))
            .collect();
        let fell_back: Vec<String> = self
            .shared
            .fell_back
            .lock()
            .expect("fell back poisoned")
            .iter()
            .filter_map(|id| names.get(id).cloned())
            .collect();
        let fw = firewall();
        let firewall_blocks = fw.is_some_and(|f| !f.lets_in);
        (firewall_blocks || !fell_back.is_empty()).then(|| NetworkHint {
            fell_back,
            firewall_blocks,
            public_network: fw.is_some_and(|f| f.public),
            port: self.addr.port(),
            os: std::env::consts::OS.to_owned(),
        })
    }

    /// What Windows Firewall does with tablets reaching this program, asked at most every
    /// 15 seconds (someone fixing it comes back to the app soon after).
    fn firewall(&self) -> Option<discovery::Firewall> {
        let mut cached = self.firewall.lock().expect("firewall poisoned");
        if let Some((at, fw)) = *cached {
            if at.elapsed() < Duration::from_secs(15) {
                return fw;
            }
        }
        // The path Windows Firewall knows this program by: started from inside a packaged
        // app, the one it sees for itself can be a redirection.
        let fw = std::env::current_exe()
            .ok()
            .and_then(|exe| discovery::windows_firewall(&real_path(&exe), self.addr.port()));
        *cached = Some((std::time::Instant::now(), fw));
        fw
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
        // A choice made here lets the rules apply again, as before any key switched it.
        self.shared
            .key_switched
            .lock()
            .expect("switches poisoned")
            .remove(device_id);
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

    /// "Start at login" as last chosen in PerritoDeck (see [`AppSettings::start_at_login`]).
    pub fn start_at_login(&self) -> bool {
        self.shared
            .settings
            .lock()
            .expect("settings poisoned")
            .start_at_login
    }

    pub fn set_start_at_login(&self, on: bool) -> Result<(), String> {
        let mut settings = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .clone();
        if settings.start_at_login == on {
            return Ok(());
        }
        settings.start_at_login = on;
        self.shared.save_settings(&settings)?;
        *self.shared.settings.lock().expect("settings poisoned") = settings;
        Ok(())
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

    // ---- Logs and diagnostics ----

    /// Where the log files are.
    pub fn logs_dir(&self) -> PathBuf {
        logging::logs_dir(&self.data_dir)
    }

    pub fn detailed_logs(&self) -> bool {
        self.shared
            .settings
            .lock()
            .expect("settings poisoned")
            .detailed_logs
    }

    /// Log debug messages too (or stop); applies at once and is remembered.
    pub fn set_detailed_logs(&self, on: bool) -> Result<(), String> {
        let mut settings = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .clone();
        settings.detailed_logs = on;
        self.shared.save_settings(&settings)?;
        *self.shared.settings.lock().expect("settings poisoned") = settings;
        logging::set_detailed(on);
        Ok(())
    }

    /// A plain-text report for bug reports: versions, paths, tablets, plugins,
    /// integrations, network and MCP settings, and the last log lines. No keys,
    /// certificates, tokens or passwords.
    pub fn diagnostics(&self, app_version: Option<&str>) -> String {
        use diagnostics::{Report, Section};
        let settings = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .clone();
        let mut sections = vec![
            diagnostics::versions(app_version),
            diagnostics::paths(&self.data_dir, &self.profile_path),
        ];

        let details = self
            .shared
            .tablet_details
            .lock()
            .expect("details poisoned")
            .clone();
        let mut tablets = Section::new("Tablets");
        for d in self.devices() {
            let mut line = format!(
                "{} ({}): {}, profile {}",
                d.name,
                diagnostics::short_id(&d.id),
                if d.connected {
                    "connected"
                } else {
                    "not connected"
                },
                d.profile
            );
            if let Some(page) = d.connected.then(|| self.deck.device_page(&d.id)).flatten() {
                line.push_str(&format!(", page {page}"));
            }
            if let Some(t) = details.get(&d.id) {
                let platform = serde_json::to_value(t.platform)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                line.push_str(&format!(
                    ", {platform} {}, Perrito {}",
                    t.os_version, t.app_version
                ));
            }
            tablets.line(line);
        }
        sections.push(tablets);

        let state_text = |status: serde_json::Value| -> String {
            let state = status["state"].as_str().unwrap_or("unknown").to_owned();
            match (status["reason"].as_str(), status["runtime"].as_str()) {
                (Some(r), _) => format!("{state}: {r}"),
                (_, Some(r)) => format!("{state}: needs {r}"),
                _ => state,
            }
        };
        let mut plugins = Section::new("Plugins");
        for p in self.plugins.list() {
            plugins.line(format!(
                "{} {}: {}",
                p.manifest.id,
                p.manifest.version,
                state_text(serde_json::to_value(&p.status).unwrap_or_default())
            ));
        }
        for b in self.plugins.broken() {
            plugins.line(format!("broken: {}: {}", b.folder, b.problem));
        }
        sections.push(plugins);

        let mut sd = Section::new("Stream Deck plugins");
        match &self.sd_plugins {
            Some(host) => {
                for p in host.list() {
                    sd.line(format!(
                        "{} {}: {}",
                        p.uuid,
                        p.version,
                        state_text(serde_json::to_value(&p.status).unwrap_or_default())
                    ));
                }
                for b in host.broken() {
                    sd.line(format!("broken: {}: {}", b.folder, b.problem));
                }
            }
            None => {
                sd.line("off (couldn't start on this computer)");
            }
        }
        sections.push(sd);

        let mut integrations = Section::new("Integrations");
        if self.integrations_error.is_some() {
            // Settings names the mistake; the report only says the file is ignored.
            integrations.line("integrations.toml has a mistake and is ignored");
        }
        let tokens = self.deck.live_tokens();
        for (name, prefix) in [("OBS", "obs:"), ("Home Assistant", "ha:")] {
            let state = if !self.integrations.iter().any(|i| i == name) {
                "not set up".to_owned()
            } else {
                let values: Vec<Option<String>> = tokens
                    .iter()
                    .filter(|t| t.starts_with(prefix))
                    .map(|t| self.deck.live_value(t))
                    .collect();
                if values.is_empty() {
                    "set up (no key shows its values, so not checked)".to_owned()
                } else if values.iter().any(|v| v.as_deref() == Some(live::UNKNOWN)) {
                    "set up, but not reachable (its values show –)".to_owned()
                } else if values.iter().any(Option::is_some) {
                    "set up and answering".to_owned()
                } else {
                    "set up (not checked yet)".to_owned()
                }
            };
            integrations.field(name, state);
        }
        integrations.field(
            "Companion",
            self.status()
                .companion
                .unwrap_or_else(|| "not set up".to_owned()),
        );
        sections.push(integrations);

        let usb = if self.usb_available {
            self.usb.lock().expect("usb poisoned").describe()
        } else {
            "off (adb not found, or turned off)".to_owned()
        };
        let mut network = diagnostics::network(&settings, Some(&self.addr.to_string()), &usb);
        // Public or Private decides whether Windows' firewall lets tablets in.
        for category in discovery::network_categories() {
            network.field("network type", category);
        }
        sections.push(network);
        let state = self.mcp_state.lock().expect("mcp poisoned").describe();
        sections.push(diagnostics::mcp_section(
            &settings,
            &state,
            mcp::read_token(&self.data_dir).is_some(),
            &self.data_dir,
        ));
        let mut detail = Section::new("Logging");
        detail.field(
            "detailed",
            if settings.detailed_logs { "on" } else { "off" },
        );
        sections.push(detail);

        Report {
            app_version: app_version.map(str::to_owned),
            host_version: env!("CARGO_PKG_VERSION").to_owned(),
            sections,
            log: logging::tail(&self.logs_dir(), diagnostics::LOG_LINES),
        }
        .render()
    }

    // ---- AI assistants (MCP) ----

    fn set_mcp_state(&self, state: McpState) {
        *self.mcp_state.lock().expect("mcp poisoned") = state;
    }

    pub fn mcp_state(&self) -> McpState {
        self.mcp_state.lock().expect("mcp poisoned").clone()
    }

    /// Start, restart or stop the MCP endpoint to match the settings (and token). Call
    /// it once after starting the host.
    pub async fn apply_mcp(self: &Arc<Self>) {
        let wanted = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .mcp
            .clone();
        let mut rt = self.mcp.lock().await;
        if !wanted.enabled {
            if let Some(server) = rt.server.take() {
                server.shutdown().await;
                info!("AI assistants (MCP): stopped");
            }
            self.set_mcp_state(McpState::Off);
            return;
        }
        let token = match mcp::token_or_new(&self.data_dir) {
            Ok(t) => t,
            Err(message) => {
                warn!("AI assistants (MCP): {message}");
                self.set_mcp_state(McpState::Error { message });
                return;
            }
        };
        if rt.server.is_some() && rt.port == wanted.port && rt.token == token {
            return;
        }
        if let Some(server) = rt.server.take() {
            server.shutdown().await;
        }
        self.mcp_bridges.clear();
        let backend = Arc::new(mcp::HostBackend(Arc::downgrade(self)));
        let opts = perritodeck_mcp::HttpOptions::new(token.clone()).port(wanted.port);
        match perritodeck_mcp::serve_http(backend, opts).await {
            Ok(server) => {
                let url = server.url();
                info!("AI assistants (MCP) can connect at {url}");
                *rt = McpRuntime {
                    server: Some(server),
                    port: wanted.port,
                    token,
                };
                self.set_mcp_state(McpState::Running { url });
            }
            Err(e) => {
                let message = if e.kind() == std::io::ErrorKind::AddrInUse {
                    format!("port {} is in use by another program", wanted.port)
                } else {
                    e.to_string()
                };
                warn!("AI assistants (MCP) couldn't start: {message}");
                self.set_mcp_state(McpState::Error { message });
            }
        }
    }

    /// Stop every plugin before the program exits, waiting (blocking, a few seconds at
    /// most) until both PerritoDeck's own plugins and Stream Deck plugins have ended,
    /// with the programs they started. Without this, quitting the app left plugins'
    /// processes running (`node.exe`, say, when Windows refused it a job object).
    pub fn stop_plugins(&self) {
        let until = std::time::Instant::now() + Duration::from_secs(3);
        // Both kinds are told first, so they stop side by side within the one limit.
        if let Some(sd) = &self.sd_plugins {
            sd.shutdown();
        }
        self.plugins.shutdown_and_wait(Duration::from_secs(3));
        if let Some(sd) = &self.sd_plugins {
            sd.wait_stopped(until.saturating_duration_since(std::time::Instant::now()));
        }
    }

    /// Stop the MCP endpoint (it also stops when the host is dropped).
    pub async fn stop_mcp(&self) {
        if let Some(server) = self.mcp.lock().await.server.take() {
            server.shutdown().await;
        }
        self.set_mcp_state(McpState::Off);
    }

    /// The MCP settings, state and the snippets assistants need. Makes the access
    /// token if there isn't one yet.
    pub fn mcp_view(&self) -> McpView {
        let settings = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .mcp
            .clone();
        let token = mcp::token_or_new(&self.data_dir).unwrap_or_default();
        let (command, args) = mcp::bridge_command(&self.data_dir);
        McpView {
            url: mcp::url(settings.port),
            snippets: mcp::snippets(&command, &args, settings.port, &token),
            state: self.mcp_state(),
            settings,
            app_bridge_used: self.mcp_bridges.contains("perritodeck-app"),
        }
    }

    /// Save the MCP settings and start, restart or stop the endpoint to match.
    /// Permission changes apply to the next request.
    pub async fn set_mcp(self: &Arc<Self>, next: McpSettings) -> Result<McpView, String> {
        if next.port < 1024 {
            return Err("Use a port from 1024 up.".into());
        }
        if next.port == self.addr.port() {
            return Err(format!(
                "Port {} is the one tablets use; pick another.",
                next.port
            ));
        }
        let mut settings = self
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .clone();
        settings.mcp = next;
        self.shared.save_settings(&settings)?;
        *self.shared.settings.lock().expect("settings poisoned") = settings;
        self.apply_mcp().await;
        Ok(self.mcp_view())
    }

    /// Replace the access token; assistants set up with the old one stop working.
    pub async fn new_mcp_token(self: &Arc<Self>) -> Result<McpView, String> {
        mcp::new_token(&self.data_dir)?;
        info!("AI assistants (MCP): made a new access token");
        self.apply_mcp().await;
        Ok(self.mcp_view())
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

    /// Install (or update) a plugin that ships with the app. The bundled plugins share
    /// one MIT LICENSE, next to their folders; a plugin without its own gets a copy.
    pub fn install_from_catalog(&self, id: &str) -> Result<String, String> {
        let (_, folder) = self
            .catalog_entries()
            .into_iter()
            .find(|(m, _)| m.id == id)
            .ok_or_else(|| format!("{id} doesn't ship with this version."))?;
        let licence = self.plugin_catalog.as_ref().map(|d| d.join("LICENSE"));
        self.plugins.install_bundled(&folder, licence.as_deref())
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
        view.show_agent_looks(&self.deck.agent_looks(&id));
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
        for icon in icons_of(&profile) {
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

/// Where tablets on the network reach a host listening on `listen`: nowhere for
/// loopback, every usable address of this machine for 0.0.0.0, else that address.
fn lan_addrs_for(listen: IpAddr, all: impl FnOnce() -> Vec<IpAddr>) -> Vec<IpAddr> {
    if listen.is_loopback() {
        vec![]
    } else if listen.is_unspecified() {
        all()
    } else {
        vec![listen]
    }
}

/// The pairing link's addresses. With `adb` around, 127.0.0.1 goes last: a tablet on
/// a USB cable reaches the host there through `adb reverse` even when the network
/// doesn't let it through (a firewall, a Public network), and without it a computer
/// with no network at all would show a link with no address in it.
///
/// Only when listening on every address, though: `adb reverse` connects to 127.0.0.1
/// on this computer, which a host listening on one network address (`--listen
/// 192.168.1.60`) doesn't answer.
fn pairing_addrs_for(listen: IpAddr, lan: Vec<IpAddr>, usb: bool) -> Vec<IpAddr> {
    let localhost = IpAddr::V4(Ipv4Addr::LOCALHOST);
    if listen.is_loopback() {
        return vec![localhost];
    }
    let mut addrs = lan;
    if usb && listen.is_unspecified() && !addrs.contains(&localhost) {
        addrs.push(localhost);
    }
    addrs
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

/// Every image a profile's keys show, once each: what a `.perritoprofile` carries.
fn icons_of(profile: &Profile) -> Vec<PathBuf> {
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
    icons
}

/// Where an imported profile's `icon` is written, relative to the profiles folder and
/// named as in the zip. `None` for a path outside the folder, or for a file that isn't
/// a picture the app takes as an icon.
fn importable_image(icon: &Path) -> Option<String> {
    let rel = icon.to_string_lossy().replace('\\', "/");
    // No colon either: Windows joins `C:/x` or `C:x` onto the folder as a path of its own.
    let inside = !rel.starts_with('/')
        && !rel.contains(':')
        && rel.split('/').all(|p| !p.is_empty() && p != "..");
    let image = Path::new(&rel)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            ["png", "jpg", "jpeg", "gif"]
                .iter()
                .any(|i| e.eq_ignore_ascii_case(i))
        });
    (inside && image).then_some(rel)
}

/// A timer for a background loop that ticks every `period`. After a slow round, or
/// waking from sleep (Windows' clock counts the time asleep), it skips the ticks it
/// missed; tokio's default would run them all back to back, thousands after a night.
pub(crate) fn poll_timer(period: Duration) -> tokio::time::Interval {
    let mut timer = tokio::time::interval(period);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    timer
}

/// Feed server events to the deck, keep live titles ticking, and tell subscribers.
fn spawn_event_loop(
    shared: Arc<Shared>,
    mut server_events: tokio::sync::mpsc::UnboundedReceiver<ServerEvent>,
) {
    let animator = shared.deck.clone();
    tokio::spawn(async move {
        // Animated (GIF) keys; cheap when there are none.
        let mut every = poll_timer(Duration::from_millis(40));
        loop {
            every.tick().await;
            animator.animate();
        }
    });
    let ticker = shared.deck.clone();
    tokio::spawn(async move {
        let mut tick = poll_timer(Duration::from_secs(1));
        loop {
            tick.tick().await;
            ticker.tick();
        }
    });
    let sliders = shared.deck.clone();
    tokio::spawn(async move {
        // Keep sliders in step with changes made on the computer, and read them as soon
        // as a tablet shows them: its layout carried the last level known.
        let mut every = poll_timer(Duration::from_secs(3));
        loop {
            tokio::select! {
                _ = every.tick() => {}
                _ = sliders.sliders_shown() => {}
            }
            sliders.poll_sliders().await;
        }
    });
    let watched = shared.clone();
    tokio::spawn(async move {
        let mut seen = watched.deck.revision();
        let mut every = poll_timer(Duration::from_millis(200));
        loop {
            every.tick().await;
            let now = watched.deck.revision();
            if now != seen {
                seen = now;
                let _ = watched.events.send(HostEvent::ProfileChanged);
            }
            // Asked on this timer, so a plugin that redraws all the time costs the editor
            // at most five refreshes a second.
            for profile in watched.deck.take_looks_changed() {
                let _ = watched.events.send(HostEvent::KeyLooksChanged { profile });
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
                    peer,
                    grid,
                    screen,
                    capabilities,
                    platform,
                    app_version,
                    os_version,
                    network_failed,
                } => {
                    // The tablet says so until it gets through over the network again.
                    let over_cable = peer.ip().to_canonical().is_loopback();
                    let mut fell_back = shared.fell_back.lock().expect("fell back poisoned");
                    if over_cable && network_failed {
                        info!(device = %device_id, "came over the USB cable: the network didn't answer it");
                        fell_back.insert(device_id.clone());
                    } else {
                        fell_back.remove(&device_id);
                    }
                    drop(fell_back);
                    shared
                        .tablet_details
                        .lock()
                        .expect("details poisoned")
                        .insert(
                            device_id.clone(),
                            TabletDetails {
                                platform,
                                app_version,
                                os_version,
                            },
                        );
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
        let mut every = poll_timer(Duration::from_millis(700));
        let mut watch = focus::Watch::default();
        loop {
            every.tick().await;
            let last = watch.last_seen().cloned();
            let Ok((front, left)) = tokio::task::spawn_blocking(move || {
                let front = focus::foreground();
                let left = front.is_none() && last.is_some_and(|l| l.has_left());
                (front, left)
            })
            .await
            else {
                continue;
            };
            // Clicking over to PerritoDeck itself shouldn't switch the tablets away.
            if front.as_ref().is_some_and(|f| is_us(&f.app)) {
                continue;
            }
            let Some(app) = watch.change(front, left) else {
                continue;
            };
            if let Some(a) = &app {
                let mut recent = shared.recent_apps.lock().expect("apps poisoned");
                recent.retain(|r| !r.eq_ignore_ascii_case(a));
                recent.push_front(a.clone());
                recent.truncate(RECENT_APPS);
            }
            *shared.foreground.lock().expect("focus poisoned") = app.clone();
            // A new app in front: the rules apply again to tablets a key switched.
            shared
                .key_switched
                .lock()
                .expect("switches poisoned")
                .clear();
            shared.apply_auto_switch();
            let _ = shared.events.send(HostEvent::ForegroundApp { app });
        }
    });
}

#[cfg(test)]
mod tests;
