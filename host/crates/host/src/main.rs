//! `perritodeck-host` — the host on the command line (the desktop app shares the same
//! runtime, see `lib.rs`).
//!
//! Serves the profile to paired tablets over TLS. Type `pair` (or start with `--pair`)
//! to let a new tablet pair.

use std::{
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use perritodeck_core::{pairing::PairingStore, HostIdentity};
use perritodeck_host::{
    default_profile_path, devices_path, integrations_path, Host, HostEvent, HostOptions,
};
use perritodeck_protocol::DEFAULT_PORT;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::info;
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
    /// Don't connect tablets on USB through adb.
    #[arg(long)]
    no_usb: bool,
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
    match &cli.data_dir {
        Some(d) => Ok(d.clone()),
        None => perritodeck_host::default_data_dir().context("pass --data-dir"),
    }
}

fn profile_path(cli: &Cli, dir: &Path) -> PathBuf {
    cli.profile
        .clone()
        .unwrap_or_else(|| default_profile_path(dir))
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
            println!("integrations: {}", integrations_path(&dir).display());
            return Ok(());
        }
        Some(Command::Serve) | None => {}
    }

    let mut opts = HostOptions::new(dir);
    opts.listen = cli.listen;
    opts.port = cli.port;
    opts.heartbeat = Duration::from_millis(cli.heartbeat_ms);
    opts.profile = cli.profile.clone();
    opts.insecure = cli.insecure;
    opts.mdns = !cli.no_mdns;
    opts.usb = !cli.no_usb;
    opts.pair_codes = cli.pair_code.clone();
    let host = Arc::new(Host::start(opts).await?);

    print_banner(&host);
    if host.store.is_some() {
        if cli.pair || host.devices().is_empty() {
            print_pairing(&host);
        }
        spawn_console(host.clone());
    }
    let mut events = host.subscribe();
    tokio::spawn(async move {
        while let Ok(e) = events.recv().await {
            if let HostEvent::Paired { device_name, .. } = e {
                println!(
                    "\n  Paired with {device_name}. It will connect automatically from now on.\n"
                );
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    info!("shutting down");
    Ok(())
}

fn print_pairing(host: &Host) {
    let Some(offer) = host.new_pairing_offer() else {
        return;
    };
    println!();
    if let Ok(qr) = qrcode::QrCode::new(offer.uri.as_bytes()) {
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
        offer.expires_in_secs / 60
    );
    println!();
    println!("      {} {}", &offer.code[..3], &offer.code[3..]);
    println!();
    println!(
        "  The tablet should show this computer's fingerprint: {}",
        offer.fingerprint
    );
    println!();
}

fn spawn_console(host: Arc<Host>) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut words = line.split_whitespace();
            match (words.next(), words.next()) {
                (Some("pair"), _) => print_pairing(&host),
                (Some("devices"), _) => {
                    let devices = host.devices();
                    if devices.is_empty() {
                        println!("no paired devices");
                    }
                    for d in devices {
                        let seen = if d.connected {
                            "connected now".to_owned()
                        } else {
                            d.last_seen
                                .map(|s| format!("last seen {}", ago(s)))
                                .unwrap_or_else(|| "never connected".into())
                        };
                        println!("  {}  {}  ({seen})", d.id, d.name);
                    }
                }
                (Some("revoke"), Some(id)) => {
                    if host.revoke(id) {
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

fn print_banner(host: &Host) {
    let status = host.status();
    let port = host.addr.port();
    println!();
    println!(
        "  PerritoDeck host v{} on {}",
        status.version, status.host_name
    );
    if status.insecure {
        println!(
            "  INSECURE development mode: ws://{}  (emulator: 10.0.2.2:{port})",
            host.addr
        );
        println!();
        return;
    }
    if let Some(fp) = &status.fingerprint {
        println!("  fingerprint  {fp}");
    }
    if host.addr.ip().is_loopback() {
        println!("  this computer only: emulator 10.0.2.2:{port}; USB after `adb reverse tcp:{port} tcp:{port}`");
    } else if status.addresses.is_empty() {
        println!("  listening on {}", host.addr);
    } else {
        println!("  listening on {}", status.addresses.join(", "));
    }
    println!("  type `pair` to pair a tablet, `devices` to list them");
    println!();
}
