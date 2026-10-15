//! What a key can do.
//!
//! An [`ActionSpec`] is pure data (it will live in profile files). The [`Executor`]
//! runs it against two narrow backends — [`InputBackend`] for synthetic keyboard
//! input and [`SystemBackend`] for opening things — so everything except the real OS
//! calls is testable headlessly.

mod hotkey;
pub mod job;
mod launch;
pub mod slider;
#[cfg(feature = "system")]
pub mod system;

use std::{path::PathBuf, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::debug;

pub use hotkey::{Chord, HotkeyParseError, Key, Modifier};
pub use slider::SliderSpec;

/// Upper bound on a single delay inside a multi-action, so a typo can't hang a key for hours.
pub const MAX_DELAY_MS: u64 = 60_000;
/// Default and maximum run time for `command` actions.
pub const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 10_000;
pub const MAX_COMMAND_TIMEOUT_MS: u64 = 10 * 60_000;
/// Multi- and random actions may nest, but not without limit.
pub const MAX_NESTING: usize = 4;
/// The most characters a `text` action types. Plenty for a signature, a canned reply or a
/// short snippet, and still a bounded wait: on Windows, where text goes a character every
/// 15 ms, it takes 30 s, can't be stopped, and other keys' typing and hotkeys wait for it.
pub const MAX_TEXT_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ActionSpec {
    /// e.g. `{"type":"hotkey","keys":"Ctrl+Shift+M"}`
    Hotkey {
        keys: String,
    },
    /// Types the text exactly as given.
    Text {
        text: String,
    },
    Media {
        key: MediaKey,
    },
    /// Opens a URL in the default browser (http, https and mailto only).
    OpenUrl {
        url: String,
    },
    /// Opens a file or folder with its default app.
    OpenPath {
        path: PathBuf,
    },
    /// Starts a program and returns immediately. Never goes through a shell.
    Launch {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Runs a program, waits for it (with a timeout) and reports success or failure.
    /// Never goes through a shell: `program` + `args` are passed as an argv array.
    Command {
        program: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "timeoutMs")]
        timeout_ms: Option<u64>,
    },
    /// Runs steps in order, with an optional delay after each.
    Multi {
        steps: Vec<Step>,
    },
    /// Runs one of its actions, picked at random on each press.
    Random {
        actions: Vec<ActionSpec>,
    },
    /// Controls OBS Studio through obs-websocket (configured in `integrations.toml`).
    Obs(ObsAction),
    /// Calls a Home Assistant service, e.g. `light.toggle` (configured in `integrations.toml`).
    HomeAssistant {
        /// `domain.service`, e.g. `light.toggle`, `scene.turn_on`, `script.good_night`.
        service: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entity: Option<String>,
        /// Extra service data, e.g. `{ brightness_pct = 40 }`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
    /// Runs an action from an installed plugin.
    Plugin {
        plugin: String,
        action: String,
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        settings: serde_json::Value,
    },
    /// An action from a Stream Deck plugin (`.sdPlugin`). The plugin draws the key and
    /// gets its presses, so this only works on a key, not in "Several steps".
    SdPlugin {
        /// The plugin's UUID, e.g. `com.example.counter`.
        plugin: String,
        /// The action's UUID, e.g. `com.example.counter.increment`.
        action: String,
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        settings: serde_json::Value,
    },
    /// Does nothing (an empty key).
    None,
}

/// What to do in OBS. `{ type = "obs", op = "scene", scene = "Gameplay" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum ObsAction {
    /// Switch the program scene.
    Scene {
        scene: String,
    },
    ToggleRecord,
    StartRecord,
    StopRecord,
    ToggleStream,
    StartStream,
    StopStream,
    /// Mute/unmute an audio input, e.g. "Mic/Aux".
    ToggleMute {
        input: String,
    },
    ToggleReplayBuffer,
    SaveReplayBuffer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    pub action: ActionSpec,
    #[serde(default)]
    pub delay_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKey {
    PlayPause,
    Next,
    Previous,
    Stop,
    VolumeUp,
    VolumeDown,
    Mute,
}

/// How an action went; becomes `showOk` / `showAlert` on the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    /// The action ran but reported a problem (e.g. a command exited non-zero).
    Alert(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error(transparent)]
    Hotkey(#[from] HotkeyParseError),
    #[error("{0} is not supported on this platform")]
    Unsupported(String),
    #[error("refusing to open {0:?}: only http, https and mailto URLs are allowed")]
    UrlScheme(String),
    #[error("multi-actions are nested more than {MAX_NESTING} deep")]
    TooDeep,
    #[error("a random action needs at least one action to pick from")]
    NothingToPick,
    #[error("delay of {0} ms exceeds the {MAX_DELAY_MS} ms limit")]
    DelayTooLong(u64),
    #[error("timeout of {0} ms exceeds the {MAX_COMMAND_TIMEOUT_MS} ms limit")]
    TimeoutTooLong(u64),
    #[error("text of {0} characters exceeds the {MAX_TEXT_CHARS} character limit")]
    TextTooLong(usize),
    #[error("input failed: {0}")]
    Input(String),
    // Quoted plainly, not with `{:?}`, which doubles every backslash in a Windows path.
    #[error("could not start \"{program}\": {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} isn't set up; add it to integrations.toml in the host's settings folder")]
    NotConfigured(String),
    /// integrations.toml has a mistake, so the host ignored all of it.
    #[error("integrations.toml has a mistake, so {service} is off: {mistake}")]
    IntegrationsMistake { service: String, mistake: String },
    #[error("{}", integration_message(.service, .reason))]
    Integration { service: String, reason: String },
    #[error("could not open \"{target}\": {reason}")]
    Open { target: String, reason: String },
}

/// "OBS: already recording", but a reason that names the service itself reads on its
/// own: "OBS didn't answer in time", not "OBS: OBS didn't answer in time".
fn integration_message(service: &str, reason: &str) -> String {
    let names_it = reason
        .strip_prefix(service)
        .is_some_and(|rest| rest.starts_with([' ', '\'', '’']));
    if names_it {
        reason.to_owned()
    } else {
        format!("{service}: {reason}")
    }
}

/// Synthetic keyboard input.
pub trait InputBackend: Send + Sync {
    fn chord(&self, chord: &Chord) -> Result<(), ActionError>;
    fn text(&self, text: &str) -> Result<(), ActionError>;
    fn media(&self, key: MediaKey) -> Result<(), ActionError>;
}

/// Opening URLs and files with the user's default apps.
pub trait SystemBackend: Send + Sync {
    fn open(&self, target: &str) -> Result<(), ActionError>;

    /// Set the main output volume, 0–100.
    fn set_volume(&self, _percent: u8) -> Result<(), ActionError> {
        Err(ActionError::Unsupported("setting the volume".into()))
    }

    /// The main output volume, 0–100, if it can be read.
    fn volume(&self) -> Option<u8> {
        None
    }
}

pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Apps the host talks to on the user's behalf (OBS, Home Assistant, …).
pub trait Integrations: Send + Sync {
    fn obs<'a>(&'a self, action: &'a ObsAction) -> BoxFuture<'a, Result<Outcome, ActionError>>;
    fn home_assistant<'a>(
        &'a self,
        service: &'a str,
        entity: Option<&'a str>,
        data: Option<&'a serde_json::Value>,
    ) -> BoxFuture<'a, Result<Outcome, ActionError>>;

    /// Send any obs-websocket request; resolves to its `responseData`.
    fn obs_request<'a>(
        &'a self,
        _request_type: &'a str,
        _data: serde_json::Value,
    ) -> BoxFuture<'a, Result<serde_json::Value, ActionError>> {
        Box::pin(async { Err(ActionError::NotConfigured("OBS".into())) })
    }

    /// An entity's state object from Home Assistant (`state`, `attributes`, …).
    fn ha_state<'a>(
        &'a self,
        _entity: &'a str,
    ) -> BoxFuture<'a, Result<serde_json::Value, ActionError>> {
        Box::pin(async { Err(ActionError::NotConfigured("Home Assistant".into())) })
    }
}

/// Installed plugins (separate programs; see plugins/PROTOCOL.md).
pub trait PluginRunner: Send + Sync {
    fn run_plugin<'a>(
        &'a self,
        plugin: &'a str,
        action: &'a str,
        settings: &'a serde_json::Value,
    ) -> BoxFuture<'a, Result<Outcome, ActionError>>;
}

#[derive(Clone)]
pub struct Executor {
    plugins: Option<Arc<dyn PluginRunner>>,
    input: Arc<dyn InputBackend>,
    pub(crate) system: Arc<dyn SystemBackend>,
    pub(crate) integrations: Option<Arc<dyn Integrations>>,
}

impl Executor {
    pub fn new(input: Arc<dyn InputBackend>, system: Arc<dyn SystemBackend>) -> Self {
        Self {
            input,
            system,
            integrations: None,
            plugins: None,
        }
    }

    pub fn with_plugins(mut self, plugins: Arc<dyn PluginRunner>) -> Self {
        self.plugins = Some(plugins);
        self
    }

    pub fn with_integrations(mut self, integrations: Arc<dyn Integrations>) -> Self {
        self.integrations = Some(integrations);
        self
    }

    /// The integrations, for reading state (live values on keys).
    pub fn integrations_handle(&self) -> Option<Arc<dyn Integrations>> {
        self.integrations.clone()
    }

    pub(crate) fn integrations(&self, what: &str) -> Result<&Arc<dyn Integrations>, ActionError> {
        self.integrations
            .as_ref()
            .ok_or_else(|| ActionError::NotConfigured(what.to_owned()))
    }

    /// Check an action without running it (used when loading profiles).
    pub fn validate(spec: &ActionSpec) -> Result<(), ActionError> {
        validate_at(spec, 0)
    }

    pub async fn run(&self, spec: &ActionSpec) -> Result<Outcome, ActionError> {
        Self::validate(spec)?;
        self.run_at(spec).await
    }

    fn run_at<'a>(
        &'a self,
        spec: &'a ActionSpec,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Outcome, ActionError>> + Send + 'a>,
    > {
        Box::pin(async move {
            debug!(?spec, "running action");
            match spec {
                ActionSpec::Hotkey { keys } => {
                    self.input.chord(&keys.parse()?)?;
                    Ok(Outcome::Ok)
                }
                ActionSpec::Text { text } => {
                    self.input.text(text)?;
                    Ok(Outcome::Ok)
                }
                ActionSpec::Media { key } => {
                    self.input.media(*key)?;
                    Ok(Outcome::Ok)
                }
                ActionSpec::OpenUrl { url } => {
                    self.system.open(url)?;
                    Ok(Outcome::Ok)
                }
                ActionSpec::OpenPath { path } => {
                    let target = launch::unquote(&path.to_string_lossy()).to_owned();
                    open_path(self.system.clone(), target).await?;
                    Ok(Outcome::Ok)
                }
                ActionSpec::Launch { program, args } => {
                    let program = launch::unquote(program);
                    let build = |p: &std::ffi::OsStr| {
                        let mut cmd = Command::new(p);
                        cmd.args(args);
                        #[cfg(not(windows))]
                        cmd.stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null());
                        #[cfg(windows)]
                        {
                            // Start in the program's own folder, like an Explorer shortcut.
                            if let Some(dir) = launch::program_dir(program) {
                                cmd.current_dir(dir);
                            }
                            launch::set_console(&mut cmd, program, p);
                        }
                        cmd
                    };
                    match spawn_found(program, build) {
                        Ok(mut child) => {
                            // Reap it in the background so it never becomes a zombie.
                            tokio::spawn(async move {
                                let _ = child.wait().await;
                            });
                            Ok(Outcome::Ok)
                        }
                        // App Paths names, shortcuts, documents, links and programs
                        // that need administrator rights.
                        #[cfg(all(windows, feature = "system"))]
                        Err(source) if launch::shell_can_try(program, &source) => {
                            match launch::shell_execute(program, args).await {
                                Ok(()) => Ok(Outcome::Ok),
                                Err(code) => {
                                    debug!(code, "ShellExecute couldn't start {program} either");
                                    Err(ActionError::Spawn {
                                        program: program.to_owned(),
                                        source: launch::shell_error(code, source),
                                    })
                                }
                            }
                        }
                        Err(source) => Err(ActionError::Spawn {
                            program: program.to_owned(),
                            source,
                        }),
                    }
                }
                ActionSpec::Command {
                    program,
                    args,
                    cwd,
                    timeout_ms,
                } => run_command(program, args, cwd.as_deref(), *timeout_ms).await,
                ActionSpec::Multi { steps } => {
                    let mut outcome = Outcome::Ok;
                    for step in steps {
                        // Keep going after an alert (like Stream Deck), but stop on hard errors.
                        if let Outcome::Alert(a) = self.run_at(&step.action).await? {
                            outcome = Outcome::Alert(a);
                        }
                        if step.delay_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(step.delay_ms)).await;
                        }
                    }
                    Ok(outcome)
                }
                ActionSpec::Random { actions } => {
                    // Validation guarantees at least one.
                    let pick = &actions[rand::random_range(0..actions.len())];
                    self.run_at(pick).await
                }
                ActionSpec::Obs(action) => self.integrations("OBS")?.obs(action).await,
                ActionSpec::HomeAssistant {
                    service,
                    entity,
                    data,
                } => {
                    self.integrations("Home Assistant")?
                        .home_assistant(service, entity.as_deref(), data.as_ref())
                        .await
                }
                ActionSpec::Plugin {
                    plugin,
                    action,
                    settings,
                } => {
                    let plugins = self
                        .plugins
                        .as_ref()
                        .ok_or_else(|| ActionError::NotConfigured("Plugins".into()))?;
                    plugins.run_plugin(plugin, action, settings).await
                }
                ActionSpec::SdPlugin { .. } => Err(ActionError::Integration {
                    service: "Stream Deck plugins".into(),
                    reason: "this action works on a key only; press the key on the tablet".into(),
                }),
                ActionSpec::None => Ok(Outcome::Ok),
            }
        })
    }
}

fn validate_at(spec: &ActionSpec, depth: usize) -> Result<(), ActionError> {
    match spec {
        ActionSpec::Plugin { plugin, action, .. } => {
            let ok = |id: &str| {
                !id.is_empty()
                    && id.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_')
                    })
            };
            if !ok(plugin) || !ok(action) {
                return Err(ActionError::Integration {
                    service: "Plugins".into(),
                    reason: format!("{plugin:?} / {action:?} isn't a plugin action id"),
                });
            }
        }
        ActionSpec::Hotkey { keys } => {
            keys.parse::<Chord>()?;
        }
        ActionSpec::Text { text } => {
            let chars = text.chars().count();
            if chars > MAX_TEXT_CHARS {
                return Err(ActionError::TextTooLong(chars));
            }
        }
        ActionSpec::OpenUrl { url } => check_url(url)?,
        ActionSpec::HomeAssistant { service, .. } => {
            let ok = service.split_once('.').is_some_and(|(d, s)| {
                !d.is_empty()
                    && !s.is_empty()
                    && service
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
            });
            if !ok {
                return Err(ActionError::Integration {
                    service: "Home Assistant".into(),
                    reason: format!("{service:?} isn't a service like \"light.toggle\""),
                });
            }
        }
        ActionSpec::Command {
            timeout_ms: Some(t),
            ..
        } if *t > MAX_COMMAND_TIMEOUT_MS => {
            return Err(ActionError::TimeoutTooLong(*t));
        }
        ActionSpec::Multi { steps } => {
            if depth >= MAX_NESTING {
                return Err(ActionError::TooDeep);
            }
            for s in steps {
                if s.delay_ms > MAX_DELAY_MS {
                    return Err(ActionError::DelayTooLong(s.delay_ms));
                }
                validate_at(&s.action, depth + 1)?;
            }
        }
        ActionSpec::Random { actions } => {
            if depth >= MAX_NESTING {
                return Err(ActionError::TooDeep);
            }
            if actions.is_empty() {
                return Err(ActionError::NothingToPick);
            }
            for a in actions {
                validate_at(a, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn check_url(url: &str) -> Result<(), ActionError> {
    let lower = url.trim().to_ascii_lowercase();
    if ["http://", "https://", "mailto:"]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        Ok(())
    } else {
        Err(ActionError::UrlScheme(url.to_owned()))
    }
}

/// Open a file or folder with its app. A missing one is an error on the key: Windows
/// would show a "cannot find" box and wait for someone to click OK. Only absolute paths
/// are checked, so links and `shell:` folders still reach the shell. It runs on a thread
/// of its own: the shell can be slow (a network folder), and opening a folder leaves
/// COM set up on the thread that did it.
async fn open_path(system: Arc<dyn SystemBackend>, target: String) -> Result<(), ActionError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let shown = target.clone();
    std::thread::spawn(move || {
        let path = std::path::Path::new(&target);
        let missing = path.is_absolute() && matches!(path.try_exists(), Ok(false));
        let _ = tx.send(if missing {
            Err(ActionError::Open {
                target,
                reason: "it doesn't exist".into(),
            })
        } else {
            system.open(&target)
        });
    });
    rx.await.unwrap_or_else(|_| {
        Err(ActionError::Open {
            target: shown,
            reason: "the system's opener crashed".into(),
        })
    })
}

/// Spawn `program`; on Windows, when it isn't found as given, retry with the PATHEXT
/// names Windows users expect (`code` is `code.cmd`).
fn spawn_found(
    program: &str,
    build: impl Fn(&std::ffi::OsStr) -> Command,
) -> std::io::Result<tokio::process::Child> {
    let first = build(std::ffi::OsStr::new(program)).spawn();
    #[cfg(windows)]
    if matches!(&first, Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
        let path = std::env::var_os("PATH");
        let pathext = std::env::var_os("PATHEXT");
        if let Some(found) = launch::search_path(program, path.as_deref(), pathext.as_deref()) {
            return build(found.as_os_str()).spawn();
        }
    }
    first
}

pub(crate) async fn run_command(
    program: &str,
    args: &[String],
    cwd: Option<&std::path::Path>,
    timeout_ms: Option<u64>,
) -> Result<Outcome, ActionError> {
    let program = launch::unquote(program);
    // `.\tools\build.exe` with a working folder is the one in that folder, everywhere.
    let in_cwd = launch::in_folder(program, cwd);
    let program = in_cwd.as_deref().unwrap_or(program);
    let build = |p: &std::ffi::OsStr| {
        let mut cmd = Command::new(p);
        // Only the error output is used, for the alert.
        cmd.args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        // Output is captured, so a console window would only flash up empty (the app is
        // a GUI process with no console of its own).
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        cmd
    };
    let mut child = spawn_found(program, build).map_err(|source| ActionError::Spawn {
        program: program.to_owned(),
        source,
    })?;
    #[cfg(windows)]
    let tree = launch::CommandTree::adopt(&child);
    let stderr = child.stderr.take().and_then(|e| ErrorOutput::read(e).ok());
    let limit = Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_COMMAND_TIMEOUT_MS));
    // The program's exit is what counts, not its output closing: a program it started
    // (`cmd /c start "" notepad`) inherits the pipe and can hold it open for hours.
    let status = match tokio::time::timeout(limit, child.wait()).await {
        Err(_) => {
            // Dropping the child ends it; on Windows the tree ends what it started too.
            #[cfg(windows)]
            if let Some(tree) = &tree {
                tree.end();
            }
            return Ok(Outcome::Alert(format!(
                "timed out after {} s",
                limit.as_secs_f32()
            )));
        }
        Ok(Err(e)) => return Ok(Outcome::Alert(format!("failed: {e}"))),
        Ok(Ok(status)) => status,
    };
    if status.success() {
        return Ok(Outcome::Ok);
    }
    let stderr = match stderr {
        Some(e) => e.text(Duration::from_millis(250)).await,
        None => String::new(),
    };
    let first = stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let code = status
        .code()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "signal".into());
    Ok(Outcome::Alert(if first.is_empty() {
        format!("exit {code}")
    } else {
        format!("exit {code}: {first}")
    }))
}

/// A command's error output, read on a thread of its own. The alert shows only the
/// first line, so only the first few KiB are kept; the rest is read and dropped, so a
/// chatty program never stalls on a full pipe. Not tokio's pipe reader: when a program
/// the command started holds the pipe open, the read waits for it, and tokio's runtime
/// waits for such reads when it shuts down, so quitting the host would hang.
struct ErrorOutput {
    kept: Arc<std::sync::Mutex<Vec<u8>>>,
    done: tokio::sync::oneshot::Receiver<()>,
}

impl ErrorOutput {
    const KEEP: usize = 4096;

    fn read(pipe: tokio::process::ChildStderr) -> std::io::Result<ErrorOutput> {
        use std::io::Read;
        #[cfg(windows)]
        let mut file = std::fs::File::from(pipe.into_owned_handle()?);
        #[cfg(unix)]
        let mut file = std::fs::File::from(pipe.into_owned_fd()?);
        let kept = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (tx, done) = tokio::sync::oneshot::channel();
        let keep = kept.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n @ 1..) = file.read(&mut buf) {
                let mut kept = keep.lock().unwrap_or_else(|e| e.into_inner());
                let room = Self::KEEP.saturating_sub(kept.len());
                kept.extend_from_slice(&buf[..n.min(room)]);
            }
            let _ = tx.send(());
        });
        Ok(ErrorOutput { kept, done })
    }

    /// What has arrived, after waiting up to `grace` for the rest: whatever the program
    /// wrote before it exited is still in the pipe.
    async fn text(self, grace: Duration) -> String {
        let _ = tokio::time::timeout(grace, self.done).await;
        let kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&kept).into_owned()
    }
}

#[cfg(test)]
mod tests;
