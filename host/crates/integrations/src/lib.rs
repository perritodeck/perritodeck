//! Built-in integrations the host talks to on the user's behalf.
//!
//! Settings live in `integrations.toml` next to the host's other files (it holds
//! passwords and tokens, so it's kept out of the shareable profile and written
//! owner-only). The app's Settings → Integrations edits it ([`file`]); by hand it reads:
//!
//! ```toml
//! [obs]
//! url = "ws://127.0.0.1:4455"       # OBS → Tools → WebSocket Server Settings
//! password = "…"
//!
//! [home_assistant]
//! url = "http://homeassistant.local:8123"
//! token = "…"                        # Profile → Security → Long-lived access tokens
//!
//! [companion]                        # Bitfocus Companion, for profiles with companion = true
//! host = "127.0.0.1"
//! port = 16622                       # Companion's Satellite port (the default)
//! enabled = false                    # kept, but off (every table takes it)
//! ```

pub mod check;
pub mod file;
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
    #[serde(default)]
    pub companion: Option<CompanionConfig>,
}

/// Where Bitfocus Companion runs. Profiles with `companion = true` show its buttons.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionConfig {
    #[serde(default = "default_companion_host")]
    pub host: String,
    #[serde(default = "default_companion_port")]
    pub port: u16,
    /// `false` keeps the settings but leaves Companion off.
    #[serde(default = "on")]
    pub enabled: bool,
}

/// `enabled`'s default: a table that's there is on.
pub(crate) fn on() -> bool {
    true
}

fn default_companion_host() -> String {
    "127.0.0.1".into()
}

fn default_companion_port() -> u16 {
    16622
}

impl Config {
    /// Read `integrations.toml`; a missing file means nothing is configured.
    ///
    /// A mistake in it is described in one line, e.g. "line 3: unknown field
    /// `pasword`, …", without the excerpt of the file toml adds or any value from the
    /// file (either may show a password). The message goes to Settings, the
    /// command line and the log.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("it can't be read ({e})")),
        }
    }

    /// The settings in `text`, the contents of `integrations.toml`. A mistake is
    /// described as [`load`](Self::load) says.
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| {
            let message = without_values(e.message());
            match e.span() {
                Some(span) => {
                    let before = text.get(..span.start).unwrap_or(text);
                    format!("line {}: {message}", before.matches('\n').count() + 1)
                }
                None => message,
            }
        })
    }

    /// The integrations that are on, in the order the app lists them.
    pub fn names(&self) -> Vec<&'static str> {
        self.listed(true)
    }

    /// The integrations saved with `enabled = false`.
    pub fn turned_off(&self) -> Vec<&'static str> {
        self.listed(false)
    }

    fn listed(&self, enabled: bool) -> Vec<&'static str> {
        [
            ("OBS", self.obs.as_ref().map(|c| c.enabled)),
            (
                "Home Assistant",
                self.home_assistant.as_ref().map(|c| c.enabled),
            ),
            ("Companion", self.companion.as_ref().map(|c| c.enabled)),
        ]
        .into_iter()
        .filter(|(_, on)| *on == Some(enabled))
        .map(|(name, _)| name)
        .collect()
    }
}

/// toml's message for a mistake, minus the value it quotes for a value of the wrong
/// kind ("invalid type: integer `12345678`, expected a string"): a password typed
/// without quotes would show. Only the kind of mistake and what was expected are kept.
fn without_values(message: &str) -> String {
    let message = message.trim();
    let quoting = [
        ("invalid type:", "wrong kind of value"),
        ("invalid value:", "wrong value"),
        ("unknown variant", "not one of the choices"),
    ];
    let Some(kind) = quoting
        .iter()
        .find_map(|(start, kind)| message.starts_with(start).then_some(*kind))
    else {
        return message.to_owned();
    };
    // The value comes first and may itself contain ", expected"; what serde expected
    // comes last and never does.
    let expected = match message.rfind(", expected ") {
        Some(at) => &message[at + ", expected ".len()..],
        None => return kind.to_owned(),
    };
    let expected = if expected.starts_with("struct ") {
        "a table"
    } else if expected == "u16" {
        "a whole number from 0 to 65535"
    } else {
        expected
    };
    format!("{kind}, expected {expected}")
}

/// The host's integrations, ready to hand to the action executor.
pub struct Builtins {
    obs: Option<obs::ObsClient>,
    ha: Option<home_assistant::HaClient>,
    /// Saved, but with `enabled = false`.
    obs_off: bool,
    ha_off: bool,
    /// Why integrations.toml was ignored, if it was.
    mistake: Option<String>,
}

impl Builtins {
    /// `mistake` is why integrations.toml was ignored (from [`Config::load`]), if it
    /// was: OBS and Home Assistant actions then point at it, rather than asking for
    /// them to be added to a file that may already have them.
    pub fn new(config: Config, mistake: Option<String>) -> Arc<Self> {
        let (obs, obs_off) = match config.obs {
            Some(c) if c.enabled => (Some(obs::ObsClient::new(c)), false),
            Some(_) => (None, true),
            None => (None, false),
        };
        let (ha, ha_off) = match config.home_assistant {
            Some(c) if c.enabled => (Some(home_assistant::HaClient::new(c)), false),
            Some(_) => (None, true),
            None => (None, false),
        };
        Arc::new(Self {
            obs,
            ha,
            obs_off,
            ha_off,
            mistake,
        })
    }

    /// Why `service` can't run: the file was ignored, it's turned off, or it isn't set up.
    fn not_set_up(&self, service: &str, off: bool) -> ActionError {
        match &self.mistake {
            Some(mistake) => ActionError::IntegrationsMistake {
                service: service.into(),
                mistake: mistake.clone(),
            },
            None if off => ActionError::TurnedOff(service.into()),
            None => ActionError::NotSetUp(service.into()),
        }
    }

    fn obs_client(&self) -> Result<&obs::ObsClient, ActionError> {
        self.obs
            .as_ref()
            .ok_or_else(|| self.not_set_up("OBS", self.obs_off))
    }

    fn ha_client(&self) -> Result<&home_assistant::HaClient, ActionError> {
        self.ha
            .as_ref()
            .ok_or_else(|| self.not_set_up("Home Assistant", self.ha_off))
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
        Box::pin(async move { self.obs_client()?.run(action).await })
    }

    fn home_assistant<'a>(
        &'a self,
        service: &'a str,
        entity: Option<&'a str>,
        data: Option<&'a serde_json::Value>,
    ) -> BoxFuture<'a, Result<Outcome, ActionError>> {
        Box::pin(async move { self.ha_client()?.call(service, entity, data).await })
    }

    fn obs_request<'a>(
        &'a self,
        request_type: &'a str,
        data: serde_json::Value,
    ) -> BoxFuture<'a, Result<serde_json::Value, ActionError>> {
        Box::pin(async move { self.obs_client()?.call(request_type, data).await })
    }

    fn ha_state<'a>(
        &'a self,
        entity: &'a str,
    ) -> BoxFuture<'a, Result<serde_json::Value, ActionError>> {
        Box::pin(async move { self.ha_client()?.state(entity).await })
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
        std::fs::write(&path, "[companion]\nhost = \"10.0.0.5\"\n").unwrap();
        let c = Config::load(&path).unwrap().companion.unwrap();
        assert_eq!((c.host.as_str(), c.port), ("10.0.0.5", 16622));
        std::fs::write(&path, "[obz]\n").unwrap();
        assert!(Config::load(&path).unwrap_err().contains("obz"));
    }

    #[test]
    fn mistakes_name_the_line_but_never_show_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.toml");
        std::fs::write(
            &path,
            "[obs]\nurl = \"ws://127.0.0.1:4455\"\npasword = \"hunter2\"\n",
        )
        .unwrap();
        let e = Config::load(&path).unwrap_err();
        assert!(e.starts_with("line 3: unknown field `pasword`"), "{e}");
        assert!(!e.contains("hunter2"), "{e}");
        assert!(!e.contains('\n'), "{e}");

        // toml quotes a value of the wrong kind, such as a password without quotes.
        for (file, secret, says) in [
            (
                "[obs]\npassword = 12345678\n",
                "12345678",
                "line 2: wrong kind of value, expected a string",
            ),
            (
                "[home_assistant]\nurl = \"http://ha:8123\"\ntoken = 998877\n",
                "998877",
                "line 3: wrong kind of value, expected a string",
            ),
            (
                "[obs]\npassword = true\n",
                "true",
                "line 2: wrong kind of value, expected a string",
            ),
            (
                "obs = \"s3cret, expected 7\"\n",
                "s3cret",
                "line 1: wrong kind of value, expected a table",
            ),
            (
                "[companion]\nport = 99999\n",
                "99999",
                "line 2: wrong value, expected a whole number from 0 to 65535",
            ),
        ] {
            std::fs::write(&path, file).unwrap();
            let e = Config::load(&path).unwrap_err();
            assert_eq!(e, says);
            assert!(!e.contains(secret), "{e}");
        }
    }

    #[tokio::test]
    async fn unconfigured_integrations_say_so() {
        let b = Builtins::new(Config::default(), None);
        let err = b.obs(&ObsAction::ToggleRecord).await.unwrap_err();
        assert!(matches!(&err, ActionError::NotSetUp(s) if s == "OBS"));
        assert_eq!(
            err.to_string(),
            "OBS isn't set up; set it up in Settings → Integrations, or in integrations.toml"
        );
    }

    /// `enabled = false` keeps the settings but leaves the integration off; keys say
    /// where to turn it back on.
    #[tokio::test]
    async fn turned_off_says_turned_off() {
        let c = Config::parse(
            "[obs]\nenabled = false\npassword = \"hunter2\"\n\n[home_assistant]\nurl = \"http://ha:8123\"\ntoken = \"t\"\nenabled = false\n\n[companion]\n",
        )
        .unwrap();
        assert_eq!(c.names(), ["Companion"]);
        assert_eq!(c.turned_off(), ["OBS", "Home Assistant"]);
        assert_eq!(c.obs.as_ref().unwrap().password.as_deref(), Some("hunter2"));
        let b = Builtins::new(c, None);
        assert!(b.configured().is_empty());
        let key = b.obs(&ObsAction::ToggleRecord).await.unwrap_err();
        assert_eq!(
            key.to_string(),
            "OBS is turned off in Settings → Integrations"
        );
        let state = b.ha_state("light.desk").await.unwrap_err();
        assert_eq!(
            state.to_string(),
            "Home Assistant is turned off in Settings → Integrations"
        );
        // A table without the key is on.
        let c = Config::parse("[obs]\n[companion]\nenabled = true\n").unwrap();
        assert_eq!(c.names(), ["OBS", "Companion"]);
    }

    /// Settings and the log may print a config with `{:?}`; secrets never show.
    #[test]
    fn secrets_never_in_debug() {
        let c = Config::parse(
            "[obs]\npassword = \"hunter2\"\n[home_assistant]\nurl = \"http://ha:8123\"\ntoken = \"hunter2\"\n",
        )
        .unwrap();
        let dump = format!("{c:?} {:?} {:?}", c.obs, c.home_assistant);
        assert!(!dump.contains("hunter2"), "{dump}");
        assert!(dump.contains("<hidden>"), "{dump}");
        assert!(dump.contains("http://ha:8123"), "{dump}");
    }

    /// With a mistake in integrations.toml, keys and sliders point at it instead of
    /// asking for OBS or Home Assistant to be added.
    #[tokio::test]
    async fn an_ignored_integrations_toml_is_what_keys_and_sliders_say() {
        let mistake = "line 3: unknown field `pasword`, expected `url` or `password`";
        let b = Builtins::new(Config::default(), Some(mistake.into()));
        let obs = format!("integrations.toml has a mistake, so OBS is off: {mistake}");
        let ha = format!("integrations.toml has a mistake, so Home Assistant is off: {mistake}");
        let key = b.obs(&ObsAction::ToggleRecord).await.unwrap_err();
        assert_eq!(key.to_string(), obs);
        let slider = b
            .obs_request("SetInputVolume", serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!(slider.to_string(), obs);
        let key = b
            .home_assistant("light.toggle", None, None)
            .await
            .unwrap_err();
        assert_eq!(key.to_string(), ha);
        assert_eq!(b.ha_state("light.desk").await.unwrap_err().to_string(), ha);
    }
}
