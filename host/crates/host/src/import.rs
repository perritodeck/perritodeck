//! Adding a profile from a `.perritoprofile` file, in two steps: read it and list the
//! keys that run things ([`Host::stage_import`]), then write it once the person has
//! chosen ([`Host::finish_import`]) or drop it ([`Host::cancel_import`]).
//!
//! What's listed comes from [`perritodeck_profile::risk`]; this module reads the file
//! safely, makes every value from it safe to show, and keeps the parsed file in memory
//! between the two steps, so what's written is exactly what was shown.

use std::{collections::HashMap, io::Read, path::Path};

use perritodeck_actions::Place;
use perritodeck_profile::{
    risk::{self, RiskKind, RiskyKey, Source, Typed, What},
    Profile,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{icons_of, importable_image, inside_profiles_folder, slug, unique_id, Host};

/// How much of a `.perritoprofile` is read before it's refused as not a profile file.
pub(crate) struct Limits {
    /// One entry, the profile or an image.
    pub entry: u64,
    /// All its images together. They stay in memory until the person chooses, so a bad
    /// file writes nothing.
    pub total: u64,
    /// Entries in the zip, read or not.
    pub files: usize,
}

pub(crate) const LIMITS: Limits = Limits {
    entry: 20 * 1024 * 1024,
    total: 100 * 1024 * 1024,
    files: 5_000,
};

/// A `.perritoprofile`, read and checked, with nothing written yet.
pub(crate) struct ProfileArchive {
    pub file_name: String,
    /// The profile exactly as in the file.
    pub text: String,
    pub profile: Profile,
    /// The images its keys show, by their path under the profiles folder.
    pub images: Vec<(String, Vec<u8>)>,
}

impl ProfileArchive {
    pub(crate) fn read(src: &Path) -> Result<Self, String> {
        Self::read_with(src, &LIMITS)
    }

    pub(crate) fn read_with(src: &Path, l: &Limits) -> Result<Self, String> {
        let bad = || format!("{} isn't a PerritoDeck profile file.", src.display());
        let file = std::fs::File::open(src)
            .map_err(|e| format!("Couldn't open {}: {e}", src.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|_| bad())?;
        if zip.len() > l.files {
            return Err(format!("{} has too many files.", src.display()));
        }
        let mut text = String::new();
        {
            let entry = zip.by_name("profile.toml").map_err(|_| bad())?;
            if entry.size() > l.entry {
                return Err(bad());
            }
            entry
                .take(l.entry + 1)
                .read_to_string(&mut text)
                .map_err(|_| bad())?;
            if text.len() as u64 > l.entry {
                return Err(bad());
            }
        }
        let profile = Profile::parse(&text).map_err(|e| format!("{}: {e}", bad()))?;
        let problems = profile.problems();
        if !problems.is_empty() {
            return Err(format!(
                "{} has problems: {}",
                src.display(),
                problems.join("; ")
            ));
        }
        // A key's image is read whenever its page is shown, so an imported profile may
        // name only images inside the profiles folder: not another file of this
        // computer's, a share (Windows would sign in to it) or a device.
        let outside = outside_icons(&profile);
        if !outside.is_empty() {
            return Err(format!(
                "{} names images outside the profiles folder, which an imported profile can't \
                 use: {}. Put the images next to the profile, name them relative to it, and \
                 export it again.",
                src.display(),
                outside
            ));
        }
        // Only the images its keys show, which is all export_profile packs. With the
        // profile in the data folder, anything else could land among the host's own
        // files: a plugin, which would start with the host, or another profile, which
        // nobody would have checked.
        let mut images: Vec<(String, Vec<u8>)> = Vec::new();
        let mut total = 0;
        for icon in icons_of(&profile) {
            let Some(rel) = importable_image(&icon) else {
                continue;
            };
            let entry = match zip.by_name(&format!("images/{rel}")) {
                Ok(entry) => entry,
                Err(zip::result::ZipError::FileNotFound) => continue,
                Err(_) => return Err(bad()),
            };
            if entry.size() > l.entry {
                return Err(bad());
            }
            // A read past the size the zip gives fails, so this is all it can unpack.
            total += entry.size();
            if total > l.total {
                return Err(format!("{} is too big once unpacked.", src.display()));
            }
            let mut bytes = Vec::new();
            entry
                .take(l.entry + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| bad())?;
            if bytes.len() as u64 > l.entry {
                return Err(bad());
            }
            images.push((rel, bytes));
        }
        Ok(ProfileArchive {
            file_name: src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            text,
            profile,
            images,
        })
    }
}

/// Keys whose image isn't inside the profiles folder, described for an error: "page
/// "Main", row 1, column 2 ("C:\\x.png")", the first few and how many more.
fn outside_icons(profile: &Profile) -> String {
    const SHOWN: usize = 5;
    let mut found = Vec::new();
    for page in &profile.pages {
        for k in &page.keys {
            let mut icons = k
                .icon
                .iter()
                .chain(k.states.iter().filter_map(|s| s.icon.as_ref()));
            if let Some(icon) = icons.find(|i| inside_profiles_folder(i).is_none()) {
                // Quoted with escapes, so nothing in the path can disguise itself.
                found.push(format!(
                    "page {:?}, row {}, column {} ({:?})",
                    page.name,
                    k.at[0] + 1,
                    k.at[1] + 1,
                    icon.to_string_lossy()
                ));
            }
        }
    }
    let more = found.len().saturating_sub(SHOWN);
    found.truncate(SHOWN);
    let mut out = found.join("; ");
    if more > 0 {
        out += &format!("; and {more} more");
    }
    out
}

/// What the person chose for a profile whose keys run things.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportChoice {
    /// Add it as it is.
    Everything,
    /// Add it with every listed action removed ([`risk::switch_off`]).
    SwitchOff,
}

/// A profile that was added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub id: String,
    /// Keys whose listed actions were removed.
    pub switched_off: usize,
}

/// Where an import stands after the file was picked.
#[derive(Debug, Serialize)]
#[serde(tag = "step", rename_all = "camelCase")]
pub enum ImportStep {
    /// The person closed the file picker (made by the app).
    Cancelled,
    /// Nothing in it runs anything, so it was added at once.
    Imported(Imported),
    /// Some keys run things: ask, then call finish_import or cancel_import.
    Confirm { preview: Box<ImportPreview> },
}

/// Part of a value from a file, safe to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Piece {
    Text(String),
    /// A character that can't be seen or would change what's around it: "Enter" for a
    /// line break, "Tab", or its code point ("U+202E").
    Mark {
        mark: String,
    },
    /// How many characters were left out at the end.
    Cut {
        more: usize,
    },
}

/// A value from a file, safe to show.
pub type Shown = Vec<Piece>;

/// Most keys listed; the rest are counted.
pub const MAX_LISTED_KEYS: usize = 500;
/// Most items listed for one key.
pub const MAX_ITEMS_PER_KEY: usize = 20;
/// Most arguments listed for one program, and steps for one typing entry.
pub const MAX_ARGS: usize = 100;
/// Most characters of a program, path, argument, typed text or settings.
pub const VALUE: usize = 4_096;
/// Most characters of a title, page, profile, file, plugin or action name.
pub const NAME: usize = 200;

/// What the window shows before a profile whose keys run things is added.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub token: String,
    pub name: Shown,
    pub file: Shown,
    pub pages: usize,
    pub keys: usize,
    /// Keys with something listed, counting those past the cap.
    pub total: usize,
    pub pages_with_risky: usize,
    pub keys_that_run: usize,
    pub keys_that_type: usize,
    pub has_sliders: bool,
    pub risky: Vec<RiskyKeyView>,
    pub more_keys: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskyKeyView {
    pub page: Shown,
    pub page_number: usize,
    pub row: u32,
    pub col: u32,
    pub title: Shown,
    pub state_count: usize,
    pub items: Vec<RiskView>,
    pub more_items: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskView {
    pub kind: RiskKind,
    pub states: Vec<usize>,
    pub unused: bool,
    pub place: Vec<Place>,
    pub what: WhatView,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum WhatView {
    Command {
        program: Shown,
        args: Vec<Shown>,
        more_args: usize,
        cwd: Option<Shown>,
    },
    Launch {
        program: Shown,
        args: Vec<Shown>,
        more_args: usize,
    },
    OpenPath {
        path: Shown,
    },
    Plugin {
        plugin: Shown,
        action: Shown,
        settings: Option<Shown>,
        installed: bool,
        plugin_name: Option<Shown>,
        action_name: Option<Shown>,
    },
    SdPlugin {
        plugin: Shown,
        action: Shown,
        settings: Option<Shown>,
        installed: bool,
        plugin_name: Option<Shown>,
        action_name: Option<Shown>,
        on_show: bool,
    },
    CommandSlider {
        program: Shown,
        args: Vec<Shown>,
        more_args: usize,
    },
    PluginSlider {
        plugin: Shown,
        action: Shown,
        settings: Option<Shown>,
        installed: bool,
        plugin_name: Option<Shown>,
        action_name: Option<Shown>,
    },
    Typing {
        steps: Vec<TypedView>,
        more_steps: usize,
    },
}

/// One step of a typing entry: text typed, or a hotkey pressed.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TypedView {
    Text { text: Shown, place: Vec<Place> },
    Keys { keys: Shown, place: Vec<Place> },
}

/// Characters that show nothing, or change how what's around them shows: every code
/// point in Unicode's general categories Cc, Cf, Zl and Zp, or with the property
/// Default_Ignorable_Code_Point (bidi controls, zero-width characters, fillers, variation
/// selectors, tags, and the unassigned ones reserved as ignorable), in sorted ranges.
/// Generated from Unicode 17.0 (Node 26's tables) by testing every code point against
/// `/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}\p{Default_Ignorable_Code_Point}]/u`, leaving out U+FE0F
/// (emoji presentation) so emoji stay readable. Regenerate when Unicode adds some.
const INVISIBLE: &[(u32, u32)] = &[
    (0x0000, 0x001F),
    (0x007F, 0x009F),
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x115F, 0x1160),
    (0x17B4, 0x17B5),
    (0x180B, 0x180F),
    (0x200B, 0x200F),
    (0x2028, 0x202E),
    (0x2060, 0x206F),
    (0x3164, 0x3164),
    (0xFE00, 0xFE0E),
    (0xFEFF, 0xFEFF),
    (0xFFA0, 0xFFA0),
    (0xFFF0, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0000, 0xE0FFF),
];

/// Blank-looking characters in no category above, marked as well: the Braille blank.
const BLANK: &[char] = &['\u{2800}'];

fn invisible(c: char) -> bool {
    let c32 = u32::from(c);
    let i = INVISIBLE.partition_point(|&(_, hi)| hi < c32);
    INVISIBLE.get(i).is_some_and(|&(lo, _)| lo <= c32) || BLANK.contains(&c)
}

/// `s` as pieces safe to show: line breaks, tabs, control characters and the
/// [`INVISIBLE`] ones become marks, and past `limit` characters the rest is counted.
pub fn shown(s: &str, limit: usize) -> Shown {
    let mut out = Vec::new();
    let mut text = String::new();
    let flush = |text: &mut String, out: &mut Vec<Piece>| {
        if !text.is_empty() {
            out.push(Piece::Text(std::mem::take(text)));
        }
    };
    let mut count = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if count >= limit {
            flush(&mut text, &mut out);
            out.push(Piece::Cut {
                more: 1 + chars.count(),
            });
            return out;
        }
        count += 1;
        let mark = match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                    count += 1;
                }
                Some("Enter".to_owned())
            }
            '\n' => Some("Enter".to_owned()),
            '\t' => Some("Tab".to_owned()),
            c if c.is_control() || invisible(c) => Some(format!("U+{:04X}", u32::from(c))),
            _ => None,
        };
        match mark {
            Some(mark) => {
                flush(&mut text, &mut out);
                out.push(Piece::Mark { mark });
            }
            None => text.push(c),
        }
    }
    flush(&mut text, &mut out);
    out
}

fn args_shown(args: &[String]) -> (Vec<Shown>, usize) {
    (
        args.iter()
            .take(MAX_ARGS)
            .map(|a| shown(a, VALUE))
            .collect(),
        args.len().saturating_sub(MAX_ARGS),
    )
}

/// Installed plugins' and Stream Deck plugins' names, and their actions' names, by id.
#[derive(Default)]
pub(crate) struct Names {
    pub plugins: HashMap<String, (String, HashMap<String, String>)>,
    pub sd_plugins: HashMap<String, (String, HashMap<String, String>)>,
}

struct PluginShown {
    plugin: Shown,
    action: Shown,
    settings: Option<Shown>,
    installed: bool,
    plugin_name: Option<Shown>,
    action_name: Option<Shown>,
}

fn plugin_shown(
    names: &HashMap<String, (String, HashMap<String, String>)>,
    plugin: &str,
    action: &str,
    settings: &Option<String>,
) -> PluginShown {
    let found = names.get(plugin);
    PluginShown {
        plugin: shown(plugin, NAME),
        action: shown(action, NAME),
        settings: settings.as_deref().map(|s| shown(s, VALUE)),
        installed: found.is_some(),
        plugin_name: found.map(|(n, _)| shown(n, NAME)),
        action_name: found
            .and_then(|(_, actions)| actions.get(action))
            .map(|n| shown(n, NAME)),
    }
}

fn what_view(what: &What, names: &Names) -> WhatView {
    match what {
        What::Command { program, args, cwd } => {
            let (args, more_args) = args_shown(args);
            WhatView::Command {
                program: shown(program, VALUE),
                args,
                more_args,
                cwd: cwd.as_deref().map(|c| shown(c, VALUE)),
            }
        }
        What::Launch { program, args } => {
            let (args, more_args) = args_shown(args);
            WhatView::Launch {
                program: shown(program, VALUE),
                args,
                more_args,
            }
        }
        What::OpenPath { path } => WhatView::OpenPath {
            path: shown(path, VALUE),
        },
        What::Plugin {
            plugin,
            action,
            settings,
        } => {
            let p = plugin_shown(&names.plugins, plugin, action, settings);
            WhatView::Plugin {
                plugin: p.plugin,
                action: p.action,
                settings: p.settings,
                installed: p.installed,
                plugin_name: p.plugin_name,
                action_name: p.action_name,
            }
        }
        What::SdPlugin {
            plugin,
            action,
            settings,
            on_show,
        } => {
            let p = plugin_shown(&names.sd_plugins, plugin, action, settings);
            WhatView::SdPlugin {
                plugin: p.plugin,
                action: p.action,
                settings: p.settings,
                installed: p.installed,
                plugin_name: p.plugin_name,
                action_name: p.action_name,
                on_show: *on_show,
            }
        }
        What::CommandSlider { program, args } => {
            let (args, more_args) = args_shown(args);
            WhatView::CommandSlider {
                program: shown(program, VALUE),
                args,
                more_args,
            }
        }
        What::PluginSlider {
            plugin,
            action,
            settings,
        } => {
            let p = plugin_shown(&names.plugins, plugin, action, settings);
            WhatView::PluginSlider {
                plugin: p.plugin,
                action: p.action,
                settings: p.settings,
                installed: p.installed,
                plugin_name: p.plugin_name,
                action_name: p.action_name,
            }
        }
        What::Typing { steps } => WhatView::Typing {
            steps: steps
                .iter()
                .take(MAX_ARGS)
                .map(|s| match s {
                    Typed::Text { text, place } => TypedView::Text {
                        text: shown(text, VALUE),
                        place: place.clone(),
                    },
                    Typed::Keys { keys, place } => TypedView::Keys {
                        keys: shown(keys, NAME),
                        place: place.clone(),
                    },
                })
                .collect(),
            more_steps: steps.len().saturating_sub(MAX_ARGS),
        },
    }
}

/// The preview of a profile with these risky keys.
pub(crate) fn preview_of(
    token: &str,
    a: &ProfileArchive,
    risky: &[RiskyKey],
    names: &Names,
) -> ImportPreview {
    let mut pages_with_risky: Vec<usize> = risky.iter().map(|k| k.page_number).collect();
    pages_with_risky.dedup();
    let has = |kind: RiskKind| {
        risky
            .iter()
            .filter(|k| k.risks.iter().any(|r| r.kind == kind))
            .count()
    };
    ImportPreview {
        token: token.to_owned(),
        name: shown(&a.profile.name, NAME),
        file: shown(&a.file_name, NAME),
        pages: a.profile.pages.len(),
        keys: a.profile.pages.iter().map(|p| p.keys.len()).sum(),
        total: risky.len(),
        pages_with_risky: pages_with_risky.len(),
        keys_that_run: has(RiskKind::Runs),
        keys_that_type: has(RiskKind::Types),
        has_sliders: risky
            .iter()
            .any(|k| k.risks.iter().any(|r| r.source == Source::Slider)),
        risky: risky
            .iter()
            .take(MAX_LISTED_KEYS)
            .map(|k| RiskyKeyView {
                page: shown(&k.page, NAME),
                page_number: k.page_number,
                row: k.at[0],
                col: k.at[1],
                title: shown(&k.title, NAME),
                state_count: k.state_count,
                items: k
                    .risks
                    .iter()
                    .take(MAX_ITEMS_PER_KEY)
                    .map(|r| RiskView {
                        kind: r.kind,
                        states: r.states.clone(),
                        unused: r.unused,
                        place: r.place.clone(),
                        what: what_view(&r.what, names),
                    })
                    .collect(),
                more_items: k.risks.len().saturating_sub(MAX_ITEMS_PER_KEY),
            })
            .collect(),
        more_keys: risky.len().saturating_sub(MAX_LISTED_KEYS),
    }
}

/// The one import waiting for the person's choice, if any.
#[derive(Default)]
pub(crate) struct Staging {
    seq: u64,
    staged: Option<(String, ProfileArchive)>,
}

const STALE: &str = "That import was replaced or cancelled; import the file again.";

impl Host {
    /// Every way a `.perritoprofile` gets in must go through here. Reads and checks it;
    /// adds it at once when no key runs anything, otherwise keeps it in memory (nothing
    /// written, images included) until [`Host::finish_import`] or
    /// [`Host::cancel_import`]. A new call drops any kept one.
    pub fn stage_import(&self, src: &Path) -> Result<ImportStep, String> {
        let archive = ProfileArchive::read(src)?;
        let risky = archive.profile.risky_keys();
        let token = {
            let mut s = self.import_staging.lock().expect("import staging poisoned");
            s.staged = None;
            if risky.is_empty() {
                None
            } else {
                s.seq += 1;
                Some(format!("import-{}", s.seq))
            }
        };
        let Some(token) = token else {
            return self
                .import_archive(archive, ImportChoice::Everything)
                .map(ImportStep::Imported);
        };
        let preview = preview_of(&token, &archive, &risky, &self.plugin_names());
        let mut s = self.import_staging.lock().expect("import staging poisoned");
        // Unless another import started meanwhile.
        if format!("import-{}", s.seq) == token {
            s.staged = Some((token, archive));
        }
        Ok(ImportStep::Confirm {
            preview: Box::new(preview),
        })
    }

    /// Add the profile [`Host::stage_import`] kept, as the person chose. A token works
    /// once, and only for the latest import.
    pub fn finish_import(&self, token: &str, choice: ImportChoice) -> Result<Imported, String> {
        let archive = {
            let mut s = self.import_staging.lock().expect("import staging poisoned");
            match s.staged.take() {
                Some((t, a)) if t == token => a,
                other => {
                    s.staged = other;
                    return Err(STALE.into());
                }
            }
        };
        self.import_archive(archive, choice)
    }

    /// Drop the import [`Host::stage_import`] kept, writing nothing.
    pub fn cancel_import(&self, token: &str) {
        let mut s = self.import_staging.lock().expect("import staging poisoned");
        if s.staged.as_ref().is_some_and(|(t, _)| t == token) {
            s.staged = None;
        }
    }

    fn plugin_names(&self) -> Names {
        let mut names = Names::default();
        for p in self.plugins.list() {
            let actions = p
                .manifest
                .actions
                .iter()
                .map(|a| (a.id.clone(), a.name.clone()))
                .collect();
            names
                .plugins
                .insert(p.manifest.id.clone(), (p.manifest.name.clone(), actions));
        }
        for p in self.sd_plugins.iter().flat_map(|s| s.list()) {
            let actions = p
                .actions
                .iter()
                .map(|a| (a.uuid.clone(), a.name.clone()))
                .collect();
            names
                .sd_plugins
                .insert(p.uuid.clone(), (p.name.clone(), actions));
        }
        names
    }

    fn import_archive(&self, a: ProfileArchive, choice: ImportChoice) -> Result<Imported, String> {
        let listed = a.profile.risky_keys().len();
        let (text, switched_off) = match choice {
            ImportChoice::Everything => (a.text, 0),
            ImportChoice::SwitchOff => risk::switch_off(&a.text)?,
        };
        // Imports never write outside the profiles folder: importable_image saw to that.
        for (rel, bytes) in a.images {
            let path = self.profiles_dir.join(&rel);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            // Images are named by content, so an existing file is the same image.
            if !path.exists() {
                std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            }
        }
        let id = unique_id(&slug(&a.profile.name), |id| self.path_of(id).exists());
        self.write_profile(&self.path_of(&id), &text)?;
        let how = match (listed, choice) {
            (0, _) => "none",
            (_, ImportChoice::Everything) => "everything",
            (_, ImportChoice::SwitchOff) => "switched off",
        };
        info!("imported profile {id}: {listed} keys that run things, {how}");
        Ok(Imported { id, switched_off })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Piece {
        Piece::Text(s.into())
    }

    fn mark(s: &str) -> Piece {
        Piece::Mark { mark: s.into() }
    }

    #[test]
    fn shown_marks_what_cannot_be_seen() {
        assert_eq!(
            shown("a\u{202E}b", VALUE),
            [text("a"), mark("U+202E"), text("b")]
        );
        assert_eq!(shown("\x1b[31m", VALUE), [mark("U+001B"), text("[31m")]);
        assert_eq!(
            shown("a\r\nb\nc\rd", VALUE),
            [
                text("a"),
                mark("Enter"),
                text("b"),
                mark("Enter"),
                text("c"),
                mark("Enter"),
                text("d")
            ]
        );
        assert_eq!(shown("a\tb", VALUE), [text("a"), mark("Tab"), text("b")]);
        for (c, code) in [
            ('\u{200B}', "U+200B"),
            ('\u{00AD}', "U+00AD"),
            ('\u{E0041}', "U+E0041"),
            ('\u{0085}', "U+0085"),
            ('\u{7F}', "U+007F"),
            ('\u{2066}', "U+2066"),
            ('\u{FEFF}', "U+FEFF"),
            // Format characters: Arabic number signs, the Syriac abbreviation mark,
            // Kaithi, Egyptian hieroglyph and shorthand format controls, musical
            // symbol formatting, the Mongolian vowel separator, unassigned ignorables.
            ('\u{0600}', "U+0600"),
            ('\u{0605}', "U+0605"),
            ('\u{06DD}', "U+06DD"),
            ('\u{070F}', "U+070F"),
            ('\u{0890}', "U+0890"),
            ('\u{08E2}', "U+08E2"),
            ('\u{110BD}', "U+110BD"),
            ('\u{110CD}', "U+110CD"),
            ('\u{13430}', "U+13430"),
            ('\u{1343F}', "U+1343F"),
            ('\u{1BCA0}', "U+1BCA0"),
            ('\u{1D173}', "U+1D173"),
            ('\u{1D17A}', "U+1D17A"),
            ('\u{180E}', "U+180E"),
            ('\u{E0080}', "U+E0080"),
            ('\u{E01F0}', "U+E01F0"),
            ('\u{FFF0}', "U+FFF0"),
        ] {
            assert_eq!(shown(&c.to_string(), VALUE), [mark(code)], "{code}");
        }
        assert_eq!(shown("❤️ é", VALUE), [text("❤️ é")]);
        let long = "é".repeat(5_000);
        let s = shown(&long, VALUE);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], text(&"é".repeat(VALUE)));
        assert_eq!(s[1], Piece::Cut { more: 904 });
        assert_eq!(shown("", VALUE), []);
        // On the wire: text, then marks and cuts as objects.
        assert_eq!(
            serde_json::to_value(shown("ab\ncd", 3)).unwrap(),
            serde_json::json!(["ab", {"mark": "Enter"}, {"more": 2}])
        );
    }
}
