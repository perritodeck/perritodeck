//! What a key can do.
//!
//! An [`ActionSpec`] is pure data (it will live in profile files). The [`Executor`]
//! runs it against two narrow backends — [`InputBackend`] for synthetic keyboard
//! input and [`SystemBackend`] for opening things — so everything except the real OS
//! calls is testable headlessly.

mod hotkey;
#[cfg(feature = "system")]
pub mod system;

use std::{path::PathBuf, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::debug;

pub use hotkey::{Chord, HotkeyParseError, Key, Modifier};

/// Upper bound on a single delay inside a multi-action, so a typo can't hang a key for hours.
pub const MAX_DELAY_MS: u64 = 60_000;
/// Default and maximum run time for `command` actions.
pub const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 10_000;
pub const MAX_COMMAND_TIMEOUT_MS: u64 = 10 * 60_000;
/// Multi-actions may nest, but not without limit.
pub const MAX_NESTING: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Does nothing (an empty key).
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[error("delay of {0} ms exceeds the {MAX_DELAY_MS} ms limit")]
    DelayTooLong(u64),
    #[error("timeout of {0} ms exceeds the {MAX_COMMAND_TIMEOUT_MS} ms limit")]
    TimeoutTooLong(u64),
    #[error("input failed: {0}")]
    Input(String),
    #[error("could not start {program:?}: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not open {target:?}: {reason}")]
    Open { target: String, reason: String },
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
}

#[derive(Clone)]
pub struct Executor {
    input: Arc<dyn InputBackend>,
    system: Arc<dyn SystemBackend>,
}

impl Executor {
    pub fn new(input: Arc<dyn InputBackend>, system: Arc<dyn SystemBackend>) -> Self {
        Self { input, system }
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
                    self.system.open(&path.to_string_lossy())?;
                    Ok(Outcome::Ok)
                }
                ActionSpec::Launch { program, args } => {
                    let mut cmd = Command::new(program);
                    cmd.args(args)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null());
                    let mut child = cmd.spawn().map_err(|source| ActionError::Spawn {
                        program: program.clone(),
                        source,
                    })?;
                    // Reap it in the background so it never becomes a zombie.
                    tokio::spawn(async move {
                        let _ = child.wait().await;
                    });
                    Ok(Outcome::Ok)
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
                ActionSpec::None => Ok(Outcome::Ok),
            }
        })
    }
}

fn validate_at(spec: &ActionSpec, depth: usize) -> Result<(), ActionError> {
    match spec {
        ActionSpec::Hotkey { keys } => {
            keys.parse::<Chord>()?;
        }
        ActionSpec::OpenUrl { url } => check_url(url)?,
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

async fn run_command(
    program: &str,
    args: &[String],
    cwd: Option<&std::path::Path>,
    timeout_ms: Option<u64>,
) -> Result<Outcome, ActionError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let child = cmd.spawn().map_err(|source| ActionError::Spawn {
        program: program.to_owned(),
        source,
    })?;
    let limit = Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_COMMAND_TIMEOUT_MS));
    match tokio::time::timeout(limit, child.wait_with_output()).await {
        Err(_) => Ok(Outcome::Alert(format!(
            "timed out after {} s",
            limit.as_secs_f32()
        ))),
        Ok(Err(e)) => Ok(Outcome::Alert(format!("failed: {e}"))),
        Ok(Ok(out)) if out.status.success() => Ok(Outcome::Ok),
        Ok(Ok(out)) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let first = stderr
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            let code = out
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into());
            Ok(Outcome::Alert(if first.is_empty() {
                format!("exit {code}")
            } else {
                format!("exit {code}: {first}")
            }))
        }
    }
}

#[cfg(test)]
mod tests;
