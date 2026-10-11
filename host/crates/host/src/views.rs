//! What the desktop app shows: serializable views of profiles and keys.

use perritodeck_profile::{edit::KeyFields, NavSpec, Profile, SimpleNav};
use serde::Serialize;

/// One profile in the profile list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    pub pages: usize,
    /// Shown on tablets that haven't been given another profile.
    pub is_default: bool,
    /// Its file has a mistake; the last good version is in use.
    pub has_error: bool,
}

/// A read-only view of the profile for UIs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    /// The file name without `.toml`; stable while the profile exists.
    pub id: String,
    pub name: String,
    pub cols: u32,
    pub rows: u32,
    /// True when the profile fixes the grid size; otherwise it follows each tablet.
    pub fixed_grid: bool,
    /// Shows Bitfocus Companion's buttons; `pages` is then unused.
    pub companion: bool,
    pub pages: Vec<PageView>,
    pub reload_error: Option<String>,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageView {
    pub name: String,
    pub keys: Vec<KeyView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyView {
    pub row: u32,
    pub col: u32,
    pub title: String,
    pub bg: String,
    pub fg: String,
    /// "action", "page", "back", "next", "previous", "toggle", "slider" or "label".
    pub kind: String,
    /// Short description of what it does, e.g. "hotkey Ctrl+C".
    pub detail: String,
    /// The key's own settings, as the editor changes them.
    pub fields: KeyFields,
    /// Toggle states (0 for an ordinary key). Toggles are edited in the file for now.
    pub states: usize,
    pub has_icon: bool,
    /// The key's image (first state) as a PNG data URL, for the editor's preview.
    pub icon_data: Option<String>,
}

pub fn view_of(id: &str, profile: &Profile, reload_error: Option<String>) -> ProfileView {
    view_with_icons(id, profile, reload_error, None)
}

/// Like [`view_of`], with key images read from `base_dir` for the preview.
pub fn view_with_icons(
    id: &str,
    profile: &Profile,
    reload_error: Option<String>,
    base_dir: Option<&std::path::Path>,
) -> ProfileView {
    let (cols, rows) = profile.grid.map(|g| (g.cols, g.rows)).unwrap_or_else(|| {
        // Sized per tablet; preview the most common tablet grid.
        let max_c = profile
            .pages
            .iter()
            .flat_map(|p| p.keys.iter().map(|k| k.at[1] + 1))
            .max()
            .unwrap_or(1);
        let max_r = profile
            .pages
            .iter()
            .flat_map(|p| p.keys.iter().map(|k| k.at[0] + 1))
            .max()
            .unwrap_or(1);
        (max_c.max(5), max_r.max(3))
    });
    let pages = profile
        .pages
        .iter()
        .map(|p| PageView {
            name: p.name.clone(),
            keys: p
                .keys
                .iter()
                .map(|k| {
                    let look = k.look(0);
                    let (kind, detail) = match (&k.nav, k.states.len() >= 2, k.action(0)) {
                        _ if k.slider.is_some() => (
                            "slider",
                            format!(
                                "slider: {}",
                                k.slider.as_ref().map(|s| s.describe()).unwrap_or_default()
                            ),
                        ),
                        (Some(NavSpec::Page { page }), _, _) => ("page", format!("opens {page}")),
                        (Some(NavSpec::Simple(SimpleNav::Back)), _, _) => {
                            ("back", "back".to_owned())
                        }
                        (Some(NavSpec::Simple(SimpleNav::Next)), _, _) => {
                            ("next", "next page".to_owned())
                        }
                        (Some(NavSpec::Simple(SimpleNav::Previous)), _, _) => {
                            ("previous", "previous page".to_owned())
                        }
                        (None, true, a) => (
                            "toggle",
                            format!(
                                "{} states{}",
                                k.states.len(),
                                a.map(|a| format!(", {}", describe(a))).unwrap_or_default()
                            ),
                        ),
                        (None, false, Some(a)) => ("action", describe(a)),
                        (None, false, None) => ("label", "no action".to_owned()),
                    };
                    KeyView {
                        row: k.at[0],
                        col: k.at[1],
                        title: look.title,
                        bg: look.bg,
                        fg: look.fg,
                        kind: kind.to_owned(),
                        detail,
                        fields: KeyFields {
                            title: k.title.clone(),
                            bg: k.bg.clone(),
                            fg: k.fg.clone(),
                            action: k.action.clone(),
                            nav: k.nav.clone(),
                            icon: k.icon.clone(),
                            slider: k.slider.clone(),
                            states: k.states.clone(),
                            follow: k.follow.clone(),
                        },
                        states: k.states.len(),
                        has_icon: k.icon.is_some() || k.states.iter().any(|s| s.icon.is_some()),
                        icon_data: look.icon.as_ref().zip(base_dir).and_then(|(icon, dir)| {
                            use base64::Engine;
                            let bytes = std::fs::read(dir.join(icon)).ok()?;
                            if !perritodeck_render::is_gif(&bytes) {
                                return Some(perritodeck_render::data_url(&bytes));
                            }
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                            Some(format!("data:image/gif;base64,{b64}"))
                        }),
                    }
                })
                .collect(),
        })
        .collect();
    ProfileView {
        id: id.to_owned(),
        name: profile.name.clone(),
        cols,
        rows,
        fixed_grid: profile.grid.is_some(),
        companion: profile.companion,
        pages,
        reload_error,
        can_undo: false,
        can_redo: false,
    }
}

pub fn describe(a: &perritodeck_actions::ActionSpec) -> String {
    use perritodeck_actions::ActionSpec as A;
    match a {
        A::Hotkey { keys } => format!("hotkey {keys}"),
        A::Text { text } => format!("types {:?}", text.chars().take(24).collect::<String>()),
        A::Media { key } => format!("media {key:?}"),
        A::OpenUrl { url } => format!("opens {url}"),
        A::OpenPath { path } => format!("opens {}", path.display()),
        A::Launch { program, .. } => format!("starts {program}"),
        A::Command { program, .. } => format!("runs {program}"),
        A::Multi { steps } => format!("{} steps", steps.len()),
        A::Obs(o) => format!("OBS {o:?}"),
        A::HomeAssistant {
            service, entity, ..
        } => {
            format!(
                "Home Assistant {service}{}",
                entity
                    .as_deref()
                    .map(|e| format!(" {e}"))
                    .unwrap_or_default()
            )
        }
        A::Plugin { plugin, action, .. } => format!("{plugin}: {action}"),
        A::None => "nothing".to_owned(),
    }
}
