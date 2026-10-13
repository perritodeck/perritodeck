//! The desktop host's own settings (`app-settings.json`): which tablet shows which
//! profile, and the rules for switching profiles with the app in front.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    /// Device id → profile id, for tablets that don't show the default profile.
    pub device_profiles: BTreeMap<String, String>,
    pub auto_switch: AutoSwitch,
    pub network: Network,
    /// Look for a newer version when the app starts.
    pub check_updates: bool,
    /// Write debug-level messages to the log, for bug reports.
    pub detailed_logs: bool,
    /// "Start at login", as last chosen in PerritoDeck. The system's own entry is the
    /// switch; this is kept so the app can put the entry back after an update removed
    /// it (on Windows, an update runs the old version's uninstaller).
    pub start_at_login: bool,
    /// The local MCP server for AI assistants.
    pub mcp: Mcp,
}

/// The local MCP server that lets AI assistants see and use the keys. The access token
/// isn't kept here but in its own owner-only file (`mcp-token`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Mcp {
    pub enabled: bool,
    /// Press existing keys and switch the profile a tablet shows.
    pub press: bool,
    /// Create and change keys, pages and profiles.
    pub edit: bool,
    /// Run any action, including programs and commands.
    pub run: bool,
    /// Its port on 127.0.0.1.
    pub port: u16,
}

impl Default for Mcp {
    fn default() -> Self {
        Self {
            enabled: false,
            press: false,
            edit: false,
            run: false,
            port: perritodeck_mcp::DEFAULT_PORT,
        }
    }
}

/// How tablets reach this computer. Changes apply when the host restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Network {
    pub port: u16,
    /// Accept tablets over Wi-Fi/Ethernet; off means USB (and this computer) only.
    pub lan: bool,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            port: perritodeck_protocol::DEFAULT_PORT,
            lan: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoSwitch {
    pub enabled: bool,
    pub rules: Vec<SwitchRule>,
}

/// "When `app` is in front, show `profile`" (on every tablet, or just `device`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchRule {
    pub app: String,
    pub profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

impl AutoSwitch {
    /// The profile `device` should show while `app` is in front, if a rule says so.
    pub fn profile_for(&self, app: Option<&str>, device: &str) -> Option<&str> {
        if !self.enabled {
            return None;
        }
        let app = app?;
        self.rules
            .iter()
            .find(|r| same_app(&r.app, app) && r.device.as_deref().map_or(true, |d| d == device))
            .map(|r| r.profile.as_str())
    }
}

/// Whether a rule's app is `app`, ignoring case, spaces around it and a trailing `.exe`:
/// on Windows the app in front is `obs64`, but people know it as `obs64.exe`.
fn same_app(rule: &str, app: &str) -> bool {
    fn bare(name: &str) -> &str {
        let name = name.trim();
        let cut = name.len().saturating_sub(4);
        match name.get(cut..) {
            Some(ext) if cut > 0 && ext.eq_ignore_ascii_case(".exe") => &name[..cut],
            _ => name,
        }
    }
    bare(rule).eq_ignore_ascii_case(bare(app))
}

impl AppSettings {
    /// A missing or unreadable file gives the defaults (and says why in the log).
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("ignoring {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).expect("settings serialize");
        let tmp = path.with_extension("json.saving");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_match_app_names_ignoring_case_and_per_device() {
        let mut a = AutoSwitch {
            enabled: true,
            rules: vec![
                SwitchRule {
                    app: "obs64".into(),
                    profile: "streaming".into(),
                    device: Some("desk".into()),
                },
                SwitchRule {
                    app: "OBS64".into(),
                    profile: "obs-all".into(),
                    device: None,
                },
                SwitchRule {
                    app: "Code".into(),
                    profile: "coding".into(),
                    device: None,
                },
            ],
        };
        assert_eq!(a.profile_for(Some("Obs64"), "desk"), Some("streaming"));
        assert_eq!(a.profile_for(Some("obs64"), "couch"), Some("obs-all"));
        assert_eq!(a.profile_for(Some("code"), "desk"), Some("coding"));
        assert_eq!(a.profile_for(Some("firefox"), "desk"), None);
        assert_eq!(a.profile_for(None, "desk"), None);
        a.enabled = false;
        assert_eq!(a.profile_for(Some("code"), "desk"), None);
    }

    #[test]
    fn rules_typed_with_exe_or_spaces_still_match() {
        let a = AutoSwitch {
            enabled: true,
            rules: vec![
                SwitchRule {
                    app: "firefox.exe".into(),
                    profile: "browsing".into(),
                    device: None,
                },
                SwitchRule {
                    app: " Code.EXE ".into(),
                    profile: "coding".into(),
                    device: None,
                },
                SwitchRule {
                    app: ".exe".into(),
                    profile: "nothing".into(),
                    device: None,
                },
            ],
        };
        assert_eq!(a.profile_for(Some("firefox"), "desk"), Some("browsing"));
        assert_eq!(a.profile_for(Some("Firefox"), "desk"), Some("browsing"));
        assert_eq!(a.profile_for(Some("code"), "desk"), Some("coding"));
        assert_eq!(a.profile_for(Some("firefox-esr"), "desk"), None);
        assert_eq!(a.profile_for(Some("exe"), "desk"), None);
        assert!(same_app("obs64", "obs64.exe"));
        assert!(!same_app("obs64.exe.bak", "obs64"));
        assert!(!same_app("notepad", "notepad++"));
        // Not every name is ASCII; looking for ".exe" mustn't split a character.
        assert!(same_app("café.exe", "Café"));
        assert!(same_app("a€xe", "A€XE"));
    }

    #[test]
    fn round_trip_and_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app-settings.json");
        assert_eq!(AppSettings::load(&path), AppSettings::default());
        let mut s = AppSettings::default();
        s.device_profiles.insert("tab".into(), "streaming".into());
        s.auto_switch.rules.push(SwitchRule {
            app: "obs64".into(),
            profile: "streaming".into(),
            device: None,
        });
        s.save(&path).unwrap();
        assert_eq!(AppSettings::load(&path), s);
        std::fs::write(&path, "{nope").unwrap();
        assert_eq!(AppSettings::load(&path), AppSettings::default());
    }

    #[test]
    fn mcp_is_off_by_default_and_older_files_still_load() {
        let s = AppSettings::default();
        assert!(!s.mcp.enabled && !s.mcp.press && !s.mcp.edit && !s.mcp.run);
        assert_eq!(s.mcp.port, 46711);
        assert!(!s.detailed_logs);
        let old: AppSettings =
            serde_json::from_str(r#"{"checkUpdates": true, "mcp": {"enabled": true}}"#).unwrap();
        assert!(old.check_updates && old.mcp.enabled);
        assert_eq!(old.mcp.port, 46711);
    }
}
