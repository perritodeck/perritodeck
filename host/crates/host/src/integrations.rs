//! Settings → Integrations on the host's side: the form's view of `integrations.toml`,
//! saving a section, Test connection, and the file's privacy.
//!
//! Saved settings are used from the next start (the app offers "Restart now"): the
//! running OBS, Home Assistant and Companion clients keep what they started with.

use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use perritodeck_core::private;
use perritodeck_integrations::{
    check::{self, Checked, Resolve},
    file::{self, CompanionView, HaView, IntegrationInput, IntegrationTest, ObsView, Secret},
    CompanionConfig, Config,
};
use serde::Serialize;
use tracing::{info, warn};

use crate::{integrations_path, Host};

const NAME: &str = "integrations.toml";

/// Who can read `integrations.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IntegrationsPrivacy {
    Private,
    ReadableByOthers,
    /// The drive keeps no permissions, or asking failed.
    Unknown,
    NoFile,
}

/// What Settings → Integrations shows. Never a password or token: only whether one is
/// saved.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationsView {
    pub path: String,
    /// The file's mistake now (one line, no values); the form is then read-only.
    pub mistake: Option<String>,
    /// The file has a mistake now, but PerritoDeck runs on the good settings it read
    /// when it started.
    pub running_older: bool,
    /// The file says something other than what PerritoDeck started with: a restart
    /// puts it to use.
    pub restart_needed: bool,
    pub privacy: IntegrationsPrivacy,
    pub obs: Option<ObsView>,
    pub home_assistant: Option<HaView>,
    pub companion: Option<CompanionView>,
}

/// What PerritoDeck read from `integrations.toml` when it started.
#[derive(Debug, Clone, Default)]
pub(crate) struct Started {
    pub config: Config,
    pub error: Option<String>,
}

/// Who can read `path`, for the form and the diagnostics.
pub(crate) fn privacy_of(path: &Path) -> IntegrationsPrivacy {
    if !path.exists() {
        return IntegrationsPrivacy::NoFile;
    }
    match private::privacy(path) {
        private::Privacy::Private => IntegrationsPrivacy::Private,
        private::Privacy::ReadableByOthers => IntegrationsPrivacy::ReadableByOthers,
        private::Privacy::Unknown => IntegrationsPrivacy::Unknown,
    }
}

/// What the diagnostics say about the file's privacy, when there's something to say.
pub(crate) fn privacy_line(path: &Path) -> Option<&'static str> {
    match privacy_of(path) {
        IntegrationsPrivacy::ReadableByOthers => {
            Some("integrations.toml can be read by other accounts on this computer")
        }
        IntegrationsPrivacy::Unknown => Some(
            "PerritoDeck can't tell who can read integrations.toml (the drive keeps no \
             permissions)",
        ),
        IntegrationsPrivacy::Private | IntegrationsPrivacy::NoFile => None,
    }
}

/// The start-up warning for a file other accounts can read. It isn't changed: saving
/// from Settings (or chmod) makes it private.
pub(crate) fn warn_if_readable(path: &Path) {
    if privacy_of(path) != IntegrationsPrivacy::ReadableByOthers {
        return;
    }
    if cfg!(windows) {
        warn!(
            "integrations.toml can be read by other accounts on this computer; the app's \
             Settings → Integrations can make it private"
        );
    } else {
        warn!(
            "integrations.toml can be read by other accounts on this computer; run chmod 600 \
             {}, or save it from the app's Settings → Integrations",
            path.display()
        );
    }
}

/// The form's view of the file at `path`.
fn read_view(path: &Path, started: &Started) -> IntegrationsView {
    let (config, mistake) = match std::fs::read_to_string(path) {
        Ok(text) => match Config::parse(&text) {
            Ok(c) => (c, None),
            Err(m) => (Config::default(), Some(m)),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => (Config::default(), None),
        Err(e) => (Config::default(), Some(format!("it can't be read ({e})"))),
    };
    let running_older =
        mistake.is_some() && started.error.is_none() && started.config != Config::default();
    let restart_needed = match (&mistake, &started.error) {
        (None, None) => config != started.config,
        // Fixed since it started.
        (None, Some(_)) => true,
        // Nothing new to use until it's fixed.
        (Some(_), _) => false,
    };
    let (obs, home_assistant, companion) = file::view(&config);
    IntegrationsView {
        path: path.display().to_string(),
        mistake,
        running_older,
        restart_needed,
        privacy: privacy_of(path),
        obs,
        home_assistant,
        companion,
    }
}

/// What a change is called in the log: never its values.
fn described(input: &IntegrationInput) -> String {
    use file::Change;
    let (name, what) = match input {
        IntegrationInput::Obs { change } => ("OBS", change_word(change)),
        IntegrationInput::HomeAssistant { change } => ("Home Assistant", change_word(change)),
        IntegrationInput::Companion { change } => ("Companion", change_word(change)),
    };
    fn change_word<T>(c: &Change<T>) -> &'static str {
        match c {
            Change::Use(_) => "saved",
            Change::TurnOff => "turned off",
            Change::Remove => "removed",
        }
    }
    format!("{what} {name}")
}

impl Host {
    fn integrations_file(&self) -> PathBuf {
        integrations_path(&self.data_dir)
    }

    /// Run `f` on the file, one at a time, on a thread meant for blocking.
    async fn on_integrations_file<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Path, &Started) -> T + Send + 'static,
    ) -> Result<T, String> {
        let _one = self.integrations_lock.lock().await;
        let path = self.integrations_file();
        let started = self.integrations_started.clone();
        tokio::task::spawn_blocking(move || f(&path, &started))
            .await
            .map_err(|e| e.to_string())
    }

    /// What Settings → Integrations shows.
    pub async fn integrations_view(&self) -> Result<IntegrationsView, String> {
        self.on_integrations_file(read_view).await
    }

    /// Save one section of the form. The file is rewritten owner-only, keeping what the
    /// change doesn't touch; the new settings are used after a restart.
    pub async fn save_integration(
        &self,
        input: IntegrationInput,
    ) -> Result<IntegrationsView, String> {
        self.on_integrations_file(move |path, started| {
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(format!("Couldn't read {NAME}: {e}")),
            };
            let new = file::edit(&text, &input)?;
            Config::parse(&new)
                .map_err(|_| format!("Couldn't save {NAME}: the change didn't come out right."))?;
            private::write_atomic(path, new.as_bytes(), NAME)?;
            info!("integrations: {} from Settings", described(&input));
            Ok(read_view(path, started))
        })
        .await?
    }

    /// Make the file owner-only where it is.
    pub async fn protect_integrations_file(&self) -> Result<IntegrationsView, String> {
        self.on_integrations_file(|path, started| {
            private::restrict(path).map_err(|e| format!("Couldn't make {NAME} private: {e}"))?;
            info!("integrations: made {NAME} private");
            Ok(read_view(path, started))
        })
        .await?
    }

    /// Put a file with a mistake aside as `integrations.toml.old` (kept private, since it
    /// holds the passwords), so the form starts from nothing.
    pub async fn start_integrations_over(&self) -> Result<IntegrationsView, String> {
        self.on_integrations_file(|path, started| {
            if path.exists() {
                let old = path.with_file_name(format!("{NAME}.old"));
                std::fs::rename(path, &old)
                    .map_err(|e| format!("Couldn't rename {NAME} to {NAME}.old: {e}"))?;
                if let Err(e) = private::restrict(&old) {
                    warn!("couldn't make {NAME}.old private: {e}");
                }
                info!("integrations: {NAME} renamed to {NAME}.old to start over");
            }
            Ok(read_view(path, started))
        })
        .await?
    }

    /// The file, made from the commented template (owner-only) if there's none yet,
    /// for "Open the file".
    pub async fn ensure_integrations_file(&self) -> Result<PathBuf, String> {
        self.on_integrations_file(|path, _| {
            if !path.exists() {
                private::write_atomic(path, file::NEW_FILE_TEMPLATE.as_bytes(), NAME)?;
            }
            Ok(path.to_owned())
        })
        .await?
    }

    /// Test connection: the values in the form, saved or not, on the real clients'
    /// handshake. Never fails: what went wrong is the answer.
    pub async fn test_integration(&self, test: IntegrationTest) -> Checked {
        self.test_integration_with(test, check::TEST_WAIT, &check::system_resolver())
            .await
    }

    pub(crate) async fn test_integration_with(
        &self,
        test: IntegrationTest,
        wait: Duration,
        resolve: &Resolve,
    ) -> Checked {
        // A kept password or token is the one in the file now.
        let keeps = match &test {
            IntegrationTest::Obs(i) => (i.password == Secret::Keep).then_some("password"),
            IntegrationTest::HomeAssistant(i) => (i.token == Secret::Keep).then_some("token"),
            IntegrationTest::Companion(_) => None,
        };
        let saved = match keeps {
            None => Config::default(),
            Some(what) => match self
                .on_integrations_file(|path, _| Config::load(path))
                .await
            {
                Ok(Ok(c)) => c,
                Ok(Err(m)) => {
                    return Checked::failed(format!(
                        "{NAME} has a mistake, so the saved {what} can't be used: {m}"
                    ))
                }
                Err(e) => return Checked::failed(e),
            },
        };
        match test {
            IntegrationTest::Obs(i) => match file::resolve_obs(&i, saved.obs.as_ref()) {
                Ok(cfg) => {
                    let given = cfg.password.as_deref().is_some_and(|p| !p.is_empty());
                    check::obs(&cfg, given, wait, resolve).await
                }
                Err(m) => Checked::failed(m),
            },
            IntegrationTest::HomeAssistant(i) => {
                match file::resolve_ha(&i, saved.home_assistant.as_ref()) {
                    Ok(cfg) => check::home_assistant(&cfg, wait, resolve).await,
                    Err(m) => Checked::failed(m),
                }
            }
            IntegrationTest::Companion(i) => match file::resolve_companion(&i) {
                Ok(cfg) => check_companion(&cfg, wait, resolve).await,
                Err(m) => Checked::failed(m),
            },
        }
    }
}

/// Test Companion: connect to its Satellite port and read its greeting.
pub(crate) async fn check_companion(
    c: &CompanionConfig,
    wait: Duration,
    resolve: &Resolve,
) -> Checked {
    check::limited(async {
        let at = check::at(&c.host, c.port);
        if let Err(m) = check::find_host(&c.host, c.port, wait, resolve).await {
            return Checked::failed(m);
        }
        match perritodeck_companion::probe(&c.host, c.port, wait).await {
            Ok(g) => Checked::ok(format!(
                "Connected to Companion {} (Satellite API {}).",
                g.companion_version, g.api
            )),
            Err(e) => Checked::failed(companion_problem(&e, &at, c.port, wait)),
        }
    })
    .await
}

pub(crate) fn companion_problem(
    e: &perritodeck_companion::ProbeError,
    at: &str,
    port: u16,
    wait: Duration,
) -> String {
    use perritodeck_companion::ProbeError as P;
    use std::io::ErrorKind;
    match e {
        P::Io(io) if io.kind() == ErrorKind::ConnectionRefused => format!(
            "Nothing answered at {at}. Is Companion running there? Its Satellite port is 16622 \
             unless it was changed in Companion."
        ),
        P::Io(io) if io.kind() == ErrorKind::TimedOut => check::no_answer(at, wait),
        P::Io(io)
            if matches!(
                io.kind(),
                ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable
            ) =>
        {
            check::unreachable(at)
        }
        P::ConnectTimeout => check::no_answer(at, wait),
        P::Silent => {
            let mut m = format!(
                "Something at {at} took the connection but didn't greet like Companion within \
                 {}. Check the port: Companion's Satellite port is 16622, not the port of its \
                 web page.",
                check::secs(wait)
            );
            if port == 16623 {
                m.push_str(" (16623 is Companion's WebSocket port; PerritoDeck uses 16622.)");
            }
            m
        }
        P::Closed => format!(
            "{at} closed the connection without greeting. Check the port: Companion's \
             Satellite port is 16622."
        ),
        P::NotSatellite(word) => format!(
            "Something answered at {at}, but it isn't Companion's Satellite port (it said \
             “{word}”)."
        ),
        P::Io(e) => format!("Couldn't connect to Companion at {at}: {e}"),
        P::Lost(e) => format!("Couldn't connect to Companion at {at}: {e}"),
    }
}
