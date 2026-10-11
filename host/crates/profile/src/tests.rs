use super::*;

#[test]
fn starter_profile_is_valid() {
    let p = Profile::parse(STARTER_PROFILE).expect("starter parses");
    assert_eq!(p.problems(), Vec::<String>::new());
    assert_eq!(p.grid, Some(GridSpec { cols: 5, rows: 3 }));
    assert_eq!(p.pages.len(), 2);
    assert!(
        p.pages[0].keys.iter().any(|k| k.states.len() == 2),
        "starter shows a toggle"
    );
    assert!(p.pages[1]
        .keys
        .iter()
        .any(|k| k.nav == Some(NavSpec::Simple(SimpleNav::Back))));
}

#[test]
fn looks_and_actions_fall_back_from_state_to_key() {
    let p = Profile::parse(
        r##"
schema_version = 1
name = "t"
[[pages]]
name = "P"
[[pages.keys]]
at = [0, 0]
title = "Base"
bg = "#111111"
action = { type = "text", text = "key" }
states = [ { title = "One" }, { bg = "#222222", action = { type = "text", text = "two" } } ]
"##,
    )
    .unwrap();
    let k = &p.pages[0].keys[0];
    assert_eq!(k.state_count(), 2);
    assert_eq!(k.look(0).title, "One");
    assert_eq!(k.look(0).bg, "#111111");
    assert_eq!(k.look(1).title, "Base");
    assert_eq!(k.look(1).bg, "#222222");
    assert_eq!(k.look(1).fg, DEFAULT_FG);
    assert_eq!(k.action(0), Some(&ActionSpec::Text { text: "key".into() }));
    assert_eq!(k.action(1), Some(&ActionSpec::Text { text: "two".into() }));
    assert_eq!(k.look(2), k.look(0), "state wraps around");
}

fn problems(toml: &str) -> Vec<String> {
    Profile::parse(toml).unwrap().problems()
}

#[test]
fn reports_problems_people_can_fix() {
    let p = problems(
        r##"
schema_version = 1
name = "bad"
[grid]
cols = 2
rows = 2
[[pages]]
name = "A"
[[pages.keys]]
at = [5, 0]
[[pages.keys]]
at = [0, 0]
bg = "red"
nav = { page = "Nowhere" }
[[pages.keys]]
at = [0, 0]
action = { type = "hotkey", keys = "Ctrl+Banana" }
[[pages.keys]]
at = [1, 1]
states = [ { title = "only one" } ]
[[pages]]
name = "A"
"##,
    );
    let all = p.join("\n");
    for needle in [
        "outside the 2×2 grid",
        "isn't a colour",
        "no page called \"Nowhere\"",
        "another key is already there",
        "unknown key",
        "at least two states",
        "page names must be unique",
    ] {
        assert!(all.contains(needle), "missing {needle:?} in:\n{all}");
    }
}

#[test]
fn nav_keys_cannot_also_act() {
    let p = problems(
        r##"
schema_version = 1
name = "x"
[[pages]]
name = "A"
[[pages.keys]]
at = [0, 0]
nav = "next"
action = { type = "text", text = "x" }
"##,
    );
    assert!(p[0].contains("navigation key"), "{p:?}");
}

#[test]
fn syntax_errors_point_at_the_line() {
    let err = Profile::parse("schema_version = 1\nname = \"x\"\n[[pages]]\nname = \n").unwrap_err();
    assert!(err.contains("line 4") || err.contains("4:"), "{err}");
    let unknown =
        Profile::parse("schema_version = 1\nname = \"x\"\ncolour = 1\npages = []\n").unwrap_err();
    assert!(unknown.contains("colour"), "{unknown}");
}

#[test]
fn wrong_schema_version_and_empty_pages() {
    let p = problems("schema_version = 9\nname = \"x\"\npages = []\n");
    assert_eq!(p.len(), 2, "{p:?}");
}

#[test]
fn load_and_ensure_starter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profiles/default.toml");
    assert!(ensure_starter(&path).unwrap());
    assert!(!ensure_starter(&path).unwrap(), "never overwrites");
    assert_eq!(load(&path).unwrap().name, "Starter");

    std::fs::write(&path, "schema_version = 1\nname = \"x\"\npages = []\n").unwrap();
    assert!(matches!(load(&path), Err(ProfileError::Invalid { .. })));
    std::fs::write(&path, "nope =").unwrap();
    assert!(matches!(load(&path), Err(ProfileError::Syntax { .. })));
}

#[test]
fn missing_icons_are_listed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("there.png"), b"x").unwrap();
    let p = Profile::parse(
        r##"
schema_version = 1
name = "x"
[[pages]]
name = "A"
[[pages.keys]]
at = [0, 0]
icon = "there.png"
[[pages.keys]]
at = [0, 1]
icon = "gone.png"
"##,
    )
    .unwrap();
    assert_eq!(
        p.missing_icons(dir.path()),
        vec![dir.path().join("gone.png")]
    );
}

#[test]
fn round_trips_through_toml() {
    let p = Profile::parse(STARTER_PROFILE).unwrap();
    let back = Profile::parse(&toml::to_string(&p).unwrap()).unwrap();
    assert_eq!(p, back);
}
