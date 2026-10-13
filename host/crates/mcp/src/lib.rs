//! PerritoDeck's local MCP server: lets AI assistants (anything that speaks the
//! [Model Context Protocol](https://modelcontextprotocol.io)) look at profiles, press keys,
//! and create or change keys.
//!
//! The protocol is implemented here from the specification (JSON-RPC 2.0, no SDK). The
//! crate doesn't know about the host: the host implements [`Backend`], and this crate
//! turns MCP tool calls into calls on it.
//!
//! # Transports
//!
//! * **Streamable HTTP** ([`serve_http`]): the host listens on `127.0.0.1` only
//!   ([`DEFAULT_PORT`], path `/mcp`). Every request needs `Authorization: Bearer <token>`
//!   (the host makes one with [`generate_token`] and shows it in its settings). Requests
//!   whose `Host` isn't the loopback address and port, or whose `Origin` isn't a
//!   localhost page, are refused with 403, which stops web pages (DNS rebinding) from
//!   reaching it. Responses are plain JSON; there's no SSE stream (GET answers 405).
//! * **stdio** ([`serve_stdio`] / [`serve_stream`]): newline-delimited JSON-RPC.
//! * **stdio → HTTP bridge** ([`bridge_stdio_to_http`]): many assistants only launch a
//!   command and talk to it over stdin/stdout. That command can't own the host (only one
//!   host runs at a time), so `perritodeck-host mcp` runs this bridge: it forwards each
//!   line to the running host's HTTP endpoint and prints the answers. The assistant's
//!   config then only needs the command; the bridge reads the port and token from the
//!   host's settings, and reads them again after they change.
//!
//! [`stdio_client_config`] and [`http_client_config`] make the JSON snippet an
//! assistant's config file needs.
//!
//! # Protocol versions
//!
//! Both eras of MCP are served:
//!
//! * the handshake-based versions `2025-11-25`, `2025-06-18`, `2025-03-26` and
//!   `2024-11-05` (`initialize`, then `notifications/initialized`; HTTP sessions with
//!   `Mcp-Session-Id`; JSON-RPC batches only for the two oldest versions), and
//! * the stateless `2026-07-28` (every request carries its version and client
//!   capabilities in `_meta`; `server/discover`; `resultType` on results; `Mcp-Method` and
//!   `Mcp-Name` headers checked against the body on HTTP).
//!
//! # Permissions
//!
//! Reading (profiles, tablets, live values) is always possible while the server runs.
//! Everything else is off until the person turns it on ([`Permissions`]): `press`
//! (press existing keys, switch a tablet's profile), `edit` (create, change, move and
//! remove keys and pages) and `run` (run any action, including commands). Tools that
//! aren't allowed aren't listed, and calling one returns an error that says which
//! setting would allow it. Because a key the assistant makes could be pressed later,
//! giving a key an action that runs programs or plugins (command, launch, open a
//! file, plugin actions) also needs `run`.

mod bridge;
pub mod config;
mod engine;
mod http;
mod jsonrpc;
mod stdio;
mod tools;

use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};

pub use bridge::{bridge, bridge_stdio_to_http, bridge_with, Endpoint};
pub use config::{
    claude_code_http_command, claude_code_stdio_command, http_client_config, stdio_client_config,
    SERVER_KEY,
};
pub use engine::{Server, Session};
pub use http::{serve_http, HttpOptions, HttpServer};
pub use perritodeck_actions::ActionSpec;
pub use perritodeck_profile::edit::{Edit, KeyFields};
pub use stdio::{serve_stdio, serve_stream};

/// The HTTP port the host listens on by default (the tablet port is 46710).
pub const DEFAULT_PORT: u16 = 46711;
/// The HTTP endpoint's path.
pub const ENDPOINT_PATH: &str = "/mcp";
/// Largest request body (and stdio line) accepted.
pub const DEFAULT_MAX_BODY: usize = 1024 * 1024;

/// Handshake-based protocol versions, newest first.
pub const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// Stateless protocol versions (version in each request's `_meta`), newest first.
pub const MODERN_VERSIONS: &[&str] = &["2026-07-28"];

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the person lets assistants do. All off by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    /// Press keys that already exist, and switch the profile a tablet shows.
    pub press: bool,
    /// Create, change, move and remove keys and pages, and create profiles.
    pub edit: bool,
    /// Run any action directly (hotkeys, text, commands, …), and give keys actions that
    /// run programs or plugins.
    pub run: bool,
}

/// One profile in [`Backend::list_profiles`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    /// The file name without `.toml`.
    pub id: String,
    pub name: String,
    pub pages: usize,
    /// Shown on tablets that haven't been given another profile.
    pub is_default: bool,
    /// Its file has a mistake; the last good version is in use.
    pub has_error: bool,
}

/// A whole profile, as [`Backend::get_profile`] returns it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDetail {
    pub id: String,
    pub name: String,
    pub rows: u32,
    pub cols: u32,
    /// True when the profile fixes the grid; otherwise it follows each tablet and
    /// `rows`/`cols` are only a preview size.
    pub fixed_grid: bool,
    /// Shows Bitfocus Companion's buttons; `pages` is then unused.
    pub companion: bool,
    pub reload_error: Option<String>,
    pub pages: Vec<PageDetail>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageDetail {
    pub name: String,
    pub keys: Vec<KeyDetail>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyDetail {
    pub row: u32,
    pub col: u32,
    /// Short description of what it does, e.g. "hotkey Ctrl+C" (the host's
    /// `KeyView::detail`). Empty to let this crate describe it.
    pub summary: String,
    /// The key as written in the profile.
    pub fields: KeyFields,
}

/// A paired tablet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabletInfo {
    pub id: String,
    pub name: String,
    pub connected: bool,
    /// The profile it shows now (or will show when it connects).
    pub profile: String,
    /// The page it shows, if known.
    pub page: Option<String>,
}

/// A value key titles can show as `{name}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveValue {
    pub name: String,
    /// The current value, if known.
    pub value: Option<String>,
}

/// A key to press, resolved: the profile and page exist and a key is at `[row, col]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Press {
    pub profile: String,
    pub page: String,
    pub row: u32,
    pub col: u32,
    /// A tablet id, when the assistant named one.
    pub tablet: Option<String>,
}

/// What the host gives the MCP server. Errors are sentences for the assistant to read
/// (and pass on to the person), like the host's own `Result<_, String>`s.
///
/// The cheap reads are synchronous; anything that runs an action returns a future.
pub trait Backend: Send + Sync {
    /// What the person allows right now. Read on every request, so changing a setting
    /// takes effect at once.
    fn permissions(&self) -> Permissions;

    fn list_profiles(&self) -> Vec<ProfileInfo>;

    /// A profile by id (the default one is marked in [`Backend::list_profiles`]).
    fn get_profile(&self, id: &str) -> Result<ProfileDetail, String>;

    fn list_tablets(&self) -> Vec<TabletInfo>;

    /// Values titles can show (`cpu`, `ha:sensor.outside`, `plugin:<id>:<value>`, …),
    /// without the clock ones. Default: none.
    fn live_values(&self) -> Vec<LiveValue> {
        Vec::new()
    }

    /// Press a key as if it were tapped on a tablet (keys paused → `Err`):
    ///
    /// * an action key runs its action and flashes ok/alert on tablets showing it;
    /// * a toggle runs its current state's action and moves to the next state (one that
    ///   `follow`s a live value runs the action for the state it shows, without flipping);
    /// * a navigation key (`nav`) changes the page on `tablet`, or else on every connected
    ///   tablet showing this profile; with none, it's an error;
    /// * a Stream Deck plugin key gets a key down and key up;
    /// * a key with neither does nothing (`Ok(None)`).
    ///
    /// `Ok(Some(note))` means it ran but reported a problem (a command that failed, say).
    /// Sliders are never passed here.
    fn press_key<'a>(&'a self, press: &'a Press) -> BoxFuture<'a, Result<Option<String>, String>>;

    /// Switch a connected tablet to a profile for now, as auto-switching does (its
    /// assigned profile doesn't change).
    fn show_profile(&self, tablet_id: &str, profile_id: &str) -> Result<(), String>;

    /// Apply edits to a profile's file (checked as a whole, shown on tablets right away,
    /// and undoable in the app like any other edit).
    fn edit_profile(&self, profile_id: &str, edits: &[Edit]) -> Result<(), String>;

    /// Make a new, empty profile; returns its id.
    fn create_profile(&self, name: &str) -> Result<String, String>;

    /// Run an action now. `Ok(Some(note))` as for [`Backend::press_key`].
    fn run_action<'a>(
        &'a self,
        action: &'a ActionSpec,
    ) -> BoxFuture<'a, Result<Option<String>, String>>;
}

/// A new random access token for the HTTP endpoint: 256 bits, hex.
pub fn generate_token() -> String {
    use rand::Rng;
    let mut raw = [0u8; 32];
    rand::rng().fill(&mut raw);
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time comparison of two secrets (hashed first, so lengths don't leak).
pub(crate) fn same_secret(a: &str, b: &str) -> bool {
    use sha2::{Digest, Sha256};
    let (x, y) = (Sha256::digest(a.as_bytes()), Sha256::digest(b.as_bytes()));
    x.iter()
        .zip(y.iter())
        .fold(0u8, |acc, (p, q)| acc | (p ^ q))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (generate_token(), generate_token());
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert!(same_secret(&a, &a.clone()));
        assert!(!same_secret(&a, &b));
        assert!(!same_secret(&a, ""));
    }
}
