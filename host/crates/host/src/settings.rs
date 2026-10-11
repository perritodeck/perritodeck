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
            .find(|r| {
                r.app.eq_ignore_ascii_case(app) && r.device.as_deref().map_or(true, |d| d == device)
            })
            .map(|r| r.profile.as_str())
    }
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
}
