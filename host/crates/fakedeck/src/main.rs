//! `perritodeck-fakedeck` — pretend to be a tablet.
//!
//!   perritodeck-fakedeck                 # connect, draw the grid, print live updates
//!   perritodeck-fakedeck press 1 1 1     # press key 1 three times, print round trips
//!   perritodeck-fakedeck bench -n 500    # latency statistics

use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use perritodeck_fakedeck::{fake_hello, now_ms, Client};
use perritodeck_protocol::{Decoded, Grid, Heartbeat, Layout, Message, DEFAULT_PORT};

#[derive(Parser)]
#[command(
    version,
    about = "A command-line PerritoDeck client for testing the host without a tablet"
)]
struct Cli {
    /// Host WebSocket URL.
    #[arg(long, default_value_t = format!("ws://127.0.0.1:{DEFAULT_PORT}"))]
    url: String,
    /// Session token printed by the host (needed when it listens on the LAN).
    #[arg(long, env = "PERRITODECK_TOKEN")]
    token: Option<String>,
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

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let hello = fake_hello(&cli.device_id, cli.token.clone(), cli.grid);
    let (mut client, welcome, layout) = Client::handshake(&cli.url, hello).await?;
    println!(
        "connected to {} (host v{}, protocol v{})",
        welcome.host_name, welcome.host_version, welcome.protocol_version
    );
    let heartbeat = Duration::from_millis(welcome.heartbeat_interval_ms.max(250) as u64);

    match cli.command.unwrap_or(Command::Watch) {
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
