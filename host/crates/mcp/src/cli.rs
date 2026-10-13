//! `perritodeck-mcp`, the standalone stdio bridge assistants start.
//!
//! The app and the host can be the bridge too (`perritodeck-app mcp`,
//! `perritodeck-host mcp`), but then the assistant keeps their program running, and on
//! Windows a running program can't be replaced: the app couldn't be rebuilt or updated
//! while an assistant was connected. So the app ships this small program and copies it
//! into the data folder (`<data dir>/bin`), and assistants start that copy.
//!
//! It only relays (see [`crate::bridge_stdio_to_http`]); the port and token come from the
//! same files the host uses, read afresh whenever they stop working. Its only output on
//! stdout is the protocol; problems go to stderr.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::{Endpoint, AGENT, BRIDGE_VERSION, DEFAULT_PORT, ENDPOINT_PATH};

/// The host's settings file in the data folder; the bridge reads `mcp.port` from it.
pub const SETTINGS_FILE: &str = "app-settings.json";
/// The endpoint's access token, in the data folder.
pub const TOKEN_FILE: &str = "mcp-token";

const USAGE: &str = "\
Usage: perritodeck-mcp [--data-dir <folder>]

Connects an AI assistant to PerritoDeck: speaks MCP on stdin and stdout and passes it
to the running PerritoDeck app or host. Assistants start this for you; PerritoDeck shows
how to set them up under Settings, AI assistants.

  --data-dir <folder>  PerritoDeck's data folder, if it isn't the usual one
                       (also PERRITODECK_DATA_DIR)
  --version            Print the version
";

/// The usual data folder, the same as the host's `default_data_dir`.
pub fn default_data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "perritodeck", "PerritoDeck")
        .map(|d| d.config_dir().to_owned())
}

/// Where the host that keeps its data in `data_dir` listens for assistants: the port in
/// its settings (`mcp.port`, else the default) and its saved token.
pub fn endpoint(data_dir: &Path) -> Endpoint {
    let port = std::fs::read_to_string(data_dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|s| s.get("mcp")?.get("port")?.as_u64())
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(DEFAULT_PORT);
    let token = std::fs::read_to_string(data_dir.join(TOKEN_FILE))
        .map(|t| t.trim().to_owned())
        .unwrap_or_default();
    Endpoint {
        url: format!("http://127.0.0.1:{port}{ENDPOINT_PATH}"),
        token,
    }
}

/// Run the bridge with these arguments (without the program name); the exit code.
pub fn run(args: impl IntoIterator<Item = OsString>) -> i32 {
    let mut data_dir = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--data-dir") => match args.next() {
                Some(dir) => data_dir = Some(PathBuf::from(dir)),
                None => {
                    eprintln!("perritodeck-mcp: --data-dir needs a folder\n\n{USAGE}");
                    return 2;
                }
            },
            Some("--version" | "-V") => {
                println!(
                    "perritodeck-mcp {} (bridge version {BRIDGE_VERSION})",
                    env!("CARGO_PKG_VERSION")
                );
                return 0;
            }
            Some("--help" | "-h") => {
                print!("{USAGE}");
                return 0;
            }
            // The app's and the host's bridges are started with `mcp`; a setup that only
            // had its command changed to this program still works.
            Some("mcp") => {}
            _ => {
                eprintln!("perritodeck-mcp: unexpected argument {arg:?}\n\n{USAGE}");
                return 2;
            }
        }
    }
    let data_dir = data_dir
        .or_else(|| {
            std::env::var_os("PERRITODECK_DATA_DIR")
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
        })
        .or_else(default_data_dir);
    let Some(data_dir) = data_dir else {
        eprintln!("perritodeck-mcp: couldn't find PerritoDeck's data folder; pass --data-dir");
        return 1;
    };

    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_target(false)
        .try_init();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("perritodeck-mcp: {e}");
            return 1;
        }
    };
    // Runs even when PerritoDeck isn't running or its connection for assistants is off,
    // so the assistant shows the bridge's explanation instead of a server that failed
    // to start.
    match runtime.block_on(crate::bridge_stdio_to_http(AGENT, || endpoint(&data_dir))) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("perritodeck-mcp: the bridge stopped: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_from_the_hosts_files() {
        let dir = tempfile::tempdir().unwrap();
        // Nothing saved yet: the default port, no token.
        assert_eq!(
            endpoint(dir.path()),
            Endpoint {
                url: "http://127.0.0.1:46711/mcp".into(),
                token: String::new(),
            }
        );
        std::fs::write(
            dir.path().join(SETTINGS_FILE),
            r#"{"checkUpdates": true, "mcp": {"enabled": true, "port": 47000}, "later": {"x": 1}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join(TOKEN_FILE), "abc123\n").unwrap();
        assert_eq!(
            endpoint(dir.path()),
            Endpoint {
                url: "http://127.0.0.1:47000/mcp".into(),
                token: "abc123".into(),
            }
        );
        // A file it can't read gives the default port.
        std::fs::write(dir.path().join(SETTINGS_FILE), "{ not json").unwrap();
        assert_eq!(endpoint(dir.path()).url, "http://127.0.0.1:46711/mcp");
        std::fs::write(
            dir.path().join(SETTINGS_FILE),
            r#"{"mcp": {"port": 99999}}"#,
        )
        .unwrap();
        assert_eq!(endpoint(dir.path()).url, "http://127.0.0.1:46711/mcp");
    }

    #[test]
    fn arguments() {
        let run_with = |a: &[&str]| run(a.iter().map(OsString::from));
        assert_eq!(run_with(&["--version"]), 0);
        assert_eq!(run_with(&["--help"]), 0);
        assert_eq!(run_with(&["--data-dir"]), 2);
        assert_eq!(run_with(&["--nope"]), 2);
        assert_eq!(run_with(&["mcp", "--config"]), 2);
    }
}
