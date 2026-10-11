//! `perritodeck-host` — the desktop host (headless until the Phase 3 app).
//!
//! Serves a demo layout to paired tablets over TLS: key 1 is a live clock, key 2
//! counts presses, and every press is logged. Type `pair` (or start with `--pair`)
//! to let a new tablet pair.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use perritodeck_actions::{
    system::{EnigoInput, OpenSystem},
    Executor,
};
use perritodeck_core::{
    discovery, display_fingerprint,
    pairing::{PairingStore, DEFAULT_OFFER_TTL},
    HostIdentity, Security, Server, ServerConfig, ServerEvent, ServerHandle,
};
use perritodeck_deck::{watch::watch_profile, Deck, Outbox};
use perritodeck_protocol::{Message, DEFAULT_PORT};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpListener,
    sync::mpsc::UnboundedReceiver,
};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "PerritoDeck desktop host")]
struct Cli {
    /// Address to listen on. The default accepts paired tablets on your network.
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::UNSPECIFIED))]
    listen: IpAddr,
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    #[arg(long, default_value_t = 2000, value_parser = clap::value_parser!(u64).range(250..=60_000))]
    heartbeat_ms: u64,
    /// Where the host keeps its certificate and paired devices.
    #[arg(long, env = "PERRITODECK_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Profile to show on the tablets (created with examples if missing).
    #[arg(long, env = "PERRITODECK_PROFILE")]
    profile: Option<PathBuf>,
    /// Open a pairing window as soon as the host starts.
    #[arg(long)]
    pair: bool,
    /// Development only: plain ws:// with no pairing, loopback only.
    #[arg(long)]
    insecure: bool,
    /// Don't advertise on the local network (mDNS).
    #[arg(long)]
    no_mdns: bool,
    /// Testing only: also accept this fixed pairing code for an hour (repeatable).
    #[arg(long, hide = true)]
    pair_code: Vec<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server (the default).
    Serve,
    /// List paired devices.
    Devices,
    /// Unpair a device. In a running host, type `revoke <id>` instead to also disconnect it.
    Revoke { device_id: String },
    /// Show where the host keeps its data.
    Paths,
    /// Run one action now, for testing, e.g.
    /// `perritodeck-host action '{"type":"hotkey","keys":"Ctrl+Alt+T"}'`.
    /// Waits 3 seconds first so you can focus the target window.
    Action {
        /// Action as JSON (see host/crates/actions).
        json: String,
        #[arg(long, default_value_t = 3)]
        delay_secs: u64,
    },
}

fn data_dir(cli: &Cli) -> Result<PathBuf> {
    if let Some(d) = &cli.data_dir {
        return Ok(d.clone());
    }
    directories::ProjectDirs::from("dev", "perritodeck", "PerritoDeck")
        .map(|d| d.config_dir().to_owned())
        .context("could not find a settings folder for this user; pass --data-dir")
}

fn devices_path(dir: &Path) -> PathBuf {
    dir.join("devices.json")
}

fn profile_path(cli: &Cli, dir: &Path) -> PathBuf {
    cli.profile
        .clone()
        .unwrap_or_else(|| dir.join("profiles").join("default.toml"))
}

async fn run_action(json: &str, delay_secs: u64) -> Result<()> {
    use perritodeck_actions::{
        system::{EnigoInput, OpenSystem},
        ActionSpec, Executor, Outcome,
    };
    let spec: ActionSpec = serde_json::from_str(json).context("not a valid action")?;
    Executor::validate(&spec)?;
    if delay_secs > 0 {
        println!("running in {delay_secs}s — switch to the target window…");
        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
    }
    let ex = Executor::new(Arc::new(EnigoInput::new()), Arc::new(OpenSystem));
    match ex.run(&spec).await? {
        Outcome::Ok => println!("ok"),
        Outcome::Alert(a) => println!("alert: {a}"),
    }
    Ok(())
}

fn print_devices(store: &PairingStore) {
    let devices = store.devices();
    if devices.is_empty() {
        println!("no paired devices");
    }
    for (id, d) in devices {
        let seen = d
            .last_seen
            .map(|s| format!("last seen {}", ago(s)))
            .unwrap_or_else(|| "never connected".into());
        println!("  {id}  {}  ({seen})", d.name);
    }
}

fn ago(unix: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(unix);
    let s = now.saturating_sub(unix);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86_400),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let dir = data_dir(&cli)?;
    match &cli.command {
        Some(Command::Action { json, delay_secs }) => return run_action(json, *delay_secs).await,
        Some(Command::Devices) => {
            print_devices(&PairingStore::open(&devices_path(&dir))?);
            return Ok(());
        }
        Some(Command::Revoke { device_id }) => {
            let store = PairingStore::open(&devices_path(&dir))?;
            if !store.revoke(device_id) {
                bail!("no paired device with id {device_id}");
            }
            println!("unpaired {device_id}");
            return Ok(());
        }
        Some(Command::Paths) => {
            let (cert, key) = HostIdentity::paths(&dir);
            println!("data folder:  {}", dir.display());
            println!("certificate:  {}", cert.display());
            println!("private key:  {}", key.display());
            println!("devices:      {}", devices_path(&dir).display());
            println!("profile:      {}", profile_path(&cli, &dir).display());
            return Ok(());
        }
        Some(Command::Serve) | None => {}
    }

    if cli.insecure && !cli.listen.is_loopback() {
        bail!("--insecure only works on loopback; add --listen 127.0.0.1");
    }

    let host_name = gethostname::gethostname().to_string_lossy().into_owned();
    let (security, store, identity) = if cli.insecure {
        warn!("insecure mode: no encryption, no pairing — development only");
        (Security::Insecure, None, None)
    } else {
        let identity = HostIdentity::load_or_create(&dir)
            .with_context(|| format!("loading the host certificate from {}", dir.display()))?;
        let store = Arc::new(PairingStore::open(&devices_path(&dir))?);
        for code in &cli.pair_code {
            if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
                bail!("--pair-code must be 6 digits");
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

    let addr = SocketAddr::new(cli.listen, cli.port);
    let listener = TcpListener::bind(addr).await.with_context(|| {
        format!("could not listen on {addr} — is another host already running?")
    })?;

    let profile_file = profile_path(&cli, &dir);
    if perritodeck_profile::ensure_starter(&profile_file)? {
        println!("  created a starter profile: {}", profile_file.display());
    }
    let profile = perritodeck_profile::load(&profile_file)?;
    let missing = profile.missing_icons(profile_file.parent().unwrap_or(Path::new(".")));
    for m in missing {
        warn!("icon not found: {}", m.display());
    }
    let base_dir = profile_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    // The deck sends through the server's handle, which exists only once the server does;
    // a small shim lets it be wired up after.
    let outbox = Arc::new(LateOutbox::default());
    let input = Arc::new(EnigoInput::new());
    if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some() {
        // Ask for keyboard access now, so the desktop's permission prompt appears at
        // startup rather than on the first key press.
        let input = input.clone();
        tokio::task::spawn_blocking(move || {
            match input.connect_now() {
            Ok(()) => info!("keyboard access granted"),
            Err(e) => warn!("hotkeys and typing won't work until the host is restarted and access is allowed: {e}"),
        }
        });
    }
    let executor = Executor::new(input, Arc::new(OpenSystem));
    let deck = Deck::new(profile, base_dir, outbox.clone(), executor);
    let layout_deck = deck.clone();

    let mut config = ServerConfig::new(
        host_name.clone(),
        security,
        Arc::new(move |hello: &perritodeck_protocol::Hello| layout_deck.initial_layout(hello)),
    );
    config.heartbeat_interval = Duration::from_millis(cli.heartbeat_ms);
    let (server, events) = Server::new(listener, config);
    let handle = server.handle();
    outbox.set(handle.clone());
    let _watcher = match watch_profile(profile_file.clone(), deck.clone()) {
        Ok(w) => Some(w),
        Err(e) => {
            warn!(
                "won't notice edits to {} until restart: {e}",
                profile_file.display()
            );
            None
        }
    };

    let lan = if cli.listen.is_loopback() {
        vec![]
    } else {
        discovery::lan_addresses()
    };
    let _mdns = match (&identity, cli.no_mdns || cli.listen.is_loopback()) {
        (Some(id), false) => match discovery::advertise(&host_name, id.fingerprint(), cli.port) {
            Ok(a) => Some(a),
            Err(e) => {
                warn!("could not advertise on the network (tablets can still connect by address): {e}");
                None
            }
        },
        _ => None,
    };

    print_banner(&host_name, addr, &lan, identity.as_ref(), cli.insecure);
    if let (Some(store), Some(identity)) = (&store, identity) {
        let pairing = PairingUi {
            host_name: host_name.clone(),
            addrs: if cli.listen.is_loopback() {
                vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]
            } else {
                lan.clone()
            },
            port: cli.port,
            identity,
        };
        if cli.pair || store.devices().is_empty() {
            pairing.open(store);
        }
        spawn_console(store.clone(), handle.clone(), pairing);
    }

    spawn_deck(deck, events);

    tokio::select! {
        r = server.run() => r.context("server stopped")?,
        _ = tokio::signal::ctrl_c() => info!("shutting down"),
    }
    Ok(())
}

/// Outbox that forwards to the server handle once it exists.
#[derive(Default)]
struct LateOutbox(std::sync::OnceLock<ServerHandle>);

impl LateOutbox {
    fn set(&self, h: ServerHandle) {
        let _ = self.0.set(h);
    }
}

impl Outbox for LateOutbox {
    fn send(&self, device_id: &str, msg: Message) -> bool {
        self.0.get().is_some_and(|h| h.send(device_id, msg))
    }
}

struct PairingUi {
    host_name: String,
    addrs: Vec<IpAddr>,
    port: u16,
    identity: HostIdentity,
}

impl PairingUi {
    fn open(&self, store: &PairingStore) {
        let fp = self.identity.fingerprint();
        let code = store.new_offer(DEFAULT_OFFER_TTL);
        let uri = discovery::pairing_uri(&self.host_name, &self.addrs, self.port, fp, &code);
        println!();
        if let Ok(qr) = qrcode::QrCode::new(uri.as_bytes()) {
            println!(
                "{}",
                qr.render::<qrcode::render::unicode::Dense1x2>()
                    .quiet_zone(true)
                    .build()
            );
        }
        println!("  Pair a tablet: scan this with the tablet's camera app, or open Perrito");
        println!(
            "  and enter this code within {} minutes:",
            DEFAULT_OFFER_TTL.as_secs() / 60
        );
        println!();
        println!("      {} {}", &code[..3], &code[3..]);
        println!();
        println!(
            "  The tablet should show this computer's fingerprint: {}",
            display_fingerprint(fp)
        );
        println!();
    }
}

fn spawn_console(store: Arc<PairingStore>, handle: ServerHandle, pairing: PairingUi) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut words = line.split_whitespace();
            match (words.next(), words.next()) {
                (Some("pair"), _) => pairing.open(&store),
                (Some("devices"), _) => {
                    print_devices(&store);
                    let online = handle.devices();
                    if !online.is_empty() {
                        println!("  connected now: {}", online.join(", "));
                    }
                }
                (Some("revoke"), Some(id)) => {
                    if store.revoke(id) {
                        handle.kick(id);
                        println!("unpaired and disconnected {id}");
                    } else {
                        println!("no paired device with id {id}");
                    }
                }
                (None, _) => {}
                _ => println!("commands: pair, devices, revoke <device id>; Ctrl+C quits"),
            }
        }
    });
}

/// Feed server events to the deck runtime and keep live titles ticking.
fn spawn_deck(deck: Arc<Deck>, mut events: UnboundedReceiver<ServerEvent>) {
    let ticker = deck.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            ticker.tick();
        }
    });

    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                ServerEvent::Connected {
                    device_id,
                    grid,
                    screen,
                    capabilities,
                    ..
                } => {
                    deck.connected(&device_id, grid, screen, &capabilities);
                }
                ServerEvent::KeyDown {
                    device_id,
                    key_index,
                } => {
                    info!(device = %device_id, "key {} pressed", key_index + 1);
                    deck.key_down(&device_id, key_index);
                }
                ServerEvent::Paired { device_name, .. } => {
                    println!("\n  Paired with {device_name}. It will connect automatically from now on.\n");
                }
                ServerEvent::Disconnected { device_id, .. } => deck.disconnected(&device_id),
                _ => {}
            }
        }
    });
}

fn print_banner(
    host_name: &str,
    addr: SocketAddr,
    lan: &[IpAddr],
    id: Option<&HostIdentity>,
    insecure: bool,
) {
    let port = addr.port();
    println!();
    println!(
        "  PerritoDeck host v{} on {host_name}",
        env!("CARGO_PKG_VERSION")
    );
    if insecure {
        println!("  INSECURE development mode: ws://{addr}  (emulator: 10.0.2.2:{port})");
        println!();
        return;
    }
    if let Some(id) = id {
        println!("  fingerprint  {}", display_fingerprint(id.fingerprint()));
    }
    if addr.ip().is_loopback() {
        println!("  this computer only: emulator 10.0.2.2:{port}; USB after `adb reverse tcp:{port} tcp:{port}`");
    } else {
        let list: Vec<String> = lan.iter().map(|ip| format!("{ip}:{port}")).collect();
        let shown = if list.is_empty() {
            addr.to_string()
        } else {
            list.join(", ")
        };
        println!("  listening on {shown}");
    }
    println!("  type `pair` to pair a tablet, `devices` to list them");
    println!();
}
