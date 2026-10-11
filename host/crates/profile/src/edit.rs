//! Change a profile file the way a person would: in place, keeping comments, layout and
//! anything the change doesn't touch.
//!
//! The desktop app sends [`Edit`]s; [`apply`] turns the file text into new file text,
//! which is checked as a whole before anything is written.

use perritodeck_actions::ActionSpec;
use serde::{Deserialize, Serialize};
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table, Value};

use crate::{NavSpec, Profile};

/// One change to a profile. Positions are `[row, column]`.
// SetKey is the big one and by far the most common; boxing it buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Edit {
    /// The profile's display name.
    SetName {
        name: String,
    },
    /// Create the key, or update it. Fields left out (`None`) are removed from the key;
    /// its icon and toggle states are kept.
    SetKey {
        page: String,
        at: [u32; 2],
        key: KeyFields,
    },
    ClearKey {
        page: String,
        at: [u32; 2],
    },
    /// Move a key; if another key is there, the two swap places.
    MoveKey {
        page: String,
        from: [u32; 2],
        to: [u32; 2],
    },
    /// Move a key to another page; a key already there swaps onto this page.
    MoveKeyToPage {
        from_page: String,
        at: [u32; 2],
        to_page: String,
        to: [u32; 2],
    },
    AddPage {
        name: String,
    },
    /// Also updates keys that open the page.
    RenamePage {
        from: String,
        to: String,
    },
    RemovePage {
        name: String,
    },
    MovePage {
        name: String,
        index: usize,
    },
}

/// The parts of a key the editor changes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyFields {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub bg: Option<String>,
    #[serde(default)]
    pub fg: Option<String>,
    #[serde(default)]
    pub action: Option<ActionSpec>,
    #[serde(default)]
    pub nav: Option<NavSpec>,
    /// Image file, relative to the profile's folder.
    #[serde(default)]
    pub icon: Option<std::path::PathBuf>,
    /// Makes the key a slider.
    #[serde(default)]
    pub slider: Option<perritodeck_actions::SliderSpec>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("the profile file can't be read as TOML: {0}")]
    Syntax(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("that change would break the profile:\n  - {}", .0.join("\n  - "))]
    Invalid(Vec<String>),
}

/// Apply `edits` in order to the file text, and return the new text. Nothing is
/// returned unless the result is a valid profile.
pub fn apply(text: &str, edits: &[Edit]) -> Result<String, EditError> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| EditError::Syntax(e.to_string()))?;
    for edit in edits {
        apply_one(&mut doc, edit)?;
    }
    // Let the file be written in the order of its structure (pages, then each page's
    // keys) so moved and new pages and keys sit where a person would expect.
    clear_positions(doc.as_table_mut());
    let out = doc.to_string();
    let profile = Profile::parse(&out).map_err(|e| EditError::Invalid(vec![e]))?;
    let problems = profile.problems();
    if !problems.is_empty() {
        return Err(EditError::Invalid(problems));
    }
    Ok(out)
}

fn apply_one(doc: &mut DocumentMut, edit: &Edit) -> Result<(), EditError> {
    match edit {
        Edit::SetName { name } => {
            let name = name.trim();
            if name.is_empty() {
                return Err(EditError::Unsupported("a profile needs a name".into()));
            }
            set_opt(doc.as_table_mut(), "name", Some(Value::from(name)));
        }
        Edit::SetKey { page, at, key } => {
            let keys = page_keys(doc, page)?;
            let i = match find_key(keys, *at) {
                Some(i) => i,
                None => {
                    let mut t = Table::new();
                    t.insert("at", value(pos(*at)));
                    keys.push(t);
                    keys.len() - 1
                }
            };
            let t = keys.get_mut(i).expect("index just found");
            set_opt(
                t,
                "title",
                key.title.as_ref().map(|s| Value::from(s.as_str())),
            );
            set_opt(t, "bg", key.bg.as_ref().map(|s| Value::from(s.as_str())));
            set_opt(t, "fg", key.fg.as_ref().map(|s| Value::from(s.as_str())));
            set_opt(t, "action", key.action.as_ref().map(to_value).transpose()?);
            set_opt(t, "nav", key.nav.as_ref().map(to_value).transpose()?);
            set_opt(t, "slider", key.slider.as_ref().map(to_value).transpose()?);
            set_opt(
                t,
                "icon",
                key.icon
                    .as_ref()
                    .map(|p| Value::from(p.to_string_lossy().replace('\\', "/"))),
            );
        }
        Edit::ClearKey { page, at } => {
            let keys = page_keys(doc, page)?;
            let i = find_key(keys, *at).ok_or_else(|| no_key(page, *at))?;
            keys.remove(i);
        }
        Edit::MoveKey { page, from, to } => {
            let keys = page_keys(doc, page)?;
            let i = find_key(keys, *from).ok_or_else(|| no_key(page, *from))?;
            if let Some(j) = find_key(keys, *to) {
                keys.get_mut(j)
                    .expect("found")
                    .insert("at", value(pos(*from)));
            }
            keys.get_mut(i)
                .expect("found")
                .insert("at", value(pos(*to)));
        }
        Edit::MoveKeyToPage {
            from_page,
            at,
            to_page,
            to,
        } => {
            if from_page == to_page {
                return apply_one(
                    doc,
                    &Edit::MoveKey {
                        page: from_page.clone(),
                        from: *at,
                        to: *to,
                    },
                );
            }
            let src = page_keys(doc, from_page)?;
            let i = find_key(src, *at).ok_or_else(|| no_key(from_page, *at))?;
            let mut moving = src.remove(i);
            let dst = page_keys(doc, to_page)?;
            let swapped = find_key(dst, *to).map(|j| dst.remove(j));
            moving.insert("at", value(pos(*to)));
            dst.push(moving);
            if let Some(mut other) = swapped {
                other.insert("at", value(pos(*at)));
                page_keys(doc, from_page)?.push(other);
            }
        }
        Edit::AddPage { name } => {
            let pages = pages(doc)?;
            if pages.iter().any(|p| page_name(p) == Some(name)) {
                return Err(EditError::Unsupported(format!(
                    "there's already a page called “{name}”"
                )));
            }
            let mut t = Table::new();
            t.insert("name", value(name.as_str()));
            pages.push(t);
        }
        Edit::RenamePage { from, to } => {
            let pages = pages(doc)?;
            if from != to && pages.iter().any(|p| page_name(p) == Some(to)) {
                return Err(EditError::Unsupported(format!(
                    "there's already a page called “{to}”"
                )));
            }
            let i = find_page(pages, from)?;
            pages
                .get_mut(i)
                .expect("found")
                .insert("name", value(to.as_str()));
            // Keys (and toggle states) that open the old name now open the new one.
            for p in pages.iter_mut() {
                if let Some(keys) = p.get_mut("keys").and_then(Item::as_array_of_tables_mut) {
                    for k in keys.iter_mut() {
                        rename_nav(k, from, to);
                    }
                }
            }
        }
        Edit::RemovePage { name } => {
            let pages = pages(doc)?;
            let i = find_page(pages, name)?;
            pages.remove(i);
        }
        Edit::MovePage { name, index } => {
            let pages = pages(doc)?;
            let i = find_page(pages, name)?;
            let t = pages.remove(i);
            pages.insert((*index).min(pages.len()), t);
        }
    }
    Ok(())
}

fn rename_nav(key: &mut Table, from: &str, to: &str) {
    let opens_from = |nav: Option<&Item>| {
        nav.and_then(|n| {
            n.as_inline_table()
                .and_then(|t| t.get("page"))
                .or_else(|| n.get("page").and_then(Item::as_value))
        })
        .and_then(Value::as_str)
            == Some(from)
    };
    if opens_from(key.get("nav")) {
        let mut nav = toml_edit::InlineTable::new();
        nav.insert("page", Value::from(to));
        key.insert("nav", value(nav));
    }
}

fn pos(at: [u32; 2]) -> Array {
    let mut a = Array::new();
    a.push(i64::from(at[0]));
    a.push(i64::from(at[1]));
    a
}

fn to_value<T: Serialize>(v: &T) -> Result<Value, EditError> {
    v.serialize(toml_edit::ser::ValueSerializer::new())
        .map_err(|e| EditError::Unsupported(format!("can't write that as TOML: {e}")))
}

fn set_opt(t: &mut Table, key: &str, v: Option<Value>) {
    match v {
        Some(v) => {
            // Keep the comment and spacing in front of an existing entry.
            match t.get_mut(key).and_then(Item::as_value_mut) {
                Some(old) => {
                    let decor = old.decor().clone();
                    *old = v;
                    *old.decor_mut() = decor;
                }
                None => {
                    t.insert(key, value(v));
                }
            }
        }
        None => {
            t.remove(key);
        }
    }
}

fn pages(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, EditError> {
    doc.get_mut("pages")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| {
            EditError::Unsupported(
                "this profile's pages aren't written as [[pages]] sections".into(),
            )
        })
}

fn page_name(t: &Table) -> Option<&str> {
    t.get("name").and_then(Item::as_str)
}

fn find_page(pages: &ArrayOfTables, name: &str) -> Result<usize, EditError> {
    pages
        .iter()
        .position(|p| page_name(p) == Some(name))
        .ok_or_else(|| EditError::NotFound(format!("there's no page called “{name}”")))
}

fn page_keys<'a>(doc: &'a mut DocumentMut, page: &str) -> Result<&'a mut ArrayOfTables, EditError> {
    let pages = pages(doc)?;
    let i = find_page(pages, page)?;
    let t = pages.get_mut(i).expect("found");
    if t.get("keys").is_none() {
        t.insert("keys", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    t.get_mut("keys")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| {
            EditError::Unsupported(format!(
                "the keys on page “{page}” aren't written as [[pages.keys]] sections"
            ))
        })
}

fn key_at(t: &Table) -> Option<[u32; 2]> {
    let a = t.get("at")?.as_array()?;
    let n = |i: usize| {
        a.get(i)
            .and_then(Value::as_integer)
            .and_then(|v| u32::try_from(v).ok())
    };
    Some([n(0)?, n(1)?])
}

fn find_key(keys: &ArrayOfTables, at: [u32; 2]) -> Option<usize> {
    keys.iter().position(|k| key_at(k) == Some(at))
}

fn no_key(page: &str, at: [u32; 2]) -> EditError {
    EditError::NotFound(format!(
        "there's no key at row {}, column {} on page “{page}”",
        at[0] + 1,
        at[1] + 1
    ))
}

fn clear_positions(t: &mut Table) {
    t.set_position(None);
    for (_, item) in t.iter_mut() {
        match item {
            Item::Table(t) => clear_positions(t),
            Item::ArrayOfTables(a) => a.iter_mut().for_each(clear_positions),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SimpleNav, STARTER_PROFILE};

    fn comments(s: &str) -> Vec<&str> {
        s.lines()
            .filter(|l| l.trim_start().starts_with('#'))
            .collect()
    }

    fn parsed(s: &str) -> Profile {
        Profile::parse(s).unwrap()
    }

    fn key<'a>(p: &'a Profile, page: &str, at: [u32; 2]) -> Option<&'a crate::KeyDef> {
        p.pages[p.page_index(page).unwrap()]
            .keys
            .iter()
            .find(|k| k.at == at)
    }

    fn set(page: &str, at: [u32; 2], key: KeyFields) -> Edit {
        Edit::SetKey {
            page: page.into(),
            at,
            key,
        }
    }

    #[test]
    fn editing_a_key_keeps_every_comment() {
        let out = apply(
            STARTER_PROFILE,
            &[set(
                "Main",
                [1, 1],
                KeyFields {
                    title: Some("Copy!".into()),
                    bg: Some("#123456".into()),
                    action: Some(ActionSpec::Hotkey {
                        keys: "Ctrl+Insert".into(),
                    }),
                    ..Default::default()
                },
            )],
        )
        .unwrap();
        assert_eq!(comments(&out), comments(STARTER_PROFILE));
        let p = parsed(&out);
        let k = key(&p, "Main", [1, 1]).unwrap();
        assert_eq!(k.title.as_deref(), Some("Copy!"));
        assert_eq!(k.bg.as_deref(), Some("#123456"));
        assert_eq!(
            k.action,
            Some(ActionSpec::Hotkey {
                keys: "Ctrl+Insert".into()
            })
        );
        // Only that key changed.
        let before = parsed(STARTER_PROFILE);
        assert_eq!(key(&p, "Main", [1, 2]), key(&before, "Main", [1, 2]));
        assert_eq!(p.pages.len(), before.pages.len());
    }

    #[test]
    fn new_keys_land_with_their_page() {
        let out = apply(
            STARTER_PROFILE,
            &[set(
                "Main",
                [2, 2],
                KeyFields {
                    title: Some("New".into()),
                    action: Some(ActionSpec::Text { text: "hi".into() }),
                    ..Default::default()
                },
            )],
        )
        .unwrap();
        let p = parsed(&out);
        assert_eq!(
            key(&p, "Main", [2, 2]).unwrap().title.as_deref(),
            Some("New")
        );
        // Written inside the Main page, before the More page starts.
        let new_at = out.find("title = \"New\"").unwrap();
        let more_at = out.find("name = \"More\"").unwrap();
        assert!(new_at < more_at, "{out}");
    }

    #[test]
    fn clear_move_and_swap() {
        let out = apply(
            STARTER_PROFILE,
            &[Edit::ClearKey {
                page: "Main".into(),
                at: [1, 3],
            }],
        )
        .unwrap();
        assert!(key(&parsed(&out), "Main", [1, 3]).is_none());

        // Swap Copy (1,1) and Paste (1,2); move Mute (0,4) to an empty spot.
        let out = apply(
            STARTER_PROFILE,
            &[
                Edit::MoveKey {
                    page: "Main".into(),
                    from: [1, 1],
                    to: [1, 2],
                },
                Edit::MoveKey {
                    page: "Main".into(),
                    from: [0, 4],
                    to: [2, 3],
                },
            ],
        )
        .unwrap();
        let p = parsed(&out);
        assert_eq!(
            key(&p, "Main", [1, 2]).unwrap().title.as_deref(),
            Some("Copy")
        );
        assert_eq!(
            key(&p, "Main", [1, 1]).unwrap().title.as_deref(),
            Some("Paste")
        );
        assert_eq!(
            key(&p, "Main", [2, 3]).unwrap().title.as_deref(),
            Some("Mute")
        );
        assert!(key(&p, "Main", [0, 4]).is_none());
    }

    #[test]
    fn move_between_pages() {
        let out = apply(
            STARTER_PROFILE,
            &[Edit::MoveKeyToPage {
                from_page: "Main".into(),
                at: [1, 1],
                to_page: "More".into(),
                to: [0, 1],
            }],
        )
        .unwrap();
        let p = parsed(&out);
        // Copy went to More; More's key there came back to Main in Copy's place.
        assert_eq!(
            key(&p, "More", [0, 1]).unwrap().title.as_deref(),
            Some("Copy")
        );
        assert_eq!(
            key(&p, "Main", [1, 1]).unwrap().title,
            key(&parsed(STARTER_PROFILE), "More", [0, 1]).unwrap().title
        );
    }

    #[test]
    fn pages_add_rename_reorder_remove() {
        let out = apply(
            STARTER_PROFILE,
            &[
                Edit::AddPage {
                    name: "Stream".into(),
                },
                set(
                    "Stream",
                    [0, 0],
                    KeyFields {
                        title: Some("‹".into()),
                        nav: Some(NavSpec::Simple(SimpleNav::Back)),
                        ..Default::default()
                    },
                ),
                Edit::RenamePage {
                    from: "More".into(),
                    to: "Extras".into(),
                },
                Edit::MovePage {
                    name: "Stream".into(),
                    index: 1,
                },
            ],
        )
        .unwrap();
        let p = parsed(&out);
        let names: Vec<_> = p.pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Main", "Stream", "Extras"]);
        // The folder key on Main follows the rename.
        assert_eq!(
            key(&p, "Main", [2, 0]).unwrap().nav,
            Some(NavSpec::Page {
                page: "Extras".into()
            })
        );
        assert_eq!(comments(&out), comments(STARTER_PROFILE));

        // A page that a key still opens can't be removed.
        let err = apply(
            &out,
            &[Edit::RemovePage {
                name: "Extras".into(),
            }],
        )
        .unwrap_err();
        assert!(matches!(err, EditError::Invalid(_)), "{err}");
        let out = apply(
            &out,
            &[
                Edit::ClearKey {
                    page: "Main".into(),
                    at: [2, 0],
                },
                Edit::RemovePage {
                    name: "Extras".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(parsed(&out).pages.len(), 2);
    }

    #[test]
    fn renaming_the_profile() {
        let out = apply(
            STARTER_PROFILE,
            &[Edit::SetName {
                name: "Streaming".into(),
            }],
        )
        .unwrap();
        assert_eq!(parsed(&out).name, "Streaming");
        assert_eq!(comments(&out), comments(STARTER_PROFILE));
        assert!(apply(STARTER_PROFILE, &[Edit::SetName { name: " ".into() }]).is_err());
    }

    #[test]
    fn bad_changes_are_refused() {
        let err = apply(
            STARTER_PROFILE,
            &[set(
                "Main",
                [0, 0],
                KeyFields {
                    bg: Some("blue".into()),
                    ..Default::default()
                },
            )],
        );
        assert!(matches!(err, Err(EditError::Invalid(_))));
        let err = apply(
            STARTER_PROFILE,
            &[Edit::ClearKey {
                page: "Main".into(),
                at: [2, 2],
            }],
        );
        assert!(matches!(err, Err(EditError::NotFound(_))));
        let err = apply(
            STARTER_PROFILE,
            &[Edit::AddPage {
                name: "Main".into(),
            }],
        );
        assert!(matches!(err, Err(EditError::Unsupported(_))));
        assert!(matches!(
            apply("not toml [", &[]),
            Err(EditError::Syntax(_))
        ));
    }

    #[test]
    fn edits_arrive_as_json_from_the_app() {
        let e: Edit = serde_json::from_str(
            r#"{"op":"setKey","page":"Main","at":[0,1],"key":{"title":"Hi","action":{"type":"media","key":"mute"}}}"#,
        )
        .unwrap();
        assert!(matches!(e, Edit::SetKey { .. }));
        let e: Edit = serde_json::from_str(
            r#"{"op":"moveKeyToPage","fromPage":"A","at":[0,0],"toPage":"B","to":[1,1]}"#,
        )
        .unwrap();
        assert!(matches!(e, Edit::MoveKeyToPage { .. }));
    }
}
