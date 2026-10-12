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

/// Arguments that start the stdio bridge: `mcp`, plus the data folder when it isn't
/// the usual one (so the bridge finds this host's settings).
pub fn bridge_args(data_dir: &Path) -> Vec<String> {
    let mut args = vec!["mcp".to_owned()];
    let usual = crate::default_data_dir().ok();
    if usual.as_deref() != Some(data_dir) {
        args.push("--data-dir".into());
        args.push(data_dir.display().to_string());
    }
    args
}

/// `perritodeck-host mcp` (and the app's `mcp` argument): the stdio bridge to the
/// running host's endpoint, or with `config` the snippets for an assistant's settings.
/// Only the protocol (or the snippets) goes to stdout.
pub async fn run_command(data_dir: &Path, config: bool) -> Result<(), String> {
    let settings = crate::AppSettings::load(&crate::settings_path(data_dir));
    let port = settings.mcp.port;
    if config {
        let token = token_or_new(data_dir)?;
        let command = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "perritodeck-host".into());
        let s = snippets(&command, &bridge_args(data_dir), port, &token);
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
    // shows the bridge's explanation instead of a server that failed to start.
    let token = read_token(data_dir).unwrap_or_default();
    perritodeck_mcp::bridge_stdio_to_http(&url(port), &token)
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
}
