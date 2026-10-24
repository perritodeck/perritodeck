//! Profiles: what's on each page of the deck, how each key looks, and what it does.
//!
//! A profile is a TOML file (see [`STARTER_PROFILE`] for a commented example). Keys sit
//! at `[row, column]` on a page and either run an action, navigate between pages, or
//! cycle through states (a toggle), each state with its own look and action.

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

pub mod edit;
pub mod risk;

use perritodeck_actions::{ActionSpec, Executor, SliderSpec};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_GRID: u32 = 32;

/// The profile written on first run. Every option appears at least once, with comments.
pub const STARTER_PROFILE: &str = include_str!("starter.toml");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub name: String,
    /// Fixed grid for every tablet. Omit to size the grid to each tablet's screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<GridSpec>,
    /// Show Bitfocus Companion's buttons instead of this profile's own pages (needs
    /// `[companion]` in `integrations.toml`). Pages are then optional and unused.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub companion: bool,
    #[serde(default)]
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridSpec {
    pub cols: u32,
    pub rows: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub name: String,
    #[serde(default)]
    pub keys: Vec<KeyDef>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyDef {
    /// `[row, column]`, counted from 0 at the top left.
    pub at: [u32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// PNG file, relative to the profile file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<ActionSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nav: Option<NavSpec>,
    /// Two or more states make the key a toggle: each press runs the current state's
    /// action, then moves to the next state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<StateDef>,
    /// Makes the cell a slider (on tablets that support them) instead of a key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slider: Option<SliderSpec>,
    /// For a two-state toggle: show the second state while this live value is "on"
    /// (e.g. `"obs:recording"`, `"ha:light.desk"`) instead of flipping on each press.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<ActionSpec>,
}

/// `nav = "back"`, `"next"`, `"previous"`, `nav = { page = "Media" }`, or
/// `nav = { profile = "streaming" }` (optionally with a `page` in that profile).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NavSpec {
    Simple(SimpleNav),
    /// Switch the tablet it's pressed on to another profile (by id), like choosing it
    /// in the app. Listed before `Page` so `{ profile, page }` isn't read as a page.
    Profile {
        profile: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page: Option<String>,
    },
    Page {
        page: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SimpleNav {
    Back,
    Next,
    Previous,
}

/// How a key looks in one state, with defaults filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    pub title: String,
    pub icon: Option<PathBuf>,
    pub bg: String,
    pub fg: String,
}

pub const DEFAULT_BG: &str = "#1E1E2E";
pub const DEFAULT_FG: &str = "#CDD6F4";
pub const EMPTY_BG: &str = "#000000";

impl KeyDef {
    /// Number of states (1 for an ordinary key).
    pub fn state_count(&self) -> usize {
        self.states.len().max(1)
    }

    /// The look for `state`; state fields fall back to the key's own fields.
    pub fn look(&self, state: usize) -> Look {
        let s = self.states.get(state % self.state_count());
        let pick = |state_v: Option<&String>, key_v: &Option<String>, default: &str| {
            state_v
                .or(key_v.as_ref())
                .cloned()
                .unwrap_or_else(|| default.to_owned())
        };
        Look {
            title: pick(s.and_then(|s| s.title.as_ref()), &self.title, ""),
            icon: s.and_then(|s| s.icon.clone()).or_else(|| self.icon.clone()),
            bg: pick(s.and_then(|s| s.bg.as_ref()), &self.bg, DEFAULT_BG),
            fg: pick(s.and_then(|s| s.fg.as_ref()), &self.fg, DEFAULT_FG),
        }
    }

    /// The action to run when pressed in `state`.
    pub fn action(&self, state: usize) -> Option<&ActionSpec> {
        self.states
            .get(state % self.state_count())
            .and_then(|s| s.action.as_ref())
            .or(self.action.as_ref())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} isn't valid: {message}")]
    Syntax { path: PathBuf, message: String },
    #[error("{path} has problems:\n  - {}", problems.join("\n  - "))]
    Invalid {
        path: PathBuf,
        problems: Vec<String>,
    },
}

impl Profile {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn page_index(&self, name: &str) -> Option<usize> {
        self.pages.iter().position(|p| p.name == name)
    }

    /// Every problem, in a form a person can fix. Empty means the profile is usable.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.schema_version != SCHEMA_VERSION {
            out.push(format!(
                "schema_version is {}, this host understands {SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.pages.is_empty() && !self.companion {
            out.push("a profile needs at least one [[pages]] entry".into());
        }
        if let Some(g) = self.grid {
            if !(1..=MAX_GRID).contains(&g.cols) || !(1..=MAX_GRID).contains(&g.rows) {
                out.push(format!(
                    "grid must be between 1×1 and {MAX_GRID}×{MAX_GRID}"
                ));
            }
        }
        let mut names = HashSet::new();
        for page in &self.pages {
            if !names.insert(page.name.as_str()) {
                out.push(format!(
                    "two pages are called {:?}; page names must be unique",
                    page.name
                ));
            }
        }
        for page in &self.pages {
            let mut taken = BTreeMap::new();
            for key in &page.keys {
                let here = format!("page {:?}, key at {:?}", page.name, key.at);
                let [row, col] = key.at;
                let (max_r, max_c) = self
                    .grid
                    .map(|g| (g.rows, g.cols))
                    .unwrap_or((MAX_GRID, MAX_GRID));
                if row >= max_r || col >= max_c {
                    out.push(format!(
                        "{here}: outside the {max_c}×{max_r} grid (positions count from 0)"
                    ));
                }
                if taken.insert(key.at, ()).is_some() {
                    out.push(format!("{here}: another key is already there"));
                }
                let kinds = [
                    key.action.is_some(),
                    key.nav.is_some(),
                    !key.states.is_empty(),
                ];
                if key.nav.is_some() && kinds.iter().filter(|k| **k).count() > 1 {
                    out.push(format!(
                        "{here}: a navigation key can't also have an action or states"
                    ));
                }
                if key.slider.is_some() && kinds.iter().any(|k| *k) {
                    out.push(format!(
                        "{here}: a slider can't also have an action, page or states"
                    ));
                }
                if key.follow.is_some() && key.states.len() != 2 {
                    out.push(format!(
                        "{here}: \"follow\" needs a toggle with exactly two states (off, then on)"
                    ));
                }
                if key.states.len() == 1 {
                    out.push(format!("{here}: a toggle needs at least two states"));
                }
                match &key.nav {
                    Some(NavSpec::Page { page: target }) if self.page_index(target).is_none() => {
                        out.push(format!("{here}: there is no page called {target:?}"));
                    }
                    // Other profiles load on their own, so only the shape is checked here;
                    // a missing profile or page shows an alert when the key is pressed.
                    Some(NavSpec::Profile { profile, page }) => {
                        if profile.trim().is_empty() {
                            out.push(format!("{here}: say which profile to switch to"));
                        }
                        if page.as_deref().is_some_and(|p| p.trim().is_empty()) {
                            out.push(format!("{here}: the page to open is empty"));
                        }
                    }
                    _ => {}
                }
                for c in [&key.bg, &key.fg].into_iter().flatten() {
                    if !is_color(c) {
                        out.push(format!("{here}: {c:?} isn't a colour like \"#1E1E2E\""));
                    }
                }
                for (i, s) in key.states.iter().enumerate() {
                    for c in [&s.bg, &s.fg].into_iter().flatten() {
                        if !is_color(c) {
                            out.push(format!(
                                "{here}, state {}: {c:?} isn't a colour like \"#1E1E2E\"",
                                i + 1
                            ));
                        }
                    }
                }
                let actions = key
                    .action
                    .iter()
                    .chain(key.states.iter().filter_map(|s| s.action.as_ref()));
                for a in actions {
                    if let Err(e) = Executor::validate(a) {
                        out.push(format!("{here}: {e}"));
                    }
                }
            }
        }
        out
    }

    /// Icon paths used anywhere, resolved against `base_dir`, that don't exist.
    pub fn missing_icons(&self, base_dir: &Path) -> Vec<PathBuf> {
        let mut missing = Vec::new();
        for page in &self.pages {
            for key in &page.keys {
                let icons = key
                    .icon
                    .iter()
                    .chain(key.states.iter().filter_map(|s| s.icon.as_ref()));
                for icon in icons {
                    let p = base_dir.join(icon);
                    if !p.exists() && !missing.contains(&p) {
                        missing.push(p);
                    }
                }
            }
        }
        missing
    }
}

/// Load and validate a profile file.
pub fn load(path: &Path) -> Result<Profile, ProfileError> {
    let text = std::fs::read_to_string(path).map_err(|source| ProfileError::Read {
        path: path.to_owned(),
        source,
    })?;
    let profile = Profile::parse(&text).map_err(|message| ProfileError::Syntax {
        path: path.to_owned(),
        message,
    })?;
    let problems = profile.problems();
    if !problems.is_empty() {
        return Err(ProfileError::Invalid {
            path: path.to_owned(),
            problems,
        });
    }
    Ok(profile)
}

/// Write the starter profile to `path` if nothing is there yet. Returns true if written.
pub fn ensure_starter(path: &Path) -> std::io::Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, STARTER_PROFILE)?;
    Ok(true)
}

pub fn is_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests;
