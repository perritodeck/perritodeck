//! Built-in integrations the host talks to on the user's behalf.
//!
//! Settings live in `integrations.toml` next to the host's other files (it holds
//! passwords and tokens, so it's kept out of the shareable profile):
//!
//! ```toml
//! [obs]
//! url = "ws://127.0.0.1:4455"       # OBS → Tools → WebSocket Server Settings
//! password = "…"
//!
//! [home_assistant]
//! url = "http://homeassistant.local:8123"
//! token = "…"                        # Profile → Security → Long-lived access tokens
//! ```

pub mod home_assistant;
pub mod obs;

use std::{path::Path, sync::Arc};

use perritodeck_actions::{ActionError, BoxFuture, Integrations, ObsAction, Outcome};
use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub obs: Option<obs::ObsConfig>,
    #[serde(default)]
    pub home_assistant: Option<home_assistant::HaConfig>,
}

impl Config {
    /// Read `integrations.toml`; a missing file means nothing is configured.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

/// The host's integrations, ready to hand to the action executor.
pub struct Builtins {
    obs: Option<obs::ObsClient>,
    ha: Option<home_assistant::HaClient>,
}

impl Builtins {
    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Self {
            obs: config.obs.map(obs::ObsClient::new),
            ha: config.home_assistant.map(home_assistant::HaClient::new),
        })
    }

    pub fn configured(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.obs.is_some() {
            v.push("OBS");
        }
        if self.ha.is_some() {
            v.push("Home Assistant");
        }
        v
    }
}

impl Integrations for Builtins {
    fn obs<'a>(&'a self, action: &'a ObsAction) -> BoxFuture<'a, Result<Outcome, ActionError>> {
        Box::pin(async move {
            let client = self
                .obs
                .as_ref()
                .ok_or_else(|| ActionError::NotConfigured("OBS".into()))?;
            client.run(action).await
        })
    }

    fn home_assistant<'a>(
        &'a self,
        service: &'a str,
        entity: Option<&'a str>,
        data: Option<&'a serde_json::Value>,
    ) -> BoxFuture<'a, Result<Outcome, ActionError>> {
        Box::pin(async move {
            let client = self
                .ha
                .as_ref()
                .ok_or_else(|| ActionError::NotConfigured("Home Assistant".into()))?;
            client.call(service, entity, data).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_loads_and_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        assert_eq!(Config::load(&path).unwrap(), Config::default());
        std::fs::write(
            &path,
            "[obs]\nurl = \"ws://127.0.0.1:4455\"\npassword = \"pw\"\n[home_assistant]\nurl = \"http://ha:8123\"\ntoken = \"t\"\n",
        )
        .unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!(c.obs.unwrap().password.as_deref(), Some("pw"));
        assert_eq!(c.home_assistant.unwrap().url, "http://ha:8123");
        std::fs::write(&path, "[obz]\n").unwrap();
        assert!(Config::load(&path).unwrap_err().contains("obz"));
    }

    #[tokio::test]
    async fn unconfigured_integrations_say_so() {
        let b = Builtins::new(Config::default());
        let err = b.obs(&ObsAction::ToggleRecord).await.unwrap_err();
        assert!(matches!(err, ActionError::NotConfigured(s) if s == "OBS"));
    }
}
