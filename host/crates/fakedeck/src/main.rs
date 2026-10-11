//! `perritodeck-fakedeck` — pretend to be a tablet.
//!
//!   perritodeck-fakedeck pair 123456     # pair using the code the host shows
//!   perritodeck-fakedeck                 # connect, draw the grid, print live updates
//!   perritodeck-fakedeck press 1 1 1     # press key 1 three times, print round trips
//!   perritodeck-fakedeck bench -n 500    # latency statistics

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use perritodeck_fakedeck::{fake_hello, now_ms, Client, ClientSecurity, Endpoint};
use perritodeck_protocol::{Decoded, Grid, Heartbeat, Layout, Message, DEFAULT_PORT};
use serde::{Deserialize, Serialize};

#[derive(Parser)]
#[command(
    version,
    about = "A command-line PerritoDeck client for testing the host without a tablet"
)]
struct Cli {
    /// Host address as host:port.
    #[arg(long, default_value_t = format!("127.0.0.1:{DEFAULT_PORT}"))]
    host: String,
    /// Plain ws:// with no pairing (host started with --insecure).
    #[arg(long)]
    insecure: bool,
    /// Where pairing secrets are kept.
    #[arg(long)]
    state: Option<PathBuf>,
    /// Stable device id to present.
    #[arg(long, default_value = "fakedeck")]
    device_id: String,
    /// Ask for a grid, e.g. 4x2.
    #[arg(long, value_parser = parse_grid)]
    grid: Option<Grid>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Pair with the host using the 6-digit code it shows.
    Pair { code: String },
    /// Forget the pairing with this host.
    Forget,
    /// Draw the grid and print every message the host sends (default).
    Watch,
    /// Press and release keys in order, printing each round trip.
    Press { keys: Vec<u32> },
    /// Press key 0 repeatedly and report round-trip latency.
    Bench {
        #[arg(short = 'n', long, default_value_t = 200)]
        count: u32,
    },
}

fn parse_grid(s: &str) -> Result<Grid, String> {
    let (c, r) = s
        .split_once(['x', 'X'])
        .ok_or("expected COLSxROWS, e.g. 4x2")?;
    let g = Grid {
        cols: c.trim().parse().map_err(|_| "bad column count")?,
        rows: r.trim().parse().map_err(|_| "bad row count")?,
    };
    if g.is_valid() {
        Ok(g)
    } else {
        Err(format!(
            "grid must be between 1x1 and {0}x{0}",
            Grid::MAX_DIMENSION
        ))
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pairing {
    fingerprint: String,
    device_secret: String,
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    hosts: BTreeMap<String, Pairing>,
}

fn state_path(cli: &Cli) -> PathBuf {
    cli.state.clone().unwrap_or_else(|| {
        directories::ProjectDirs::from("dev", "perritodeck", "PerritoDeck")
            .map(|d| d.config_dir().join("fakedeck.json"))
            .unwrap_or_else(|| PathBuf::from("fakedeck.json"))
    })
}

fn load_state(path: &Path) -> Result<State> {
    match std::fs::read(path) {
        Ok(b) => Ok(
            serde_json::from_slice(&b).with_context(|| format!("reading {}", path.display()))?
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(e) => Err(e.into()),
    }
}

fn save_state(path: &Path, state: &State) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(state)?)?;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = state_path(&cli);
    let mut state = load_state(&path)?;

    match &cli.command {
        Some(Command::Pair { code }) => {
            let ep = Endpoint {
                addr: cli.host.clone(),
                security: ClientSecurity::TrustOnFirstUse,
            };
            let paired = Client::pair(&ep, code.trim(), &cli.device_id, "fakedeck").await?;
            println!(
                "paired with {} — host fingerprint {}",
                paired.host_name,
                perritodeck_fakedeck_display(&paired.host_id)
            );
            state.hosts.insert(
                cli.host.clone(),
                Pairing {
                    fingerprint: paired.host_id,
                    device_secret: paired.device_secret,
                },
            );
            save_state(&path, &state)?;
            println!("saved to {}", path.display());
            return Ok(());
        }
        Some(Command::Forget) => {
            state.hosts.remove(&cli.host);
            save_state(&path, &state)?;
            println!("forgot {}", cli.host);
            return Ok(());
        }
        _ => {}
    }

    let (security, secret) = if cli.insecure {
        (ClientSecurity::Insecure, None)
    } else {
        let p = state.hosts.get(&cli.host).with_context(|| {
            format!("not paired with {} yet — run `perritodeck-fakedeck pair <code>` first (or --insecure)", cli.host)
        })?;
        (
            ClientSecurity::Pinned(p.fingerprint.clone()),
            Some(p.device_secret.clone()),
        )
    };
    let ep = Endpoint {
        addr: cli.host.clone(),
        security,
    };
    let hello = fake_hello(&cli.device_id, secret, cli.grid);
    let (mut client, welcome, layout) = Client::handshake(&ep, hello).await?;
    println!(
        "connected to {} (host v{}, protocol v{})",
        welcome.host_name, welcome.host_version, welcome.protocol_version
    );
    let heartbeat = Duration::from_millis(welcome.heartbeat_interval_ms.max(250) as u64);

    match cli.command.unwrap_or(Command::Watch) {
        Command::Pair { .. } | Command::Forget => unreachable!("handled above"),
        Command::Watch => watch(client, layout, heartbeat).await,
        Command::Press { keys } => {
            for k in keys {
                let rtt_down = round_trip(&mut client, k, true).await?;
                let rtt_up = round_trip(&mut client, k, false).await?;
                println!("key {k}: down {rtt_down:.2} ms, up {rtt_up:.2} ms");
            }
            client.close().await;
            Ok(())
        }
        Command::Bench { count } => {
            let mut samples = Vec::with_capacity(count as usize);
            for _ in 0..count {
                samples.push(round_trip(&mut client, 0, true).await?);
                round_trip(&mut client, 0, false).await?;
            }
            samples.sort_by(f64::total_cmp);
            let pct = |p: f64| samples[((samples.len() - 1) as f64 * p).round() as usize];
            println!(
                "{count} presses — round trip median {:.2} ms, p95 {:.2} ms, max {:.2} ms",
                pct(0.5),
                pct(0.95),
                samples[samples.len() - 1]
            );
            client.close().await;
            Ok(())
        }
    }
}

/// Send keyDown/keyUp and wait for its keyAck. Returns milliseconds.
async fn round_trip(client: &mut Client, key: u32, down: bool) -> Result<f64> {
    let start = Instant::now();
    let id = if down {
        client.key_down(key).await?
    } else {
        client.key_up(key).await?
    };
    loop {
        let env = client
            .recv_msg(Duration::from_secs(5))
            .await
            .context("waiting for keyAck")?;
        match env.msg {
            Message::KeyAck(_) if env.id == Some(id) => {
                return Ok(start.elapsed().as_secs_f64() * 1000.0)
            }
            Message::Error(e) => bail!("host error ({:?}): {}", e.code, e.message),
            _ => continue, // setTitle etc. may arrive in between
        }
    }
}

async fn watch(mut client: Client, mut layout: Layout, heartbeat: Duration) -> Result<()> {
    draw(&layout);
    let mut ticker = tokio::time::interval(heartbeat);
    let mut seq = 0u32;
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                seq = seq.wrapping_add(1);
                client.send(Message::Heartbeat(Heartbeat { seq, sent_at_ms: now_ms() })).await?;
            }
            frame = client.recv() => match frame? {
                None => { println!("host closed the connection"); return Ok(()); }
                Some(Decoded::UnknownType { type_name, .. }) => println!("(ignored unknown message `{type_name}`)"),
                Some(Decoded::Message(env)) => match env.msg {
                    Message::HeartbeatAck(_) | Message::KeyAck(_) => {}
                    Message::SetTitle(t) => {
                        if let Some(k) = layout.keys.iter_mut().find(|k| k.index == t.key_index) {
                            k.title = t.title.clone();
                        }
                        println!("setTitle key {} → {:?}", t.key_index, t.title);
                    }
                    Message::Layout(l) => { layout = l; draw(&layout); }
                    Message::Error(e) => println!("host error ({:?}): {}", e.code, e.message),
                    other => println!("{} {:?}", other.type_name(), other),
                },
            }
        }
    }
}

fn draw(layout: &Layout) {
    const W: usize = 10;
    println!("layout {}×{}", layout.cols, layout.rows);
    for row in 0..layout.rows {
        let cells: Vec<String> = (0..layout.cols)
            .map(|col| {
                let idx = row * layout.cols + col;
                let title = layout
                    .keys
                    .iter()
                    .find(|k| k.index == idx)
                    .map(|k| k.title.replace('\n', " "))
                    .unwrap_or_default();
                let short: String = title.chars().take(W).collect();
                format!("{short:^W$}")
            })
            .collect();
        println!("|{}|", cells.join("|"));
    }
}

/// Same short form the host prints: first 6 bytes of the fingerprint as hex.
fn perritodeck_fakedeck_display(fp: &str) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let bytes = URL_SAFE_NO_PAD.decode(fp).unwrap_or_default();
    let hex: String = bytes.iter().take(6).map(|b| format!("{b:02X}")).collect();
    hex.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}
