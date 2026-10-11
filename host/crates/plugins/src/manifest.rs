//! `perritodeck-plugin.json`: what a plugin is and what it offers.

use serde::{Deserialize, Serialize};

pub const MANIFEST: &str = "perritodeck-plugin.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    pub runtime: Runtime,
    /// Script (node/python) or program (exec), relative to the plugin's folder.
    pub main: String,
    #[serde(default)]
    pub actions: Vec<ActionDef>,
    #[serde(default)]
    pub values: Vec<ValueDef>,
    /// Settings for the whole plugin (a server address, an API token), asked for on the
    /// Plugins page and sent to the plugin in `hello` and `settings`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<Field>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Node,
    Python,
    Exec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub fields: Vec<Field>,
    /// The action takes a value from a slider (`settings.value`, 0–100) and can be put
    /// on a slider instead of a key.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub slider: bool,
}

/// One setting the desktop editor asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub id: String,
    pub label: String,
    #[serde(rename = "type", default = "default_field_type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<FieldOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

fn default_field_type() -> String {
    "text".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueDef {
    pub id: String,
    pub name: String,
    /// The value needs an argument saying what to show (a host, a VM id): titles use
    /// `{plugin:<plugin id>:<value id>:<argument>}`. This is the argument's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arg: Option<String>,
}

const FIELD_TYPES: &[&str] = &["text", "number", "checkbox", "select", "password"];

/// Plugin and action ids: lowercase letters, digits, `.`, `-`, `_`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
        && !id.starts_with('.')
        && !id.contains("..")
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let m: Manifest = serde_json::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
        m.check()?;
        Ok(m)
    }

    fn check(&self) -> Result<(), String> {
        if !valid_id(&self.id) {
            return Err(format!(
                "plugin id {:?} must be lowercase letters, digits, '.', '-' or '_'",
                self.id
            ));
        }
        if self.main.is_empty()
            || self.main.contains("..")
            || std::path::Path::new(&self.main).is_absolute()
        {
            return Err(format!(
                "\"main\" must be a file inside the plugin's folder, not {:?}",
                self.main
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for a in &self.actions {
            if !valid_id(&a.id) || !seen.insert(&a.id) {
                return Err(format!("action id {:?} is invalid or repeated", a.id));
            }
            for f in &a.fields {
                if !FIELD_TYPES.contains(&f.kind.as_str()) {
                    return Err(format!("field {:?} has unknown type {:?}", f.id, f.kind));
                }
            }
        }
        for f in &self.settings {
            if f.id.is_empty() || !FIELD_TYPES.contains(&f.kind.as_str()) {
                return Err(format!("setting {:?} has unknown type {:?}", f.id, f.kind));
            }
        }
        for v in &self.values {
            if !valid_id(&v.id) {
                return Err(format!("value id {:?} is invalid", v.id));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_example() {
        let m = Manifest::parse(
            r#"{ "id": "com.example.weather", "name": "Weather", "version": "1.0.0",
                 "runtime": "python", "main": "plugin.py",
                 "actions": [{ "id": "refresh", "name": "Refresh",
                   "fields": [{ "id": "city", "label": "City", "type": "text", "placeholder": "Lisbon" }] }],
                 "values": [{ "id": "temperature", "name": "Temperature" }] }"#,
        )
        .unwrap();
        assert_eq!(m.runtime, Runtime::Python);
        assert_eq!(m.actions[0].fields[0].kind, "text");
    }

    #[test]
    fn rejects_bad_ids_and_paths() {
        let base = |id: &str, main: &str| {
            format!(
                r#"{{ "id": "{id}", "name": "x", "version": "1", "runtime": "exec", "main": "{main}" }}"#
            )
        };
        assert!(Manifest::parse(&base("Com.Example", "p")).is_err());
        assert!(Manifest::parse(&base("../evil", "p")).is_err());
        assert!(Manifest::parse(&base("ok.id", "../outside")).is_err());
        assert!(Manifest::parse(&base("ok.id", "/bin/sh")).is_err());
        assert!(Manifest::parse(&base("ok.id", "bin/plugin")).is_ok());
        assert!(valid_id("dev.perritodeck.http") && !valid_id("") && !valid_id("a..b"));
    }
}

#[cfg(test)]
mod bundled {
    /// Every first-party plugin's manifest in `plugins/` is one the host accepts.
    #[test]
    fn first_party_manifests_parse() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../plugins");
        let mut n = 0;
        for e in std::fs::read_dir(&root).unwrap().flatten() {
            let m = e.path().join(super::MANIFEST);
            if m.is_file() {
                let text = std::fs::read_to_string(&m).unwrap();
                if let Err(err) = super::Manifest::parse(&text) {
                    panic!("{}: {err}", m.display());
                }
                n += 1;
            }
        }
        assert!(n >= 3, "found {n} plugins");
    }
}
