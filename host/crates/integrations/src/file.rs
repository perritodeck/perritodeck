//! What Settings → Integrations reads and writes: the form's view of `integrations.toml`
//! (never a password or token), its checks on what was typed, and the edit that saves a
//! section while keeping the rest of the file, comments and all, as it was.

use std::fmt;

use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::http::Uri;
use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

use crate::{home_assistant::HaConfig, obs::ObsConfig, CompanionConfig, Config};

/// A password or token from the form: keep the saved one, use a new one, or remove it.
/// The window never gets the saved one, so it can only say which.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Secret {
    Keep,
    Set(String),
    Clear,
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Secret::Keep => f.write_str("Keep"),
            Secret::Set(_) => f.write_str("Set(<hidden>)"),
            Secret::Clear => f.write_str("Clear"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsInput {
    pub url: String,
    pub password: Secret,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HaInput {
    pub url: String,
    pub token: Secret,
}

/// The port comes as typed, so the checks here say what's wrong with it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionInput {
    pub host: String,
    pub port: String,
}

/// What saving a section does.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", content = "settings", rename_all = "camelCase")]
pub enum Change<T> {
    /// Save these settings and turn it on.
    Use(T),
    /// Keep what's saved, with `enabled = false`.
    TurnOff,
    /// Delete the section.
    Remove,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum IntegrationInput {
    Obs { change: Change<ObsInput> },
    HomeAssistant { change: Change<HaInput> },
    Companion { change: Change<CompanionInput> },
}

/// What Test connection tries: the values in the form, saved or not.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum IntegrationTest {
    Obs(ObsInput),
    HomeAssistant(HaInput),
    Companion(CompanionInput),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsView {
    pub enabled: bool,
    pub url: String,
    pub has_password: bool,
    /// An address OBS keys can use, when the saved one can't work.
    pub suggested_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HaView {
    pub enabled: bool,
    pub url: String,
    pub has_token: bool,
    pub suggested_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionView {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
}

/// The top of a new `integrations.toml`.
pub const NEW_FILE_HEADER: &str = "\
# PerritoDeck's settings for OBS Studio, Home Assistant and Bitfocus Companion.
# Settings → Integrations in the PerritoDeck app edits this file; you can also edit it by
# hand (https://perritodeck.dev/guide/integrations#integrations-file). It holds passwords.
";

/// What Open the file makes when there's no file yet: the header and a commented
/// example, which reads as nothing set up.
pub const NEW_FILE_TEMPLATE: &str = "\
# PerritoDeck's settings for OBS Studio, Home Assistant and Bitfocus Companion.
# Settings → Integrations in the PerritoDeck app edits this file; you can also edit it by
# hand (https://perritodeck.dev/guide/integrations#integrations-file). It holds passwords.
#
# [obs]
# url = \"ws://127.0.0.1:4455\"       # OBS → Tools → WebSocket Server Settings
# password = \"…\"
#
# [home_assistant]
# url = \"http://homeassistant.local:8123\"
# token = \"…\"                        # Profile → Security → Long-lived access tokens
#
# [companion]                        # Bitfocus Companion, for profiles with companion = true
# host = \"127.0.0.1\"
# port = 16622                       # Companion's Satellite port (the default)
#
# Add enabled = false to a table to keep its settings but leave it off.
";

// ---- the form's messages (the app repeats them in lib/integrations.ts) ----

pub(crate) const OBS_EMPTY: &str = "Type OBS's address, like ws://127.0.0.1:4455.";
pub(crate) const OBS_WSS: &str =
    "OBS's WebSocket server uses ws://, not wss://. Change the address to start with ws://.";
pub(crate) const OBS_SCHEME: &str = "The address must start with ws://, like ws://127.0.0.1:4455.";
pub(crate) const UNUSABLE: &str = "That isn't an address PerritoDeck can use.";
const HA_EMPTY: &str =
    "Type the address you open Home Assistant with, like http://homeassistant.local:8123.";
const HA_SCHEME: &str = "The address must start with http:// or https://.";
const USERINFO: &str = "Leave any user name and password out of the address.";
const NO_TOKEN: &str = "Paste a long-lived access token.";
const SPACED_TOKEN: &str = "That doesn't look like an access token: it has spaces in it.";
const COMPANION_EMPTY: &str =
    "Type the name or address of the computer running Companion, like 127.0.0.1.";
const COMPANION_SCHEME: &str = "Type just the name or address, without http:// or ws://.";
const COMPANION_PORT_IN_HOST: &str = "Put the port in the Satellite port box.";
const PORT_RANGE: &str = "Use a number from 1 to 65535.";
const RETYPE_TOKEN: &str = "Type the token again to use it with a new address.";

const OBS_PORT: u16 = 4455;
const HA_PORT: u16 = 8123;

/// An address split up: `scheme://host:port/rest`. `host` keeps an IPv6 address's
/// brackets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Address {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
}

impl Address {
    /// `host:port`, with `default` for a missing port.
    pub(crate) fn host_port(&self, default: u16) -> String {
        format!("{}:{}", self.host, self.port.unwrap_or(default))
    }

    /// The host without an IPv6 address's brackets, for lookups.
    pub(crate) fn bare_host(&self) -> &str {
        crate::check::bare(&self.host)
    }
}

/// Split `text` (trimmed, with a scheme) into its parts. A user name or password, a
/// space, a missing host or a port outside 1–65535 make it unusable.
pub(crate) fn split_address(text: &str) -> Result<Address, &'static str> {
    if text.chars().any(char::is_whitespace) {
        return Err(UNUSABLE);
    }
    let (scheme, rest) = text.split_once("://").ok_or(UNUSABLE)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err(USERINFO);
    }
    let uri: Uri = text.parse().map_err(|_| UNUSABLE)?;
    let host = uri.host().filter(|h| !h.is_empty()).ok_or(UNUSABLE)?;
    // http's Uri takes any digits as a port; only 1–65535 can be used.
    let after_host = authority
        .rsplit_once(']')
        .map(|(_, tail)| tail)
        .unwrap_or(authority);
    let port = match after_host.rsplit_once(':') {
        Some((_, p)) => match p.parse::<u16>() {
            Ok(p) if p > 0 => Some(p),
            _ => return Err(UNUSABLE),
        },
        None => None,
    };
    let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("");
    Ok(Address {
        scheme: scheme.to_ascii_lowercase(),
        host: host.to_ascii_lowercase(),
        port,
        path: if path == "/" {
            String::new()
        } else {
            path.to_owned()
        },
    })
}

/// `scheme://` put in front of an address typed without one; an IPv6 address gets
/// brackets.
fn with_scheme(text: &str, scheme: &str) -> String {
    let bare_ipv6 = !text.contains('[') && text.matches(':').count() >= 2;
    if bare_ipv6 {
        format!("{scheme}://[{text}]")
    } else {
        format!("{scheme}://{text}")
    }
}

/// OBS's address as keys use it: `ws://host:port`, with 4455 when no port is given.
pub(crate) fn obs_url(text: &str) -> Result<String, &'static str> {
    let text = text.trim();
    let text = text.strip_suffix('/').unwrap_or(text);
    if text.is_empty() {
        return Err(OBS_EMPTY);
    }
    let full = match text.split_once("://") {
        Some((scheme, _)) if scheme.eq_ignore_ascii_case("wss") => return Err(OBS_WSS),
        Some((scheme, _)) if !scheme.eq_ignore_ascii_case("ws") => return Err(OBS_SCHEME),
        Some(_) => text.to_owned(),
        None => with_scheme(text, "ws"),
    };
    let a = split_address(&full)?;
    Ok(format!("ws://{}{}", a.host_port(OBS_PORT), a.path))
}

/// Home Assistant's address as keys use it: `scheme://host[:port]`. Typed without a
/// scheme it gets `http://` and port 8123. Anything after the address (a dashboard's
/// path) is dropped: the keys add `/api/…`.
pub(crate) fn ha_url(text: &str) -> Result<String, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Err(HA_EMPTY);
    }
    let (full, bare) = match text.split_once("://") {
        Some((scheme, _))
            if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") =>
        {
            (text.to_owned(), false)
        }
        Some(_) => return Err(HA_SCHEME),
        None => (with_scheme(text, "http"), true),
    };
    let a = split_address(&full)?;
    let authority = match (a.port, bare) {
        (None, true) => a.host_port(HA_PORT),
        (Some(p), _) => format!("{}:{p}", a.host),
        (None, false) => a.host.clone(),
    };
    Ok(format!("{}://{authority}", a.scheme))
}

/// A pasted token: trimmed, without a leading `Bearer `.
fn clean_token(text: &str) -> Result<String, &'static str> {
    let mut t = text.trim();
    if t.len() >= 6 && t.is_char_boundary(6) && t[..6].eq_ignore_ascii_case("bearer") {
        let rest = &t[6..];
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            t = rest.trim();
        }
    }
    if t.is_empty() {
        return Err(NO_TOKEN);
    }
    if t.chars().any(char::is_whitespace) {
        return Err(SPACED_TOKEN);
    }
    Ok(t.to_owned())
}

/// Whether `host` is on the local network: a loopback, private, link-local or unique
/// local address, a name without dots, or a name under `.local`, `.lan`, `.home.arpa` or
/// `.internal`.
pub(crate) fn is_local(host: &str) -> bool {
    use std::net::IpAddr;
    let host = crate::check::bare(host)
        .trim_end_matches('.')
        .to_ascii_lowercase();
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            ip.is_loopback() || (first & 0xffc0) == 0xfe80 || (first & 0xfe00) == 0xfc00
        }
        Err(_) => {
            !host.contains('.')
                || [".local", ".lan", ".home.arpa", ".internal"]
                    .iter()
                    .any(|end| host.ends_with(end))
        }
    }
}

/// The host in a saved address, when it can be read.
fn saved_host(url: &str) -> Option<String> {
    let url = url.trim();
    let full = if url.contains("://") {
        url.to_owned()
    } else {
        with_scheme(url, "http")
    };
    split_address(&full).ok().map(|a| a.bare_host().to_owned())
}

/// What the form shows for each section: `None` where nothing is saved.
pub fn view(c: &Config) -> (Option<ObsView>, Option<HaView>, Option<CompanionView>) {
    let obs = c.obs.as_ref().map(|o| ObsView {
        enabled: o.enabled,
        url: o.url.clone(),
        has_password: o.password.as_deref().is_some_and(|p| !p.is_empty()),
        suggested_url: obs_suggestion(&o.url),
    });
    let ha = c.home_assistant.as_ref().map(|h| HaView {
        enabled: h.enabled,
        url: h.url.clone(),
        has_token: !h.token.trim().is_empty(),
        suggested_url: ha_suggestion(&h.url),
    });
    let companion = c.companion.as_ref().map(|k| CompanionView {
        enabled: k.enabled,
        host: k.host.clone(),
        port: k.port,
    });
    (obs, ha, companion)
}

/// An address OBS keys can use, for a saved one they can't: without `ws://` (`wss://`
/// included), or without a port (`ws://pc` would dial port 80).
fn obs_suggestion(url: &str) -> Option<String> {
    let url = url.trim();
    let works = url
        .get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("ws://"))
        && split_address(url).is_ok_and(|a| a.port.is_some());
    if works {
        return None;
    }
    let candidate = match url.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("wss") => format!("ws://{rest}"),
        _ => url.to_owned(),
    };
    obs_url(&candidate).ok().filter(|s| s != url)
}

/// An address Home Assistant keys can use, for a saved one they can't: without a
/// scheme, or with a path after it (keys add `/api/…` to the whole address).
fn ha_suggestion(url: &str) -> Option<String> {
    let url = url.trim();
    let works = url.contains("://") && split_address(url).is_ok_and(|a| a.path.is_empty());
    if works {
        return None;
    }
    ha_url(url).ok().filter(|s| s != url.trim_end_matches('/'))
}

/// The settings to save or test from the form's OBS section. A kept password is the
/// one in `saved`.
pub fn resolve_obs(i: &ObsInput, saved: Option<&ObsConfig>) -> Result<ObsConfig, String> {
    let url = obs_url(&i.url)?;
    let password = match &i.password {
        Secret::Keep => saved.and_then(|s| s.password.clone()),
        Secret::Set(p) if p.is_empty() => None,
        Secret::Set(p) => Some(p.clone()),
        Secret::Clear => None,
    };
    Ok(ObsConfig {
        url,
        password,
        enabled: true,
    })
}

/// The settings to save or test from the form's Home Assistant section. A kept token is
/// the one in `saved`, and only for an address where it's safe to send it.
pub fn resolve_ha(i: &HaInput, saved: Option<&HaConfig>) -> Result<HaConfig, String> {
    let url = ha_url(&i.url)?;
    let token = match &i.token {
        Secret::Set(t) => clean_token(t)?,
        Secret::Clear => return Err(NO_TOKEN.into()),
        Secret::Keep => {
            let saved = saved
                .filter(|s| !s.token.trim().is_empty())
                .ok_or(NO_TOKEN)?;
            // The token travels in the clear over http://: never to a new host out on
            // the internet without being typed again.
            let new = split_address(&url).map_err(|e| e.to_string())?;
            let moved = saved_host(&saved.url).as_deref() != Some(new.bare_host());
            if moved && new.scheme == "http" && !is_local(&new.host) {
                return Err(RETYPE_TOKEN.into());
            }
            saved.token.clone()
        }
    };
    Ok(HaConfig {
        url,
        token,
        enabled: true,
    })
}

/// The settings to save or test from the form's Companion section.
pub fn resolve_companion(i: &CompanionInput) -> Result<CompanionConfig, String> {
    let host = i.host.trim();
    if host.is_empty() {
        return Err(COMPANION_EMPTY.into());
    }
    if host.contains("://") {
        return Err(COMPANION_SCHEME.into());
    }
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || "/?#@[]".contains(c))
    {
        return Err(UNUSABLE.into());
    }
    if host.matches(':').count() == 1 {
        return Err(COMPANION_PORT_IN_HOST.into());
    }
    let port = i.port.trim();
    let port = port
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| port.parse::<u32>().ok())
        .flatten()
        .filter(|p| (1..=65535).contains(p))
        .ok_or(PORT_RANGE)?;
    Ok(CompanionConfig {
        host: host.to_owned(),
        port: port as u16,
        enabled: true,
    })
}

/// `text` (the file as it is now) with one section changed as the form asks. What the
/// change doesn't touch stays byte for byte, comments included. A file with a mistake
/// is never changed.
pub fn edit(text: &str, input: &IntegrationInput) -> Result<String, String> {
    let current = Config::parse(text).map_err(|m| {
        format!(
            "integrations.toml has a mistake ({m}), so it can't be changed here. Fix it, or \
             start over."
        )
    })?;
    // toml_edit's own messages can quote the file, which holds passwords.
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|_| "integrations.toml can't be read, so it can't be changed here.".to_owned())?;
    let (key, name) = match input {
        IntegrationInput::Obs { .. } => ("obs", "OBS"),
        IntegrationInput::HomeAssistant { .. } => ("home_assistant", "Home Assistant"),
        IntegrationInput::Companion { .. } => ("companion", "Companion"),
    };
    let change = match input {
        IntegrationInput::Obs { change } => change_kind(change),
        IntegrationInput::HomeAssistant { change } => change_kind(change),
        IntegrationInput::Companion { change } => change_kind(change),
    };
    match change {
        Kind::Remove => {
            if !doc.contains_key(key) {
                return Ok(text.to_owned());
            }
            remove_table(&mut doc, key);
            return checked(doc.to_string());
        }
        Kind::TurnOff => {
            let table = doc
                .get_mut(key)
                .and_then(Item::as_table_like_mut)
                .ok_or_else(|| format!("There's nothing saved for {name} to turn off."))?;
            set(table, "enabled", Value::from(false));
            return checked(doc.to_string());
        }
        Kind::Use => {}
    }
    // The values to write, checked as Save and Test check them.
    let mut values: Vec<(&str, Option<Value>)> = Vec::new();
    match input {
        IntegrationInput::Obs {
            change: Change::Use(i),
        } => {
            let r = resolve_obs(i, current.obs.as_ref())?;
            values.push(("url", Some(r.url.into())));
            if !matches!(i.password, Secret::Keep) {
                values.push(("password", r.password.map(Value::from)));
            }
        }
        IntegrationInput::HomeAssistant {
            change: Change::Use(i),
        } => {
            let r = resolve_ha(i, current.home_assistant.as_ref())?;
            values.push(("url", Some(r.url.into())));
            if !matches!(i.token, Secret::Keep) {
                values.push(("token", Some(r.token.into())));
            }
        }
        IntegrationInput::Companion {
            change: Change::Use(i),
        } => {
            let r = resolve_companion(i)?;
            values.push(("host", Some(r.host.into())));
            values.push(("port", Some(i64::from(r.port).into())));
        }
        _ => unreachable!("only Use is left"),
    }
    // A file without any settings yet (new, or only comments): its text stays on top
    // and the section goes below it.
    if doc.as_table().is_empty() {
        let mut fresh = DocumentMut::new();
        let mut table = Table::new();
        for (k, v) in values {
            if let Some(v) = v {
                table.insert(k, Item::Value(v));
            }
        }
        fresh.insert(key, Item::Table(table));
        let mut out = if text.trim().is_empty() {
            NEW_FILE_HEADER.to_owned()
        } else {
            text.to_owned()
        };
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&fresh.to_string());
        return checked(out);
    }
    if !doc.contains_key(key) {
        let mut table = Table::new();
        table.decor_mut().set_prefix("\n");
        let last = doc
            .iter()
            .filter_map(|(_, item)| item.as_table().and_then(Table::position))
            .max();
        table.set_position(last.map(|p| p + 1));
        doc.insert(key, Item::Table(table));
    }
    let table = doc
        .get_mut(key)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| {
            format!("integrations.toml has {key} in a form that can't be changed here.")
        })?;
    for (k, v) in values {
        match v {
            Some(v) => set(table, k, v),
            None => {
                table.remove(k);
            }
        }
    }
    if table.contains_key("enabled") {
        set(table, "enabled", Value::from(true));
    }
    checked(doc.to_string())
}

enum Kind {
    Use,
    TurnOff,
    Remove,
}

fn change_kind<T>(c: &Change<T>) -> Kind {
    match c {
        Change::Use(_) => Kind::Use,
        Change::TurnOff => Kind::TurnOff,
        Change::Remove => Kind::Remove,
    }
}

/// The edited file, if it reads back as settings.
fn checked(out: String) -> Result<String, String> {
    Config::parse(&out)
        .map(|_| out)
        .map_err(|_| "Couldn't save integrations.toml: the change didn't come out right.".into())
}

/// Set `key` to `value`, keeping the spacing and comment around a value already there;
/// an equal value stays exactly as written.
fn set(table: &mut dyn TableLike, key: &str, mut value: Value) {
    match table.get_mut(key) {
        Some(item) => {
            if let Some(old) = item.as_value() {
                let same = match (old, &value) {
                    (Value::String(a), Value::String(b)) => a.value() == b.value(),
                    (Value::Integer(a), Value::Integer(b)) => a.value() == b.value(),
                    (Value::Boolean(a), Value::Boolean(b)) => a.value() == b.value(),
                    _ => false,
                };
                if same {
                    return;
                }
                *value.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(value);
        }
        None => {
            table.insert(key, Item::Value(value));
        }
    }
}

/// Delete a table with its own comments. What stands apart from it above (the file's
/// header, or anything up to the last blank line before it) stays, above the next table
/// or at the end of the file.
fn remove_table(doc: &mut DocumentMut, key: &str) {
    let (prefix, position) = match doc.get(key).and_then(Item::as_table) {
        Some(t) => (
            t.decor()
                .prefix()
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_owned(),
            t.position(),
        ),
        None => (String::new(), None),
    };
    doc.remove(key);
    // The part up to and including its last blank line isn't the table's own.
    let lines: Vec<&str> = prefix.split_inclusive('\n').collect();
    let keep: String = match lines.iter().rposition(|l| l.trim().is_empty()) {
        Some(i) => lines[..=i].concat(),
        None => String::new(),
    };
    if keep.is_empty() {
        return;
    }
    let next = position.and_then(|at| {
        doc.iter()
            .filter_map(|(k, item)| Some((k.to_owned(), item.as_table()?.position()?)))
            .filter(|(_, p)| *p > at)
            .min_by_key(|(_, p)| *p)
            .map(|(k, _)| k)
    });
    let join = |tail: &str| format!("{keep}{}", tail.trim_start_matches(['\r', '\n']));
    match next.and_then(|k| doc.get_mut(&k).and_then(Item::as_table_mut)) {
        Some(t) => {
            let old = t
                .decor()
                .prefix()
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_owned();
            t.decor_mut().set_prefix(join(&old));
        }
        None => {
            let old = doc.trailing().as_str().unwrap_or("").to_owned();
            doc.set_trailing(join(&old));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(url: &str, password: Secret) -> IntegrationInput {
        IntegrationInput::Obs {
            change: Change::Use(ObsInput {
                url: url.into(),
                password,
            }),
        }
    }

    fn ha(url: &str, token: Secret) -> IntegrationInput {
        IntegrationInput::HomeAssistant {
            change: Change::Use(HaInput {
                url: url.into(),
                token,
            }),
        }
    }

    fn companion(host: &str, port: &str) -> IntegrationInput {
        IntegrationInput::Companion {
            change: Change::Use(CompanionInput {
                host: host.into(),
                port: port.into(),
            }),
        }
    }

    const FILE: &str = "\
# My integrations. Edited by hand.

# The streaming PC
[obs]
url = \"ws://192.168.1.5:4455\"   # Tools → WebSocket Server Settings
password = 'hunter2'

[companion]
host = \"10.0.0.7\" # the rack
";

    #[test]
    fn edits_keep_comments_order_and_other_tables() {
        let out = edit(FILE, &obs("192.168.1.6", Secret::Keep)).unwrap();
        assert_eq!(
            out,
            "\
# My integrations. Edited by hand.

# The streaming PC
[obs]
url = \"ws://192.168.1.6:4455\"   # Tools → WebSocket Server Settings
password = 'hunter2'

[companion]
host = \"10.0.0.7\" # the rack
"
        );
        // A new section goes at the end, after a blank line.
        let out = edit(FILE, &ha("http://ha.lan:8123", Secret::Set("abc".into()))).unwrap();
        assert_eq!(
            out,
            format!("{FILE}\n[home_assistant]\nurl = \"http://ha.lan:8123\"\ntoken = \"abc\"\n")
        );
        let out = edit(FILE, &companion("10.0.0.8", "16623")).unwrap();
        assert!(
            out.ends_with("[companion]\nhost = \"10.0.0.8\" # the rack\nport = 16623\n"),
            "{out}"
        );
        assert!(out.starts_with(&FILE[..FILE.find("[companion]").unwrap()]));
    }

    #[test]
    fn keep_leaves_the_secret_as_written_and_clear_removes_it() {
        let out = edit(FILE, &obs("ws://192.168.1.5:4455", Secret::Keep)).unwrap();
        assert_eq!(out, FILE, "nothing changed, nothing rewritten");
        let out = edit(
            FILE,
            &obs("ws://192.168.1.5:4455", Secret::Set("s3cret".into())),
        )
        .unwrap();
        assert_eq!(out, FILE.replace("'hunter2'", "\"s3cret\""));
        let out = edit(FILE, &obs("ws://192.168.1.5:4455", Secret::Clear)).unwrap();
        assert_eq!(out, FILE.replace("password = 'hunter2'\n", ""));
        // Typing an empty password means no password.
        let out = edit(
            FILE,
            &obs("ws://192.168.1.5:4455", Secret::Set(String::new())),
        )
        .unwrap();
        assert_eq!(out, FILE.replace("password = 'hunter2'\n", ""));
    }

    #[test]
    fn turning_off_keeps_values_and_sets_enabled_false() {
        let off = IntegrationInput::Obs {
            change: Change::TurnOff,
        };
        let out = edit(FILE, &off).unwrap();
        assert_eq!(
            out,
            FILE.replace(
                "password = 'hunter2'\n",
                "password = 'hunter2'\nenabled = false\n"
            )
        );
        let c = Config::parse(&out).unwrap();
        assert!(!c.obs.as_ref().unwrap().enabled);
        assert_eq!(c.obs.unwrap().password.as_deref(), Some("hunter2"));
        // Again: the same.
        assert_eq!(edit(&out, &off).unwrap(), out);
        // Nothing saved for it: nothing to turn off.
        let e = edit(
            FILE,
            &IntegrationInput::HomeAssistant {
                change: Change::TurnOff,
            },
        )
        .unwrap_err();
        assert_eq!(e, "There's nothing saved for Home Assistant to turn off.");
    }

    #[test]
    fn turning_on_again_sets_an_existing_enabled_true_and_never_adds_one() {
        let off = FILE.replace(
            "password = 'hunter2'\n",
            "password = 'hunter2'\nenabled = false # for now\n",
        );
        let out = edit(&off, &obs("ws://192.168.1.5:4455", Secret::Keep)).unwrap();
        assert_eq!(
            out,
            FILE.replace(
                "password = 'hunter2'\n",
                "password = 'hunter2'\nenabled = true # for now\n"
            )
        );
        let out = edit(FILE, &companion("10.0.0.7", "16622")).unwrap();
        assert!(!out.contains("enabled"), "{out}");
    }

    #[test]
    fn removing_keeps_the_header_and_drops_the_tables_own_notes() {
        let text = "\
# header

# OBS notes
[obs]
url = \"ws://pc:4455\"
# HA notes
[home_assistant]
url = \"http://ha:8123\"
token = \"t\"
";
        let remove = IntegrationInput::Obs {
            change: Change::Remove,
        };
        assert_eq!(
            edit(text, &remove).unwrap(),
            "\
# header

# HA notes
[home_assistant]
url = \"http://ha:8123\"
token = \"t\"
"
        );
        // The last table: the header stays, in the file's tail.
        let remove = IntegrationInput::Companion {
            change: Change::Remove,
        };
        let out = edit(FILE, &remove).unwrap();
        assert_eq!(out, &FILE[..FILE.find("\n[companion]").unwrap() + 1]);
        // Nothing saved: nothing to do.
        let remove = IntegrationInput::HomeAssistant {
            change: Change::Remove,
        };
        assert_eq!(edit(FILE, &remove).unwrap(), FILE);
    }

    #[test]
    fn new_and_comment_only_files_keep_their_text_on_top() {
        let out = edit("", &companion("127.0.0.1", "16622")).unwrap();
        assert_eq!(
            out,
            format!("{NEW_FILE_HEADER}\n[companion]\nhost = \"127.0.0.1\"\nport = 16622\n")
        );
        let out = edit(NEW_FILE_TEMPLATE, &companion("127.0.0.1", "16622")).unwrap();
        assert_eq!(
            out,
            format!("{NEW_FILE_TEMPLATE}\n[companion]\nhost = \"127.0.0.1\"\nport = 16622\n")
        );
        assert_eq!(Config::parse(NEW_FILE_TEMPLATE).unwrap(), Config::default());
        let out = edit("# just a note", &companion("127.0.0.1", "16622")).unwrap();
        assert_eq!(
            out,
            "# just a note\n\n[companion]\nhost = \"127.0.0.1\"\nport = 16622\n"
        );
    }

    #[test]
    fn inline_and_dotted_tables_are_edited_in_place() {
        let inline = "obs = { url = \"ws://pc:4455\", password = \"pw\" } # inline\n";
        let out = edit(inline, &obs("ws://pc2:4455", Secret::Keep)).unwrap();
        assert_eq!(
            out,
            "obs = { url = \"ws://pc2:4455\", password = \"pw\" } # inline\n"
        );
        let dotted = "obs.url = \"ws://pc:4455\"\nobs.password = \"pw\"\n";
        let out = edit(dotted, &obs("ws://pc2:4455", Secret::Clear)).unwrap();
        assert_eq!(out, "obs.url = \"ws://pc2:4455\"\n");
        let off = IntegrationInput::Obs {
            change: Change::TurnOff,
        };
        let out = edit(inline, &off).unwrap();
        assert!(!Config::parse(&out).unwrap().obs.unwrap().enabled, "{out}");
    }

    #[test]
    fn a_file_with_a_mistake_is_never_edited() {
        // An unknown key is a mistake too: it would be lost, or kept and still wrong.
        for text in [
            "[obs]\npasword = \"x\"\n",
            "[obs\n",
            "[obs]\nfuture_key = 1\n",
        ] {
            let e = edit(text, &companion("127.0.0.1", "16622")).unwrap_err();
            assert!(e.starts_with("integrations.toml has a mistake ("), "{e}");
            assert!(
                e.ends_with("), so it can't be changed here. Fix it, or start over."),
                "{e}"
            );
            assert!(!e.contains("\"x\""));
        }
    }

    #[test]
    fn every_edit_loads() {
        let inputs = [
            obs("pc", Secret::Set("pw".into())),
            obs("ws://pc:4455", Secret::Clear),
            ha("ha.lan", Secret::Set("abc".into())),
            companion("::1", "16622"),
            IntegrationInput::Obs {
                change: Change::TurnOff,
            },
            IntegrationInput::Companion {
                change: Change::Remove,
            },
        ];
        for start in ["", NEW_FILE_TEMPLATE, FILE] {
            for i in &inputs {
                match edit(start, i) {
                    Ok(out) => {
                        Config::parse(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
                    }
                    Err(e) => assert!(e.starts_with("There's nothing saved"), "{e}"),
                }
            }
        }
    }

    /// The bad values come back as one of the form's messages; an edit with one changes
    /// nothing.
    #[test]
    fn bad_values_are_never_saved() {
        let e = edit(FILE, &obs("wss://pc:4455", Secret::Keep)).unwrap_err();
        assert!(e.contains("not wss://"), "{e}");
        let e = edit(FILE, &companion("10.0.0.7", "0")).unwrap_err();
        assert_eq!(e, "Use a number from 1 to 65535.");
    }

    #[derive(Deserialize)]
    struct Case {
        kind: String,
        input: serde_json::Value,
        #[serde(default)]
        saved: Option<serde_json::Value>,
        #[serde(default)]
        normalized: Option<serde_json::Value>,
        #[serde(default)]
        message: Option<String>,
    }

    /// The cases the app's own checks share (host/app/tests/integrations.spec.ts).
    #[test]
    fn validation_fixtures() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../fixtures/validation.json")).unwrap();
        for c in cases {
            let what = format!("{} {}", c.kind, c.input);
            let got = match c.kind.as_str() {
                "obs" => {
                    let saved: Option<ObsConfig> =
                        c.saved.map(|s| serde_json::from_value(s).unwrap());
                    let input: ObsInput = serde_json::from_value(c.input).unwrap();
                    resolve_obs(&input, saved.as_ref())
                        .map(|r| serde_json::json!({ "url": r.url, "password": r.password }))
                }
                "homeAssistant" => {
                    let saved: Option<HaConfig> =
                        c.saved.map(|s| serde_json::from_value(s).unwrap());
                    let input: HaInput = serde_json::from_value(c.input).unwrap();
                    resolve_ha(&input, saved.as_ref())
                        .map(|r| serde_json::json!({ "url": r.url, "token": r.token }))
                }
                "companion" => {
                    let input: CompanionInput = serde_json::from_value(c.input).unwrap();
                    resolve_companion(&input)
                        .map(|r| serde_json::json!({ "host": r.host, "port": r.port }))
                }
                other => panic!("unknown kind {other}"),
            };
            match (c.normalized, c.message) {
                (Some(want), None) => assert_eq!(got, Ok(want), "{what}"),
                (None, Some(want)) => assert_eq!(got, Err(want), "{what}"),
                _ => panic!("a case needs normalized or message: {what}"),
            }
        }
    }

    #[test]
    fn the_view_has_no_secrets_and_suggests_addresses_that_work() {
        let c = Config::parse(
            "[obs]\nurl = \"192.168.1.5\"\npassword = \"hunter2\"\n\n[home_assistant]\nurl = \"http://ha.lan:8123/lovelace/0\"\ntoken = \"hunter2\"\nenabled = false\n\n[companion]\nport = 16623\n",
        )
        .unwrap();
        let (o, h, k) = view(&c);
        let json = serde_json::to_string(&(&o, &h, &k)).unwrap();
        assert!(!json.contains("hunter2"), "{json}");
        assert_eq!(
            o,
            Some(ObsView {
                enabled: true,
                url: "192.168.1.5".into(),
                has_password: true,
                suggested_url: Some("ws://192.168.1.5:4455".into()),
            })
        );
        assert_eq!(
            h,
            Some(HaView {
                enabled: false,
                url: "http://ha.lan:8123/lovelace/0".into(),
                has_token: true,
                suggested_url: Some("http://ha.lan:8123".into()),
            })
        );
        assert_eq!(
            k,
            Some(CompanionView {
                enabled: true,
                host: "127.0.0.1".into(),
                port: 16623,
            })
        );
        // Addresses that work need no suggestion; an empty password isn't one.
        let c = Config::parse(
            "[obs]\nurl = \"ws://pc:4455/\"\npassword = \"\"\n[home_assistant]\nurl = \"https://ha.example.com/\"\ntoken = \"t\"\n",
        )
        .unwrap();
        let (o, h, _) = view(&c);
        let o = o.unwrap();
        assert!(!o.has_password);
        assert_eq!(o.suggested_url, None);
        assert_eq!(h.unwrap().suggested_url, None);
        // wss:// can't work; ws:// can.
        let c = Config::parse("[obs]\nurl = \"wss://pc:4455\"\n").unwrap();
        assert_eq!(
            view(&c).0.unwrap().suggested_url.as_deref(),
            Some("ws://pc:4455")
        );
        assert_eq!(view(&Config::default()), (None, None, None));
    }

    #[test]
    fn secrets_never_in_debug_or_the_inputs() {
        let dump = format!(
            "{:?} {:?} {:?}",
            obs("pc", Secret::Set("hunter2".into())),
            IntegrationTest::HomeAssistant(HaInput {
                url: "http://ha".into(),
                token: Secret::Set("hunter2".into()),
            }),
            Secret::Set("hunter2".into()),
        );
        assert!(!dump.contains("hunter2"), "{dump}");
        // The window's JSON reads as the TypeScript types say.
        let t: IntegrationTest = serde_json::from_str(
            r#"{"kind":"obs","url":"ws://pc:4455","password":{"kind":"set","value":"x"}}"#,
        )
        .unwrap();
        assert!(matches!(
            t,
            IntegrationTest::Obs(ObsInput {
                password: Secret::Set(_),
                ..
            })
        ));
        let i: IntegrationInput = serde_json::from_str(
            r#"{"kind":"homeAssistant","change":{"kind":"use","settings":{"url":"h","token":{"kind":"keep"}}}}"#,
        )
        .unwrap();
        assert!(matches!(
            i,
            IntegrationInput::HomeAssistant {
                change: Change::Use(HaInput {
                    token: Secret::Keep,
                    ..
                })
            }
        ));
        let i: IntegrationInput =
            serde_json::from_str(r#"{"kind":"companion","change":{"kind":"turnOff"}}"#).unwrap();
        assert!(matches!(
            i,
            IntegrationInput::Companion {
                change: Change::TurnOff
            }
        ));
    }
}
