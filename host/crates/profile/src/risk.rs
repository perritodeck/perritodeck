//! Which keys run things: what a profile import lists before anything is written, and
//! what "Import with these keys off" removes. MCP's "run actions" permission uses the
//! same rules, so a key that needs it there is a key an import lists.
//!
//! Two kinds are listed. **Runs**: an action that starts a program, opens a file with
//! its app, or hands the press to a plugin ([`ActionSpec::runs_programs`]), and sliders
//! that run a program or plugin on every move. **Types**: text that ends a line, or
//! text in a key that also presses Enter, because "Win+R, type a command, Enter" runs a
//! command just as surely. Hotkeys alone, plain text, web addresses, media, OBS and
//! Home Assistant are not listed.
//!
//! This holds raw values; making them safe to show is the host's job.

use perritodeck_actions::{ActionSpec, Chord, Place, SliderSpec};
use serde::Serialize;

use crate::{
    edit::{self, Edit, EditError, KeyFields},
    KeyDef, Profile, StateDef,
};

/// What a listed action does when pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskKind {
    /// Runs a program or a plugin.
    Runs,
    /// Types text and presses Enter.
    Types,
}

/// Which part of the key holds it; used by [`switch_off`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The key's own action.
    Key,
    /// A toggle state's own action, counted from 0.
    State(usize),
    /// The key's slider.
    Slider,
}

/// What it runs or types, with the values from the file.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum What {
    Command {
        program: String,
        args: Vec<String>,
        cwd: Option<String>,
    },
    Launch {
        program: String,
        args: Vec<String>,
    },
    OpenPath {
        path: String,
    },
    /// `settings` is compact JSON, `None` when there are none.
    Plugin {
        plugin: String,
        action: String,
        settings: Option<String>,
    },
    /// `on_show`: the plugin gets the key, and its settings, as soon as a tablet shows
    /// its page, before anyone presses it.
    SdPlugin {
        plugin: String,
        action: String,
        settings: Option<String>,
        on_show: bool,
    },
    CommandSlider {
        program: String,
        args: Vec<String>,
    },
    PluginSlider {
        plugin: String,
        action: String,
        settings: Option<String>,
    },
    /// `enter_in_text`: the text itself ends a line; otherwise another step presses Enter.
    Typing {
        text: String,
        enter_in_text: bool,
    },
}

/// One listed action in a key.
#[derive(Debug, Clone, PartialEq)]
pub struct Risk {
    pub kind: RiskKind,
    pub source: Source,
    /// Toggle states (from 1) whose press runs it; empty for an ordinary key or a slider.
    pub states: Vec<usize>,
    /// The key's own action in a toggle whose states all have their own: not used now,
    /// but editing a state brings it back.
    pub unused: bool,
    /// Where it sits inside several-step and random actions.
    pub place: Vec<Place>,
    pub what: What,
}

/// A key with something listed.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskyKey {
    pub page: String,
    /// The page's place in the file, from 1.
    pub page_number: usize,
    pub at: [u32; 2],
    /// The key's title in its first state.
    pub title: String,
    /// Toggle states; 0 for an ordinary key.
    pub state_count: usize,
    pub risks: Vec<Risk>,
}

/// What [`switch_off`] says when its result still runs something.
pub const SWITCH_OFF_FAILED: &str = "Couldn't switch those keys off, so nothing was imported.";

/// Everything listed in a key made of these parts, in order: the key's own action, each
/// state's own action, then the slider.
pub fn risks_of(
    action: Option<&ActionSpec>,
    states: &[StateDef],
    slider: Option<&SliderSpec>,
) -> Vec<Risk> {
    // Which states press the key's own action: those without one of their own.
    let fallback: Vec<usize> = states
        .iter()
        .enumerate()
        .filter(|(_, s)| s.action.is_none())
        .map(|(i, _)| i + 1)
        .collect();
    let mut sources: Vec<(Source, &ActionSpec, Vec<usize>, bool)> = Vec::new();
    if let Some(a) = action {
        let unused = !states.is_empty() && fallback.is_empty();
        sources.push((Source::Key, a, fallback, unused));
    }
    for (i, s) in states.iter().enumerate() {
        if let Some(a) = &s.action {
            sources.push((Source::State(i), a, vec![i + 1], false));
        }
    }
    let walked: Vec<_> = sources
        .iter()
        .map(|(source, a, states, unused)| (*source, a.leaves(), states, *unused))
        .collect();
    // Enter pressed anywhere in the key: typed text then runs as a command line.
    let enter = walked
        .iter()
        .flat_map(|(_, leaves, _, _)| leaves)
        .any(|(_, a)| match a {
            ActionSpec::Hotkey { keys } => keys.parse::<Chord>().is_ok_and(|c| c.presses_enter()),
            _ => false,
        });
    // A tablet hands a Stream Deck plugin its key as soon as it shows it: the action for
    // the first state.
    let first_state_falls_back = states.first().map_or(true, |s| s.action.is_none());
    let mut out = Vec::new();
    for (source, leaves, states, unused) in walked {
        for (place, leaf) in leaves {
            let on_show = place.is_empty()
                && match source {
                    Source::State(0) => true,
                    Source::Key => first_state_falls_back,
                    _ => false,
                };
            let (kind, what) = match leaf {
                ActionSpec::Text { text } => {
                    let enter_in_text = text.contains(['\n', '\r']);
                    if !(enter_in_text || enter) {
                        continue;
                    }
                    let what = What::Typing {
                        text: text.clone(),
                        enter_in_text,
                    };
                    (RiskKind::Types, what)
                }
                leaf if leaf.runs_programs() => (RiskKind::Runs, what_runs(leaf, on_show)),
                _ => continue,
            };
            out.push(Risk {
                kind,
                source,
                states: states.clone(),
                unused,
                place,
                what,
            });
        }
    }
    if let Some(s) = slider.filter(|s| s.runs_programs()) {
        let what = match s {
            SliderSpec::Command { program, args } => What::CommandSlider {
                program: program.clone(),
                args: args.clone(),
            },
            SliderSpec::Plugin {
                plugin,
                action,
                settings,
                ..
            } => What::PluginSlider {
                plugin: plugin.clone(),
                action: action.clone(),
                settings: settings_text(settings),
            },
            SliderSpec::Volume | SliderSpec::HomeAssistant { .. } | SliderSpec::Obs { .. } => {
                unreachable!("these don't run programs")
            }
        };
        out.push(Risk {
            kind: RiskKind::Runs,
            source: Source::Slider,
            states: vec![],
            unused: false,
            place: vec![],
            what,
        });
    }
    out
}

fn settings_text(v: &serde_json::Value) -> Option<String> {
    (!v.is_null()).then(|| serde_json::to_string(v).unwrap_or_default())
}

/// An action that [`ActionSpec::runs_programs`], described.
fn what_runs(a: &ActionSpec, on_show: bool) -> What {
    match a {
        ActionSpec::Command {
            program, args, cwd, ..
        } => What::Command {
            program: program.clone(),
            args: args.clone(),
            cwd: cwd.as_ref().map(|c| c.to_string_lossy().into_owned()),
        },
        ActionSpec::Launch { program, args } => What::Launch {
            program: program.clone(),
            args: args.clone(),
        },
        ActionSpec::OpenPath { path } => What::OpenPath {
            path: path.to_string_lossy().into_owned(),
        },
        ActionSpec::Plugin {
            plugin,
            action,
            settings,
        } => What::Plugin {
            plugin: plugin.clone(),
            action: action.clone(),
            settings: settings_text(settings),
        },
        ActionSpec::SdPlugin {
            plugin,
            action,
            settings,
        } => What::SdPlugin {
            plugin: plugin.clone(),
            action: action.clone(),
            settings: settings_text(settings),
            on_show,
        },
        other => unreachable!("{other:?} doesn't run programs"),
    }
}

impl What {
    /// A short description, e.g. `runs "cmd.exe" ["/c","dir"]`, for MCP's refusal.
    pub fn summary(&self) -> String {
        let list = |v: &[String]| serde_json::to_string(v).unwrap_or_default();
        let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
        let with =
            |s: &Option<String>| s.as_ref().map(|s| format!(" with {s}")).unwrap_or_default();
        match self {
            What::Command { program, args, cwd } => format!(
                "runs {} {}{}",
                q(program),
                list(args),
                cwd.as_ref()
                    .map(|c| format!(" in {}", q(c)))
                    .unwrap_or_default()
            ),
            What::Launch { program, args } => format!("starts {} {}", q(program), list(args)),
            What::OpenPath { path } => format!("opens {}", q(path)),
            What::Plugin {
                plugin,
                action,
                settings,
            } => format!("uses the plugin {plugin}: {action}{}", with(settings)),
            What::SdPlugin {
                plugin,
                action,
                settings,
                ..
            } => format!(
                "uses the Stream Deck plugin {plugin}: {action}{}",
                with(settings)
            ),
            What::CommandSlider { program, args } => {
                format!("a slider that runs {} {}", q(program), list(args))
            }
            What::PluginSlider {
                plugin,
                action,
                settings,
            } => format!(
                "a slider that uses the plugin {plugin}: {action}{}",
                with(settings)
            ),
            What::Typing { text, .. } => format!("types {} and presses Enter", q(text)),
        }
    }
}

impl KeyDef {
    /// What an import lists for this key.
    pub fn risks(&self) -> Vec<Risk> {
        risks_of(self.action.as_ref(), &self.states, self.slider.as_ref())
    }
}

impl KeyFields {
    /// What an import lists for a key with these fields; MCP's `set_key` checks this.
    pub fn risks(&self) -> Vec<Risk> {
        risks_of(self.action.as_ref(), &self.states, self.slider.as_ref())
    }
}

impl Profile {
    /// Every key with something listed: every page in file order, reachable or not, and
    /// the keys on each by row, then column.
    pub fn risky_keys(&self) -> Vec<RiskyKey> {
        let mut out = Vec::new();
        for (i, page) in self.pages.iter().enumerate() {
            let mut keys: Vec<&KeyDef> = page.keys.iter().collect();
            keys.sort_by_key(|k| k.at);
            for k in keys {
                let risks = k.risks();
                if !risks.is_empty() {
                    out.push(RiskyKey {
                        page: page.name.clone(),
                        page_number: i + 1,
                        at: k.at,
                        title: k.look(0).title,
                        state_count: k.states.len(),
                        risks,
                    });
                }
            }
        }
        out
    }
}

/// The profile text with every listed action removed, and how many keys changed. Keys
/// keep their title, icon, colours, states and `follow`: a listed key action goes, a
/// listed state's action becomes "nothing" (so it doesn't fall back to the key's), and
/// a listed slider becomes a plain key. Fails rather than return text that still runs
/// anything.
pub fn switch_off(text: &str) -> Result<(String, usize), String> {
    let profile = Profile::parse(text)?;
    let risky = profile.risky_keys();
    if risky.is_empty() {
        return Ok((text.to_owned(), 0));
    }
    let mut changed = profile.clone();
    let mut edits = Vec::new();
    for rk in &risky {
        let key = changed.pages[rk.page_number - 1]
            .keys
            .iter_mut()
            .find(|k| k.at == rk.at)
            .ok_or(SWITCH_OFF_FAILED)?;
        for r in &rk.risks {
            match r.source {
                Source::Key => key.action = None,
                // "Nothing", not removed: a state without an action presses the key's.
                Source::State(i) => key.states[i].action = Some(ActionSpec::None),
                Source::Slider => key.slider = None,
            }
        }
        edits.push(Edit::SetKey {
            page: rk.page.clone(),
            at: rk.at,
            key: KeyFields::from(&*key),
        });
    }
    let out = match edit::apply(text, &edits) {
        Ok(out) => out,
        // Pages or keys written inline rather than as [[…]] sections: write it afresh.
        Err(EditError::Unsupported(_)) => {
            toml::to_string(&changed).map_err(|_| SWITCH_OFF_FAILED.to_owned())?
        }
        Err(e) => return Err(e.to_string()),
    };
    let check = Profile::parse(&out).map_err(|_| SWITCH_OFF_FAILED.to_owned())?;
    if !check.risky_keys().is_empty() || !check.problems().is_empty() {
        return Err(SWITCH_OFF_FAILED.into());
    }
    Ok((out, risky.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::STARTER_PROFILE;

    fn key(s: &str) -> KeyDef {
        toml::from_str(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn cmd(program: &str) -> What {
        What::Command {
            program: program.into(),
            args: vec![],
            cwd: None,
        }
    }

    fn comments(s: &str) -> Vec<&str> {
        s.lines()
            .filter(|l| l.trim_start().starts_with('#'))
            .collect()
    }

    fn find<'a>(p: &'a Profile, page: &str, at: [u32; 2]) -> &'a KeyDef {
        p.pages[p.page_index(page).unwrap()]
            .keys
            .iter()
            .find(|k| k.at == at)
            .unwrap()
    }

    fn one_page(keys: &str) -> String {
        format!("schema_version = 1\nname = \"T\"\n[[pages]]\nname = \"Main\"\n{keys}")
    }

    #[test]
    fn navigation_and_labels_are_quiet() {
        let p = Profile::parse(
            r#"
schema_version = 1
name = "Quiet"
[[pages]]
name = "Main"
keys = [
  { at = [0, 0], nav = { page = "Two" } },
  { at = [0, 1], nav = { profile = "streaming" } },
  { at = [0, 2], nav = "back" },
  { at = [0, 3], nav = "next" },
  { at = [0, 4], nav = "previous" },
  { at = [1, 0], title = "Just a label" },
  { at = [1, 1], states = [
      { action = { type = "hotkey", keys = "Ctrl+Shift+M" } },
      { action = { type = "hotkey", keys = "Ctrl+Shift+M" } } ] },
  { at = [1, 2], slider = { type = "volume" } },
  { at = [1, 3], slider = { type = "obs", input = "Mic" } },
  { at = [1, 4], slider = { type = "homeAssistant", entity = "light.desk" } },
  { at = [2, 0], action = { type = "text", text = "Hello from Perrito! " } },
  { at = [2, 1], action = { type = "openUrl", url = "https://perritodeck.dev" } },
  { at = [2, 2], action = { type = "homeAssistant", service = "lock.unlock" } },
  { at = [2, 3], action = { type = "obs", op = "startStream" } },
  { at = [2, 4], action = { type = "media", key = "mute" } },
]
[[pages]]
name = "Two"
"#,
        )
        .unwrap();
        assert!(p.problems().is_empty(), "{:?}", p.problems());
        assert_eq!(p.risky_keys(), []);
    }

    #[test]
    fn each_running_kind_is_listed() {
        let cases = [
            (
                r#"action = { type = "command", program = "cmd.exe", args = ["/c", "echo hi"], cwd = "C:/t" }"#,
                What::Command {
                    program: "cmd.exe".into(),
                    args: vec!["/c".into(), "echo hi".into()],
                    cwd: Some("C:/t".into()),
                },
            ),
            (
                r#"action = { type = "launch", program = "notepad", args = ["a.txt"] }"#,
                What::Launch {
                    program: "notepad".into(),
                    args: vec!["a.txt".into()],
                },
            ),
            (
                r#"action = { type = "openPath", path = "C:/Users/x/run.bat" }"#,
                What::OpenPath {
                    path: "C:/Users/x/run.bat".into(),
                },
            ),
            (
                r#"action = { type = "plugin", plugin = "dev.x", action = "go", settings = { a = 1 } }"#,
                What::Plugin {
                    plugin: "dev.x".into(),
                    action: "go".into(),
                    settings: Some(r#"{"a":1}"#.into()),
                },
            ),
            (
                r#"action = { type = "plugin", plugin = "dev.x", action = "go" }"#,
                What::Plugin {
                    plugin: "dev.x".into(),
                    action: "go".into(),
                    settings: None,
                },
            ),
            (
                r#"action = { type = "sdPlugin", plugin = "com.x", action = "com.x.y" }"#,
                What::SdPlugin {
                    plugin: "com.x".into(),
                    action: "com.x.y".into(),
                    settings: None,
                    on_show: true,
                },
            ),
        ];
        for (action, what) in cases {
            let risks = key(&format!("at = [0, 0]\n{action}")).risks();
            assert_eq!(
                risks,
                [Risk {
                    kind: RiskKind::Runs,
                    source: Source::Key,
                    states: vec![],
                    unused: false,
                    place: vec![],
                    what,
                }],
                "{action}"
            );
        }
    }

    #[test]
    fn sliders() {
        let k = key(r#"at = [0, 0]
slider = { type = "command", program = "brightnessctl", args = ["set", "{value}%"] }"#);
        assert_eq!(
            k.risks(),
            [Risk {
                kind: RiskKind::Runs,
                source: Source::Slider,
                states: vec![],
                unused: false,
                place: vec![],
                what: What::CommandSlider {
                    program: "brightnessctl".into(),
                    args: vec!["set".into(), "{value}%".into()],
                },
            }]
        );
        let k = key(r#"at = [0, 0]
slider = { type = "plugin", plugin = "dev.x", action = "level", follow = "v" }"#);
        let risks = k.risks();
        assert_eq!(risks.len(), 1);
        assert_eq!(risks[0].source, Source::Slider);
        assert_eq!(
            risks[0].what,
            What::PluginSlider {
                plugin: "dev.x".into(),
                action: "level".into(),
                settings: None,
            }
        );
    }

    #[test]
    fn nested_places() {
        let k = key(r#"at = [0, 0]
[action]
type = "multi"
steps = [
  { action = { type = "none" } },
  { action = { type = "random", actions = [
      { type = "media", key = "next" },
      { type = "multi", steps = [ { action = { type = "command", program = "rm" } } ] },
  ] } },
]"#);
        let risks = k.risks();
        assert_eq!(risks.len(), 1);
        assert_eq!(
            risks[0].place,
            [
                Place::Step { n: 2, of: 2 },
                Place::Choice { n: 2, of: 2 },
                Place::Step { n: 1, of: 1 },
            ]
        );
        assert_eq!(risks[0].what, cmd("rm"));

        let steps: Vec<String> = (0..50)
            .map(|i| format!("{{ action = {{ type = \"command\", program = \"p{i}\" }} }}"))
            .collect();
        let k = key(&format!(
            "at = [0, 0]\naction = {{ type = \"multi\", steps = [{}] }}",
            steps.join(", ")
        ));
        let risks = k.risks();
        assert_eq!(risks.len(), 50);
        for (i, r) in risks.iter().enumerate() {
            assert_eq!(r.what, cmd(&format!("p{i}")));
            assert_eq!(r.place, [Place::Step { n: i + 1, of: 50 }]);
        }
    }

    #[test]
    fn toggle_states() {
        // A command in state 2 only.
        let k = key(r#"at = [0, 0]
states = [ { title = "A" }, { title = "B", action = { type = "command", program = "x" } } ]"#);
        let r = k.risks();
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].source, &r[0].states[..], r[0].unused),
            (Source::State(1), &[2][..], false)
        );

        // The key's own command, used by both states.
        let k = key(r#"at = [0, 0]
action = { type = "command", program = "x" }
states = [ { title = "A" }, { title = "B" } ]"#);
        let r = k.risks();
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].source, &r[0].states[..], r[0].unused),
            (Source::Key, &[1, 2][..], false)
        );

        // Both states have their own action, so the key's is unused.
        let k = key(r#"at = [0, 0]
action = { type = "command", program = "x" }
states = [
  { action = { type = "hotkey", keys = "F1" } },
  { action = { type = "hotkey", keys = "F2" } } ]"#);
        let r = k.risks();
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].source, &r[0].states[..], r[0].unused),
            (Source::Key, &[][..], true)
        );

        // Commands in both states.
        let k = key(r#"at = [0, 0]
states = [
  { action = { type = "command", program = "a" } },
  { action = { type = "command", program = "b" } } ]"#);
        let r = k.risks();
        assert_eq!(r.len(), 2);
        assert_eq!(
            (r[0].source, &r[0].states[..]),
            (Source::State(0), &[1][..])
        );
        assert_eq!(
            (r[1].source, &r[1].states[..]),
            (Source::State(1), &[2][..])
        );

        // An ordinary key.
        let r = key("at = [0, 0]\naction = { type = \"command\", program = \"x\" }").risks();
        assert_eq!((&r[0].states[..], r[0].unused), (&[][..], false));
    }

    #[test]
    fn sd_plugin_on_show() {
        let sd = r#"{ type = "sdPlugin", plugin = "com.x", action = "com.x.y" }"#;
        let on_show = |k: &KeyDef| -> Vec<bool> {
            k.risks()
                .into_iter()
                .filter_map(|r| match r.what {
                    What::SdPlugin { on_show, .. } => Some(on_show),
                    _ => None,
                })
                .collect()
        };
        // The key's own action.
        assert_eq!(
            on_show(&key(&format!("at = [0, 0]\naction = {sd}"))),
            [true]
        );
        // State 1's own action.
        let k = key(&format!(
            "at = [0, 0]\nstates = [ {{ action = {sd} }}, {{ title = \"B\" }} ]"
        ));
        assert_eq!(on_show(&k), [true]);
        // State 2's.
        let k = key(&format!(
            "at = [0, 0]\nstates = [ {{ title = \"A\" }}, {{ action = {sd} }} ]"
        ));
        assert_eq!(on_show(&k), [false]);
        // Inside a multi.
        let k = key(&format!(
            "at = [0, 0]\naction = {{ type = \"multi\", steps = [ {{ action = {sd} }} ] }}"
        ));
        assert_eq!(on_show(&k), [false]);
        // The key's own action, when state 1 has its own.
        let k = key(&format!(
            "at = [0, 0]\naction = {sd}\nstates = [ {{ action = {{ type = \"none\" }} }}, {{ title = \"B\" }} ]"
        ));
        assert_eq!(on_show(&k), [false]);
        // ... and when state 1 falls back to it.
        let k = key(&format!(
            "at = [0, 0]\naction = {sd}\nstates = [ {{ title = \"A\" }}, {{ action = {{ type = \"none\" }} }} ]"
        ));
        assert_eq!(on_show(&k), [true]);
    }

    #[test]
    fn typing_that_presses_enter() {
        let typing = |k: &KeyDef| -> Vec<(bool, Vec<usize>)> {
            k.risks()
                .into_iter()
                .map(|r| match r.what {
                    What::Typing { enter_in_text, .. } => {
                        assert_eq!(r.kind, RiskKind::Types);
                        (enter_in_text, r.states)
                    }
                    other => panic!("{other:?}"),
                })
                .collect()
        };
        let text = |t: &str| format!("at = [0, 0]\naction = {{ type = \"text\", text = {t:?} }}");
        assert_eq!(typing(&key(&text("ls\n"))), [(true, vec![])]);
        assert_eq!(typing(&key(&text("a\r\nb"))), [(true, vec![])]);
        assert_eq!(typing(&key(&text("a\rb"))), [(true, vec![])]);
        assert_eq!(typing(&key(&text("hello"))), []);

        let with_chord = |chord: &str| {
            key(&format!(
                r#"at = [0, 0]
action = {{ type = "multi", steps = [
  {{ action = {{ type = "text", text = "hello" }} }},
  {{ action = {{ type = "hotkey", keys = "{chord}" }} }} ] }}"#
            ))
        };
        for chord in [
            "Enter",
            "Return",
            "Ctrl+Enter",
            "Shift+Enter",
            "Ctrl+M",
            "Ctrl+J",
        ] {
            assert_eq!(typing(&with_chord(chord)), [(false, vec![])], "{chord}");
        }
        for chord in ["Ctrl+Shift+M", "Ctrl+C"] {
            assert_eq!(typing(&with_chord(chord)), [], "{chord}");
        }

        // Text in state 1, Enter in state 2: the key types and presses Enter.
        let k = key(r#"at = [0, 0]
states = [
  { action = { type = "text", text = "calc" } },
  { action = { type = "hotkey", keys = "Enter" } } ]"#);
        assert_eq!(typing(&k), [(false, vec![1])]);

        // Picked at random.
        let k = key(r#"at = [0, 0]
action = { type = "random", actions = [ { type = "text", text = "gg" }, { type = "hotkey", keys = "enter" } ] }"#);
        let r = k.risks();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].place, [Place::Choice { n: 1, of: 2 }]);
        assert_eq!(
            r[0].what,
            What::Typing {
                text: "gg".into(),
                enter_in_text: false
            }
        );
    }

    #[test]
    fn key_fields_are_judged_the_same() {
        let k = key(r#"at = [0, 0]
title = "X"
action = { type = "multi", steps = [ { action = { type = "text", text = "x" } }, { action = { type = "hotkey", keys = "Enter" } } ] }"#);
        assert_eq!(KeyFields::from(&k).risks(), k.risks());
        assert_eq!(KeyFields::from(&k).risks().len(), 1);
    }

    #[test]
    fn every_page_in_order() {
        let p = Profile::parse(
            r#"
schema_version = 1
name = "Pages"
[[pages]]
name = "Main"
[[pages.keys]]
at = [1, 0]
action = { type = "command", program = "b" }
[[pages.keys]]
at = [0, 3]
action = { type = "command", program = "a" }
[[pages]]
name = "Two"
[[pages]]
name = "Nowhere"
[[pages.keys]]
at = [0, 0]
title = "Hidden"
action = { type = "launch", program = "c" }
"#,
        )
        .unwrap();
        let r = p.risky_keys();
        let got: Vec<_> = r
            .iter()
            .map(|k| (k.page.as_str(), k.page_number, k.at))
            .collect();
        assert_eq!(
            got,
            [
                ("Main", 1, [0, 3]),
                ("Main", 1, [1, 0]),
                ("Nowhere", 3, [0, 0])
            ]
        );
        assert_eq!(r[2].title, "Hidden");

        // A Companion profile's own pages are checked too.
        let p = Profile::parse(
            "schema_version = 1\nname = \"C\"\ncompanion = true\n[[pages]]\nname = \"Main\"\n\
             [[pages.keys]]\nat = [0, 0]\naction = { type = \"command\", program = \"x\" }\n",
        )
        .unwrap();
        assert_eq!(p.risky_keys().len(), 1);
    }

    #[test]
    fn starter_profile_has_one() {
        let r = Profile::parse(STARTER_PROFILE).unwrap().risky_keys();
        assert_eq!(r.len(), 1, "{r:?}");
        let k = &r[0];
        assert_eq!(
            (
                k.page.as_str(),
                k.page_number,
                k.at,
                k.title.as_str(),
                k.state_count
            ),
            ("More", 2, [0, 1], "Run a\ncommand", 0)
        );
        assert_eq!(k.risks.len(), 1);
        assert_eq!(k.risks[0].what, cmd("hostname"));
    }

    #[test]
    fn switch_off_keeps_looks() {
        let (out, n) = switch_off(STARTER_PROFILE).unwrap();
        assert_eq!(n, 1);
        let p = Profile::parse(&out).unwrap();
        assert_eq!(p.risky_keys(), []);
        let k = find(&p, "More", [0, 1]);
        assert_eq!(k.title.as_deref(), Some("Run a\ncommand"));
        assert_eq!(k.action, None);
        assert_eq!(comments(&out), comments(STARTER_PROFILE));
        // Every other key is as it was.
        let before = Profile::parse(STARTER_PROFILE).unwrap();
        for (a, b) in before.pages.iter().zip(&p.pages) {
            for ka in a
                .keys
                .iter()
                .filter(|k| !(a.name == "More" && k.at == [0, 1]))
            {
                assert_eq!(Some(ka), b.keys.iter().find(|kb| kb.at == ka.at));
            }
        }
    }

    #[test]
    fn switch_off_keeps_toggles() {
        let text = one_page(
            r##"[[pages.keys]]
at = [0, 0]
title = "Three"
action = { type = "hotkey", keys = "F5" }
follow = "obs:recording"
states = [
  { title = "One" },
  { title = "Two", bg = "#112233", action = { type = "command", program = "x" } },
]
"##,
        );
        // follow needs exactly two states; this toggle has two.
        assert!(Profile::parse(&text).unwrap().problems().is_empty());
        let (out, n) = switch_off(&text).unwrap();
        assert_eq!(n, 1);
        let p = Profile::parse(&out).unwrap();
        let k = find(&p, "Main", [0, 0]);
        assert_eq!(k.states.len(), 2);
        assert_eq!(k.states[0].title.as_deref(), Some("One"));
        assert_eq!(k.states[0].action, None);
        assert_eq!(k.states[1].title.as_deref(), Some("Two"));
        assert_eq!(k.states[1].bg.as_deref(), Some("#112233"));
        assert_eq!(k.states[1].action, Some(ActionSpec::None));
        assert_eq!(k.action, Some(ActionSpec::Hotkey { keys: "F5".into() }));
        assert_eq!(k.follow.as_deref(), Some("obs:recording"));
        assert_eq!(k.title.as_deref(), Some("Three"));
        // Pressing state 2 now does nothing, rather than falling back to F5.
        assert_eq!(k.action(1), Some(&ActionSpec::None));
        assert_eq!(k.action(0), Some(&ActionSpec::Hotkey { keys: "F5".into() }));
    }

    #[test]
    fn switch_off_a_three_state_toggle_keeps_the_other_states() {
        let text = one_page(
            r#"[[pages.keys]]
at = [0, 0]
states = [
  { title = "1", action = { type = "hotkey", keys = "F1" } },
  { title = "2", action = { type = "launch", program = "x" } },
  { title = "3", action = { type = "hotkey", keys = "F3" } },
]
"#,
        );
        let (out, _) = switch_off(&text).unwrap();
        let p = Profile::parse(&out).unwrap();
        let k = find(&p, "Main", [0, 0]);
        assert_eq!(k.states.len(), 3);
        assert_eq!(
            k.states[0].action,
            Some(ActionSpec::Hotkey { keys: "F1".into() })
        );
        assert_eq!(k.states[1].action, Some(ActionSpec::None));
        assert_eq!(
            k.states[2].action,
            Some(ActionSpec::Hotkey { keys: "F3".into() })
        );
    }

    #[test]
    fn switch_off_unused_key_action() {
        let text = one_page(
            r#"[[pages.keys]]
at = [0, 0]
action = { type = "command", program = "x" }
states = [
  { action = { type = "hotkey", keys = "F1" } },
  { action = { type = "hotkey", keys = "F2" } },
]
"#,
        );
        let (out, n) = switch_off(&text).unwrap();
        assert_eq!(n, 1);
        let p = Profile::parse(&out).unwrap();
        let k = find(&p, "Main", [0, 0]);
        assert_eq!(k.action, None);
        assert_eq!(
            k.states[0].action,
            Some(ActionSpec::Hotkey { keys: "F1".into() })
        );
        assert_eq!(
            k.states[1].action,
            Some(ActionSpec::Hotkey { keys: "F2".into() })
        );
    }

    #[test]
    fn switch_off_slider_becomes_a_key() {
        let text = one_page(
            r#"[[pages.keys]]
at = [0, 0]
title = "Bright"
slider = { type = "command", program = "brightnessctl", args = ["set", "{value}%"] }
[[pages.keys]]
at = [0, 1]
title = "Vol"
slider = { type = "volume" }
"#,
        );
        let (out, n) = switch_off(&text).unwrap();
        assert_eq!(n, 1);
        let p = Profile::parse(&out).unwrap();
        let k = find(&p, "Main", [0, 0]);
        assert_eq!((k.title.as_deref(), &k.slider), (Some("Bright"), &None));
        assert_eq!(find(&p, "Main", [0, 1]).slider, Some(SliderSpec::Volume));
    }

    #[test]
    fn switch_off_typing() {
        let text = one_page(
            r#"[[pages.keys]]
at = [0, 0]
title = "Run box"
[pages.keys.action]
type = "multi"
steps = [
  { action = { type = "hotkey", keys = "Win+R" } },
  { action = { type = "text", text = "cmd" } },
  { action = { type = "hotkey", keys = "Enter" } },
]
"#,
        );
        let (out, n) = switch_off(&text).unwrap();
        assert_eq!(n, 1);
        let p = Profile::parse(&out).unwrap();
        assert_eq!(find(&p, "Main", [0, 0]).action, None);
        assert_eq!(p.risky_keys(), []);
    }

    #[test]
    fn switch_off_inline_pages_falls_back() {
        let text = r#"schema_version = 1
name = "Inline"
pages = [{ name = "Main", keys = [{ at = [0,0], title = "X", action = { type = "command", program = "x" } }] }]
"#;
        let (out, n) = switch_off(text).unwrap();
        assert_eq!(n, 1);
        let p = Profile::parse(&out).unwrap();
        assert!(p.problems().is_empty());
        assert_eq!(p.risky_keys(), []);
        assert_eq!(find(&p, "Main", [0, 0]).title.as_deref(), Some("X"));
    }

    #[test]
    fn switch_off_without_risky_keys_is_unchanged() {
        let text = format!(
            "# kept\n{}",
            one_page(
                "[[pages.keys]]\nat = [0, 0]\naction = { type = \"hotkey\", keys = \"Ctrl+C\" }\n"
            )
        );
        assert_eq!(switch_off(&text).unwrap(), (text.clone(), 0));
    }
}
