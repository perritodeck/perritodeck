//! `perritodeck-host` — the desktop host, headless for Phase 1.
//!
//! Serves a demo layout to every tablet that connects: key 1 is a live clock,
//! key 2 counts presses, and every press is logged with its processing time.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::Parser;
use perritodeck_core::{demo, generate_token, Server, ServerConfig, ServerEvent};
use perritodeck_protocol::{Message, SetTitle, DEFAULT_PORT};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "PerritoDeck desktop host")]
struct Cli {
    /// Address to listen on. The default (127.0.0.1) serves the Android emulator and USB
    /// (adb reverse). Use 0.0.0.0 to accept tablets on your LAN; a session token is then required.
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
    listen: IpAddr,
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    /// Use this session token instead of a random one (implies token checking even on loopback).
    #[arg(long, env = "PERRITODECK_TOKEN")]
    token: Option<String>,
    #[arg(long, default_value_t = 2000, value_parser = clap::value_parser!(u64).range(250..=60_000))]
    heartbeat_ms: u64,
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
    let token = match (&cli.token, cli.listen.is_loopback()) {
        (Some(t), _) => Some(t.clone()),
        (None, false) => Some(generate_token()),
        (None, true) => None,
    };

    let addr = SocketAddr::new(cli.listen, cli.port);
    let listener = TcpListener::bind(addr).await.with_context(|| {
        format!("could not listen on {addr} — is another host already running?")
    })?;

    let host_name = gethostname::gethostname().to_string_lossy().into_owned();
    let mut config = ServerConfig::new(host_name.clone(), Arc::new(demo::layout_for));
    config.token = token.clone();
    config.heartbeat_interval = Duration::from_millis(cli.heartbeat_ms);

    let (server, mut events) = Server::new(listener, config);
    let handle = server.handle();
    print_banner(&host_name, addr, token.as_deref());

    // Live clock on key 1 of every connected device.
    let clock = handle.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            clock.broadcast(Message::SetTitle(SetTitle {
                key_index: demo::CLOCK_KEY,
                title: clock_text(),
            }));
        }
    });

    // Press counter on key 2, per device.
    let counters = handle.clone();
    tokio::spawn(async move {
        let mut counts: HashMap<String, u64> = HashMap::new();
        while let Some(event) = events.recv().await {
            match event {
                ServerEvent::KeyDown {
                    device_id,
                    key_index,
                } => {
                    info!(device = %device_id, "key {} down", key_index + 1);
                    if key_index == demo::COUNTER_KEY {
                        let n = counts.entry(device_id.clone()).or_default();
                        *n += 1;
                        counters.send(
                            &device_id,
                            Message::SetTitle(SetTitle {
                                key_index,
                                title: demo::counter_title(*n),
                            }),
                        );
                    }
                }
                ServerEvent::KeyUp {
                    device_id,
                    key_index,
                } => {
                    info!(device = %device_id, "key {} up", key_index + 1);
                }
                ServerEvent::Disconnected { device_id, .. } => {
                    counts.remove(&device_id);
                }
                _ => {}
            }
        }
    });

    tokio::select! {
        r = server.run() => r.context("server stopped")?,
        _ = tokio::signal::ctrl_c() => info!("shutting down"),
    }
    Ok(())
}

fn print_banner(host_name: &str, addr: SocketAddr, token: Option<&str>) {
    let port = addr.port();
    println!();
    println!(
        "  PerritoDeck host v{} on {host_name}",
        env!("CARGO_PKG_VERSION")
    );
    println!("  listening on ws://{addr}");
    if addr.ip().is_loopback() {
        println!("  Android emulator:  ws://10.0.2.2:{port}");
        println!(
            "  USB tablet:        adb reverse tcp:{port} tcp:{port}   then  ws://127.0.0.1:{port}"
        );
        println!("  LAN tablets:       restart with --listen 0.0.0.0");
    } else {
        let lan = lan_ip()
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "<this computer's IP>".into());
        println!("  LAN tablets:       ws://{lan}:{port}");
    }
    if let Some(t) = token {
        println!("  session token:     {t}");
    }
    println!();
}

/// The address this machine would use to reach the internet (no packets are sent).
fn lan_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

/// Local wall-clock time for the demo clock key.
fn clock_text() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}
