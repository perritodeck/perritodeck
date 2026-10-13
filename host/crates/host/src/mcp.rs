//! The local MCP server for AI assistants (`perritodeck-mcp`), connected to the host:
//! [`HostBackend`] answers its questions from the running [`Host`], and the host starts
//! and stops the HTTP endpoint as the settings say (see [`Host::apply_mcp`]).

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use perritodeck_mcp::{
    ActionSpec, Backend, BoxFuture, Edit, KeyDetail, LiveValue, PageDetail, Permissions, Press,
    ProfileDetail, ProfileInfo, TabletInfo,
};
use serde::Serialize;
use tracing::info;

use crate::{settings, Host, HostEvent};

/// Where the endpoint's access token is kept (owner-only), in the data folder.
pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join("mcp-token")
}

/// The saved access token, if there is one.
pub fn read_token(data_dir: &Path) -> Option<String> {
    std::fs::read_to_string(token_path(data_dir))
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
}

/// Save a new random token (readable only by this user) and return it.
pub fn new_token(data_dir: &Path) -> Result<String, String> {
    let token = perritodeck_mcp::generate_token();
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let path = token_path(data_dir);
    perritodeck_plugins::write_private(&path, token.as_bytes())
        .map_err(|e| format!("Couldn't save {}: {e}", path.display()))?;
    Ok(token)
}

/// The saved token, or a new one.
pub fn token_or_new(data_dir: &Path) -> Result<String, String> {
    match read_token(data_dir) {
        Some(t) => Ok(t),
        None => new_token(data_dir),
    }
}

/// The endpoint's URL for a port.
pub fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}{}", perritodeck_mcp::ENDPOINT_PATH)
}

/// Whether the endpoint is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum McpState {
    Off,
    #[serde(rename_all = "camelCase")]
    Running {
        url: String,
    },
    /// It's turned on but couldn't start (say, the port is in use).
    #[serde(rename_all = "camelCase")]
    Error {
        message: String,
    },
}

impl McpState {
    pub fn describe(&self) -> String {
        match self {
            McpState::Off => "off".into(),
            McpState::Running { url } => format!("running at {url}"),
            McpState::Error { message } => format!("on, but not running: {message}"),
        }
    }
}

/// What to paste into an assistant's settings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSnippets {
    /// The command assistants launch (this program) and its arguments.
    pub command: String,
    pub args: Vec<String>,
    /// `mcpServers` JSON for assistants that launch a command (Claude Desktop, …).
    pub stdio_config: String,
    /// `mcpServers` JSON for assistants that connect to a URL (includes the token).
    pub http_config: String,
    pub claude_code_stdio: String,
    pub claude_code_http: String,
}

/// Everything the Settings page shows about the MCP server.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpView {
    pub settings: settings::Mcp,
    pub state: McpState,
    pub url: String,
    pub snippets: McpSnippets,
    /// Since the endpoint started, an assistant has connected through the app's own
    /// program (`perritodeck-app mcp`, the setup from before the standalone bridge). It
    /// works, but keeps the app's program in use, so the app can't be updated meanwhile.
    pub app_bridge_used: bool,
}

/// The program names of the stdio bridges requests came through (`perritodeck-mcp`,
/// `perritodeck-app`, …).
#[derive(Debug, Default)]
pub(crate) struct BridgesSeen(std::sync::Mutex<std::collections::BTreeSet<String>>);

impl BridgesSeen {
    pub(crate) fn note(&self, agent: &str) {
        let name = agent.split('/').next().unwrap_or_default().trim();
        if !name.is_empty() {
            self.0
                .lock()
                .expect("bridges poisoned")
                .insert(name.to_owned());
        }
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.0.lock().expect("bridges poisoned").contains(name)
    }

    pub(crate) fn clear(&self) {
        self.0.lock().expect("bridges poisoned").clear();
    }
}

/// The snippets for a command (this program plus `args`), port and token.
pub fn snippets(command: &str, args: &[String], port: u16, token: &str) -> McpSnippets {
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    let pretty = |v: serde_json::Value| serde_json::to_string_pretty(&v).unwrap_or_default();
    McpSnippets {
        command: command.to_owned(),
        args: args.to_vec(),
        stdio_config: pretty(perritodeck_mcp::stdio_client_config(command, &args_ref)),
        http_config: pretty(perritodeck_mcp::http_client_config(&url(port), token)),
        claude_code_stdio: perritodeck_mcp::claude_code_stdio_command(command, &args_ref),
        claude_code_http: perritodeck_mcp::claude_code_http_command(&url(port), token),
    }
}

/// The standalone bridge's file name.
#[cfg(windows)]
pub const BRIDGE_FILE: &str = "perritodeck-mcp.exe";
#[cfg(not(windows))]
pub const BRIDGE_FILE: &str = "perritodeck-mcp";

/// Where assistants start the bridge from: `<data dir>/bin/perritodeck-mcp`. It's a copy
/// of the one that ships with the app, so a running assistant never keeps the app's own
/// program in use (on Windows that would stop the app from being updated).
pub fn bridge_path(data_dir: &Path) -> PathBuf {
    data_dir.join("bin").join(BRIDGE_FILE)
}

/// The bridge that ships next to this program (the app or the host), if there is one.
pub fn shipped_bridge() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join(BRIDGE_FILE);
    path.is_file().then_some(path)
}

/// What [`install_bridge`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeInstall {
    /// The copy in the data folder was already this one.
    Unchanged,
    /// Copied there, new or in place of another version.
    Copied,
    /// The copy there is in use and couldn't be replaced, so it stays as it is. It still
    /// works: the endpoint answers every bridge version.
    InUse,
}

/// Copy `shipped` to [`bridge_path`], unless the copy there is already the same.
///
/// On Windows a bridge an assistant is running can't be overwritten, but it can be
/// renamed: it's moved aside (`*.old`, deleted on a later start once nothing runs it) and
/// the new one takes its place, so the assistant's next session starts the new one.
pub fn install_bridge(shipped: &Path, data_dir: &Path) -> std::io::Result<BridgeInstall> {
    let dest = bridge_path(data_dir);
    let dir = data_dir.join("bin");
    std::fs::create_dir_all(&dir)?;
    // Copies moved aside earlier, once nothing runs them, and a copy left half done.
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(BRIDGE_FILE) && (name.ends_with(".old") || name.ends_with(".new")) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let new = std::fs::read(shipped)?;
    if std::fs::read(&dest).is_ok_and(|old| old == new) {
        return Ok(BridgeInstall::Unchanged);
    }
    let part = dir.join(format!("{BRIDGE_FILE}.new"));
    std::fs::write(&part, &new)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))?;
    }
    if std::fs::rename(&part, &dest).is_ok() {
        return Ok(BridgeInstall::Copied);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let aside = dir.join(format!("{BRIDGE_FILE}.{stamp}.old"));
    if std::fs::rename(&dest, &aside).is_ok() {
        if std::fs::rename(&part, &dest).is_ok() {
            return Ok(BridgeInstall::Copied);
        }
        let _ = std::fs::rename(&aside, &dest);
    }
    let _ = std::fs::remove_file(&part);
    Ok(BridgeInstall::InUse)
}

/// The command that starts the stdio bridge, and its arguments: the copy in the data
/// folder, or this program's own bridge (`mcp`) when there's no copy (the command-line
/// host on its own, or a development build). Plus the data folder when it isn't the
/// usual one, so the bridge finds this host's settings.
pub fn bridge_command(data_dir: &Path) -> (String, Vec<String>) {
    let copy = bridge_path(data_dir);
    let mut args = Vec::new();
    let command = if copy.is_file() {
        copy.display().to_string()
    } else {
        args.push("mcp".to_owned());
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "perritodeck-host".into())
    };
    let usual = crate::default_data_dir().ok();
    if usual.as_deref() != Some(data_dir) {
        args.push("--data-dir".into());
        args.push(data_dir.display().to_string());
    }
    (command, args)
}

/// Where the stdio bridge finds the host that uses `data_dir`: the port in its settings
/// and its saved token, read afresh each time (the same as `perritodeck-mcp` does).
pub fn bridge_endpoint(data_dir: &Path) -> perritodeck_mcp::Endpoint {
    perritodeck_mcp::cli::endpoint(data_dir)
}

/// `perritodeck-host mcp` (and the app's `mcp` argument): the stdio bridge to the
/// running host's endpoint, or with `config` the snippets for an assistant's settings.
/// Only the protocol (or the snippets) goes to stdout. `agent` names the program for the
/// host, e.g. `perritodeck-host/0.2.0`.
pub async fn run_command(data_dir: &Path, config: bool, agent: &str) -> Result<(), String> {
    if config {
        let settings = crate::AppSettings::load(&crate::settings_path(data_dir));
        let port = settings.mcp.port;
        let token = token_or_new(data_dir)?;
        let (command, args) = bridge_command(data_dir);
        let s = snippets(&command, &args, port, &token);
        if !settings.mcp.enabled {
            println!("Note: the connection for AI assistants is off. Turn it on in the app (Settings → AI assistants),");
            println!(
                "or set \"mcp\": {{\"enabled\": true}} in {} and restart the host.",
                crate::settings_path(data_dir).display()
            );
            println!();
        }
        println!("For assistants that start a command (Claude Desktop's claude_desktop_config.json, Cursor, …):");
        println!("{}", s.stdio_config);
        println!();
        println!("Claude Code:");
        println!("  {}", s.claude_code_stdio);
        println!();
        println!("For assistants that connect to a URL (keep the token private):");
        println!("{}", s.http_config);
        println!();
        println!("  {}", s.claude_code_http);
        return Ok(());
    }
    // Runs even when the endpoint is off or the host isn't running, so the assistant
    // shows the bridge's explanation instead of a server that failed to start. The
    // bridge looks the port and token up again if they stop working, so a new token or
    // port doesn't leave the assistant stuck until it restarts the bridge.
    perritodeck_mcp::bridge_stdio_to_http(agent, || bridge_endpoint(data_dir))
        .await
        .map_err(|e| format!("the MCP bridge stopped: {e}"))
}

/// Answers the MCP server from the host. Holds the host weakly: the host owns the
/// endpoint, which owns this.
pub(crate) struct HostBackend(pub(crate) Weak<Host>);

impl HostBackend {
    fn host(&self) -> Result<Arc<Host>, String> {
        self.0
            .upgrade()
            .ok_or_else(|| "PerritoDeck is shutting down.".to_owned())
    }
}

impl Backend for HostBackend {
    fn permissions(&self) -> Permissions {
        let Ok(host) = self.host() else {
            return Permissions::default();
        };
        let m = host
            .shared
            .settings
            .lock()
            .expect("settings poisoned")
            .mcp
            .clone();
        Permissions {
            press: m.press,
            edit: m.edit,
            run: m.run,
        }
    }

    fn list_profiles(&self) -> Vec<ProfileInfo> {
        let Ok(host) = self.host() else {
            return Vec::new();
        };
        host.profiles()
            .into_iter()
            .map(|p| ProfileInfo {
                id: p.id,
                name: p.name,
                pages: p.pages,
                is_default: p.is_default,
                has_error: p.has_error,
            })
            .collect()
    }

    fn get_profile(&self, id: &str) -> Result<ProfileDetail, String> {
        let v = self.host()?.profile_view(Some(id))?;
        Ok(ProfileDetail {
            id: v.id,
            name: v.name,
            rows: v.rows,
            cols: v.cols,
            fixed_grid: v.fixed_grid,
            companion: v.companion,
            reload_error: v.reload_error,
            pages: v
                .pages
                .into_iter()
                .map(|p| PageDetail {
                    name: p.name,
                    keys: p
                        .keys
                        .into_iter()
                        .map(|k| KeyDetail {
                            row: k.row,
                            col: k.col,
                            summary: k.detail,
                            fields: k.fields,
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    fn list_tablets(&self) -> Vec<TabletInfo> {
        let Ok(host) = self.host() else {
            return Vec::new();
        };
        host.devices()
            .into_iter()
            .map(|d| TabletInfo {
                page: d.connected.then(|| host.deck.device_page(&d.id)).flatten(),
                id: d.id,
                name: d.name,
                connected: d.connected,
                profile: d.profile,
            })
            .collect()
    }

    fn live_values(&self) -> Vec<LiveValue> {
        let Ok(host) = self.host() else {
            return Vec::new();
        };
        host.deck
            .live_tokens()
            .into_iter()
            .map(|name| LiveValue {
                value: host.deck.live_value(&name),
                name,
            })
            .collect()
    }

    fn press_key<'a>(&'a self, press: &'a Press) -> BoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(async move {
            let host = self.host()?;
            info!(
                "AI assistant pressed {}/{} [{}, {}]{}",
                press.profile,
                press.page,
                press.row,
                press.col,
                press
                    .tablet
                    .as_deref()
                    .map(|t| format!(" on {t}"))
                    .unwrap_or_default()
            );
            host.deck
                .press_at(
                    &press.profile,
                    &press.page,
                    [press.row, press.col],
                    press.tablet.as_deref(),
                )
                .await
        })
    }

    fn show_profile(&self, tablet_id: &str, profile_id: &str) -> Result<(), String> {
        let host = self.host()?;
        if !host
            .deck
            .connected_devices()
            .iter()
            .any(|(d, _)| d == tablet_id)
        {
            return Err(format!("Tablet {tablet_id:?} isn't connected."));
        }
        if host.deck.profile_by_id(profile_id).is_none() {
            return Err(format!("There's no profile called {profile_id:?}."));
        }
        info!(
            device = tablet_id,
            "AI assistant showed profile {profile_id}"
        );
        // Like a switch-profile key: remembered as the tablet's choice, and switching
        // rules leave it be until another app comes to the front.
        let default = host.deck.default_profile_id();
        let assign = (profile_id != default).then_some(profile_id);
        host.deck.assign(tablet_id, assign);
        host.shared.remember_key_switch(tablet_id, profile_id);
        let _ = host.shared.events.send(HostEvent::ProfileChanged);
        Ok(())
    }

    fn edit_profile(&self, profile_id: &str, edits: &[Edit]) -> Result<(), String> {
        info!("AI assistant edited profile {profile_id}");
        self.host()?.edit_profile(profile_id, edits).map(|_| ())
    }

    fn create_profile(&self, name: &str) -> Result<String, String> {
        let id = self.host()?.create_profile(name, None)?;
        info!("AI assistant created profile {id}");
        Ok(id)
    }

    fn run_action<'a>(
        &'a self,
        action: &'a ActionSpec,
    ) -> BoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(async move {
            let host = self.host()?;
            let kind = serde_json::to_value(action)
                .ok()
                .and_then(|v| v["type"].as_str().map(str::to_owned))
                .unwrap_or_default();
            info!("AI assistant ran a {kind} action");
            host.test_action(action).await
        })
    }

    fn bridge_seen(&self, agent: &str, _version: u32) {
        if let Ok(host) = self.host() {
            host.mcp_bridges.note(agent);
        }
    }
}
