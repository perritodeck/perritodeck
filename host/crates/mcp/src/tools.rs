//! The tools an assistant sees, their JSON Schemas, and what calling them does.

use std::path::{Component, Path};

use perritodeck_actions::{ActionSpec, Chord, SliderSpec};
use perritodeck_profile::{edit::Edit, is_color, NavSpec, StateDef};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::{
    jsonrpc::RpcError, Backend, KeyDetail, KeyFields, PageDetail, Permissions, Press,
    ProfileDetail, ProfileInfo, TabletInfo,
};

/// What a tool call returns: text for the assistant, plus the same data as JSON where
/// that's useful.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolOutput {
    text: String,
    structured: Option<Value>,
    is_error: bool,
}

impl ToolOutput {
    fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            structured: None,
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            structured: None,
            is_error: true,
        }
    }

    fn with(mut self, structured: Value) -> Self {
        self.structured = Some(structured);
        self
    }

    pub fn into_result(self, structured_ok: bool) -> Value {
        let mut r = json!({
            "content": [{ "type": "text", "text": self.text }],
            "isError": self.is_error,
        });
        if let (true, Some(s)) = (structured_ok, self.structured) {
            r["structuredContent"] = s;
        }
        r
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Read,
    Press,
    Edit,
    Run,
}

impl Group {
    fn allowed(self, p: Permissions) -> bool {
        match self {
            Group::Read => true,
            Group::Press => p.press,
            Group::Edit => p.edit,
            Group::Run => p.run,
        }
    }

    fn refusal(self, tool: &str) -> String {
        let what = match self {
            Group::Read => unreachable!("reading is always allowed"),
            Group::Press => "press keys",
            Group::Edit => "edit keys",
            Group::Run => "run actions",
        };
        format!(
            "PerritoDeck isn't letting assistants {what}, so {tool} isn't available. The person \
             can allow it in PerritoDeck's settings, under AI assistants (\"{what}\")."
        )
    }
}

struct ToolDef {
    name: &'static str,
    title: &'static str,
    group: Group,
    description: String,
    input: Value,
    annotations: Value,
}

impl ToolDef {
    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input,
            "annotations": self.annotations,
        })
    }
}

const POSITIONS: &str = "Positions are [row, col], counted from 0 at the top left: [0,0] is \
the top-left key, [0,1] the one to its right, [1,0] the first key of the second row.";

const TITLES: &str = "Titles may contain \\n for a line break and live values in braces, kept \
up to date on the tablet: {time} (21:07), {time_s} (21:07:45), {date} (Sun 11 Oct), {cpu} and \
{mem} (percent used), {ha:<entity>} (a Home Assistant state with its unit, e.g. \
{ha:sensor.outside}) or {ha:<entity>.<attribute>}, {obs:scene}, {obs:recording}, \
{obs:streaming}, and {plugin:<plugin id>:<value id>}. list_live_values shows the ones known now.";

const ACTIONS: &str = "Action types (JSON with a \"type\"): hotkey {keys:\"Ctrl+Shift+M\"}; \
text {text} (typed exactly); media {key: playPause|next|previous|stop|volumeUp|volumeDown|mute}; \
openUrl {url} (http, https or mailto); openPath {path} (a file or folder, opened with its app); \
launch {program, args} (starts a program); command {program, args, cwd?, timeoutMs?} (runs a \
program and waits; never through a shell, so args are separate strings); multi {steps: \
[{action, delayMs}]} (several actions in order); random {actions: [action, …]} (runs one of \
them, picked at random on each press); obs {op: scene (+scene) | toggleRecord | \
startRecord | stopRecord | toggleStream | startStream | stopStream | toggleMute (+input) | \
toggleReplayBuffer | saveReplayBuffer}; homeAssistant {service:\"light.toggle\", entity?, data?}; \
plugin {plugin, action, settings?}; sdPlugin {plugin, action, settings?} (Stream Deck plugins); \
none.";

const HOTKEYS: &str = "Hotkeys: modifiers and one key joined by + (spaces around it are fine; \
\"Ctrl + +\" is Ctrl and the plus key). Modifiers: Ctrl, Alt (Option), Shift, Cmd (Win, Super, \
Meta). Keys: a single character, F1-F24, Enter, Tab, Space, Escape, Backspace, Delete, Insert, \
Home, End, PageUp, PageDown, Up, Down, Left, Right, CapsLock, Plus, Minus, or punctuation by \
name: Slash, Backslash, Comma, Period, Semicolon, Quote, Backquote, BracketLeft, BracketRight, \
Equal. A character means the key that types it on the keyboard layout in use. On Windows, a \
symbol brings the Shift or AltGr it needs (\"ctrl+!\" sends Ctrl+Shift+1 on a US keyboard), \
except Plus, which is the =/+ key without Shift: Ctrl+Plus is zoom in, and a lone Plus types \
\"=\" on a US keyboard. Also on Windows, letters and digits go by Windows' own key codes, as \
its shortcuts do, so Ctrl+C works on Cyrillic and other layouts too. On Linux and macOS, name \
a symbol's key without Shift and add Shift yourself (Ctrl+Shift+/, not Ctrl+?), which works on \
Windows too.";

fn defs() -> Vec<ToolDef> {
    let read = json!({ "readOnlyHint": true, "openWorldHint": false });
    vec![
        ToolDef {
            name: "list_profiles",
            title: "List profiles",
            group: Group::Read,
            description:
                "List PerritoDeck's profiles (sets of key pages). Tablets show the \
                default profile unless they've been given another. Use the id with the other tools."
                    .into(),
            input: no_args(),
            annotations: read.clone(),
        },
        ToolDef {
            name: "get_profile",
            title: "Show a profile",
            group: Group::Read,
            description: format!(
                "Show a profile's grid size, pages and every key: its position, title, what it \
                 does, and its settings as JSON (the same shape set_key takes). {POSITIONS}"
            ),
            input: object(json!({ "profile": profile_prop() }), &[]),
            annotations: read.clone(),
        },
        ToolDef {
            name: "list_tablets",
            title: "List tablets",
            group: Group::Read,
            description: "List the paired tablets: whether each is connected, and which \
                profile and page it shows."
                .into(),
            input: no_args(),
            annotations: read.clone(),
        },
        ToolDef {
            name: "list_live_values",
            title: "List live values",
            group: Group::Read,
            description: format!(
                "List the live values key titles can show, with their current values. {TITLES}"
            ),
            input: no_args(),
            annotations: read,
        },
        ToolDef {
            name: "press_key",
            title: "Press a key",
            group: Group::Press,
            description: format!(
                "Press an existing key as if someone tapped it on a tablet. An action key runs \
                 its action; a toggle runs its current state's action and moves to the next \
                 state; a key that opens a page (or goes back, next or previous) switches the \
                 page on the tablet given, or on every connected tablet showing this profile. \
                 Sliders can't be pressed. Look up the key with get_profile first. {POSITIONS}"
            ),
            input: object(
                json!({
                    "profile": profile_prop(),
                    "page": page_prop(),
                    "row": index_prop("Row, from 0 at the top."),
                    "col": index_prop("Column, from 0 at the left."),
                    "tablet": { "type": "string", "description": "Tablet id or name (from list_tablets). Only matters for keys that change the page." },
                }),
                &["row", "col"],
            ),
            annotations: json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true }),
        },
        ToolDef {
            name: "show_profile",
            title: "Show a profile on a tablet",
            group: Group::Press,
            description: "Switch a connected tablet to a profile now (like auto-switching; the \
                profile the tablet is assigned in PerritoDeck doesn't change)."
                .into(),
            input: object(
                json!({
                    "profile": { "type": "string", "description": "Profile id or name, from list_profiles." },
                    "tablet": { "type": "string", "description": "Tablet id or name, from list_tablets. May be left out when only one tablet is connected." },
                }),
                &["profile"],
            ),
            annotations: json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }),
        },
        ToolDef {
            name: "set_key",
            title: "Create or change a key",
            group: Group::Edit,
            description: format!(
                "Create a key, or change one, at a position on a page. Tablets update at once, \
                 and the person can undo it in the PerritoDeck app. Only the fields you give \
                 change; give a field as null to remove it; replace: true starts from an empty \
                 key instead. A key does one thing: giving action, nav, slider or states \
                 replaces whichever of those it had. {POSITIONS} Keep titles short (a tablet \
                 key is small; about 8 characters a line). {TITLES} {ACTIONS} {HOTKEYS} Colours \
                 are \"#RRGGBB\". Actions that run programs or plugins (command, launch, \
                 openPath, plugin, sdPlugin, and command or plugin sliders) also need the \
                 person to allow running actions."
            ),
            input: with_defs(object(
                json!({
                    "profile": profile_prop(),
                    "page": page_prop(),
                    "row": index_prop("Row, from 0 at the top."),
                    "col": index_prop("Column, from 0 at the left."),
                    "title": nullable_string("The text on the key."),
                    "bg": nullable_string("Background colour, \"#RRGGBB\"."),
                    "fg": nullable_string("Text colour, \"#RRGGBB\"."),
                    "action": { "anyOf": [{ "$ref": "#/$defs/action" }, { "type": "null" }], "description": "What pressing the key does." },
                    "nav": { "anyOf": [{ "$ref": "#/$defs/nav" }, { "type": "null" }], "description": "Make the key open a page instead: {\"page\":\"Media\"}, or \"back\", \"next\", \"previous\"; or switch the tablet to another profile: {\"profile\":\"streaming\"}, optionally with \"page\"." },
                    "slider": { "anyOf": [{ "$ref": "#/$defs/slider" }, { "type": "null" }], "description": "Make the cell a slider (dragged up and down on tablets that support it)." },
                    "states": { "anyOf": [{ "type": "array", "items": { "$ref": "#/$defs/state" }, "minItems": 2 }, { "type": "null" }], "description": "Two or more states make a toggle: each press runs the current state's action, then shows the next state." },
                    "follow": nullable_string("For a two-state toggle: show the second state while this live value is on (e.g. \"obs:recording\", \"ha:light.desk\") instead of flipping on each press."),
                    "replace": { "type": "boolean", "description": "Start from an empty key instead of changing the existing one.", "default": false },
                }),
                &["row", "col"],
            )),
            annotations: json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": false }),
        },
        ToolDef {
            name: "clear_key",
            title: "Remove a key",
            group: Group::Edit,
            description: format!(
                "Remove the key at a position, leaving the cell empty. {POSITIONS}"
            ),
            input: object(
                json!({
                    "profile": profile_prop(),
                    "page": page_prop(),
                    "row": index_prop("Row, from 0 at the top."),
                    "col": index_prop("Column, from 0 at the left."),
                }),
                &["row", "col"],
            ),
            annotations: json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false }),
        },
        ToolDef {
            name: "move_key",
            title: "Move a key",
            group: Group::Edit,
            description: format!(
                "Move a key to another position, on the same page or another one. If a key is \
                 already there, the two swap places. {POSITIONS}"
            ),
            input: object(
                json!({
                    "profile": profile_prop(),
                    "page": page_prop(),
                    "from": position_prop("Where the key is now, [row, col]."),
                    "to": position_prop("Where it goes, [row, col]."),
                    "to_page": { "type": "string", "description": "The page it goes to. Leave out to stay on the same page." },
                }),
                &["from", "to"],
            ),
            annotations: json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }),
        },
        ToolDef {
            name: "add_page",
            title: "Add a page",
            group: Group::Edit,
            description: "Add an empty page to a profile. To reach it from a tablet, also add a \
                key with nav {\"page\": \"<name>\"} on another page, and usually a \"back\" key \
                on the new page."
                .into(),
            input: object(
                json!({
                    "profile": profile_prop(),
                    "name": { "type": "string", "minLength": 1, "description": "The new page's name." },
                }),
                &["name"],
            ),
            annotations: json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }),
        },
        ToolDef {
            name: "rename_page",
            title: "Rename a page",
            group: Group::Edit,
            description: "Rename a page. Keys that open it are updated too.".into(),
            input: object(
                json!({
                    "profile": profile_prop(),
                    "from": { "type": "string", "description": "The page's name now." },
                    "to": { "type": "string", "minLength": 1, "description": "Its new name." },
                }),
                &["from", "to"],
            ),
            annotations: json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }),
        },
        ToolDef {
            name: "remove_page",
            title: "Remove a page",
            group: Group::Edit,
            description: "Remove a page and its keys. A page that a key still opens can't be \
                removed; change or clear that key first."
                .into(),
            input: object(
                json!({
                    "profile": profile_prop(),
                    "name": { "type": "string", "description": "The page to remove." },
                }),
                &["name"],
            ),
            annotations: json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false }),
        },
        ToolDef {
            name: "create_profile",
            title: "Create a profile",
            group: Group::Edit,
            description: "Create a new, empty profile and return its id. Add keys with set_key; \
                show it on a tablet with show_profile."
                .into(),
            input: object(
                json!({ "name": { "type": "string", "minLength": 1, "description": "The profile's name." } }),
                &["name"],
            ),
            annotations: json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }),
        },
        ToolDef {
            name: "run_action",
            title: "Run an action",
            group: Group::Run,
            description: format!(
                "Run an action on the person's computer right now, without a key: press a \
                 hotkey, type text, control media, OBS or Home Assistant, open a URL, or run a \
                 program. {ACTIONS} {HOTKEYS}"
            ),
            input: with_defs(object(
                json!({ "action": { "$ref": "#/$defs/action" } }),
                &["action"],
            )),
            annotations: json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true }),
        },
    ]
}

/// The tools the permissions allow, in a fixed order.
pub(crate) fn list(perms: Permissions) -> Vec<Value> {
    defs()
        .iter()
        .filter(|d| d.group.allowed(perms))
        .map(ToolDef::to_json)
        .collect()
}

/// Call a tool. Unknown tools are a protocol error; everything else (not allowed, bad
/// arguments, the host saying no) is a tool error the assistant can act on.
pub(crate) async fn call(
    b: &dyn Backend,
    name: &str,
    args: Map<String, Value>,
) -> Result<ToolOutput, RpcError> {
    let all = defs();
    let def = all
        .iter()
        .find(|d| d.name == name)
        .ok_or_else(|| RpcError::invalid_params(format!("Unknown tool: {name}")))?;
    let perms = b.permissions();
    if !def.group.allowed(perms) {
        return Ok(ToolOutput::error(def.group.refusal(name)));
    }
    let args = Args(args);
    if let Err(e) = args.only(&def.input) {
        return Ok(ToolOutput::error(e));
    }
    let out = match name {
        "list_profiles" => Ok(list_profiles(b)),
        "get_profile" => get_profile(b, &args),
        "list_tablets" => Ok(list_tablets(b)),
        "list_live_values" => Ok(list_live_values(b)),
        "press_key" => press_key(b, &args).await,
        "show_profile" => show_profile(b, &args),
        "set_key" => set_key(b, &args, perms),
        "clear_key" => clear_key(b, &args),
        "move_key" => move_key(b, &args),
        "add_page" => add_page(b, &args),
        "rename_page" => rename_page(b, &args),
        "remove_page" => remove_page(b, &args),
        "create_profile" => create_profile(b, &args),
        "run_action" => run_action(b, &args).await,
        _ => unreachable!("every defined tool is handled"),
    };
    Ok(out.unwrap_or_else(ToolOutput::error))
}

// ---- Schemas ----

fn no_args() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn object(props: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false,
    })
}

fn with_defs(mut schema: Value) -> Value {
    schema["$defs"] = json!({
        "action": action_schema(),
        "nav": {
            "anyOf": [
                { "type": "string", "enum": ["back", "next", "previous"] },
                { "type": "object", "description": "Open a page of this profile.", "properties": { "page": { "type": "string" } }, "required": ["page"], "additionalProperties": false },
                { "type": "object", "description": "Switch the tablet it's pressed on to another profile (by id, from list_profiles), optionally at one of its pages; remembered like choosing the profile in the app.", "properties": { "profile": { "type": "string" }, "page": { "type": "string" } }, "required": ["profile"], "additionalProperties": false },
            ]
        },
        "slider": slider_schema(),
        "state": {
            "type": "object",
            "description": "One state of a toggle. Fields left out use the key's own.",
            "properties": {
                "title": { "type": "string" },
                "bg": { "type": "string", "description": "\"#RRGGBB\"" },
                "fg": { "type": "string", "description": "\"#RRGGBB\"" },
                "action": { "$ref": "#/$defs/action" },
            },
            "additionalProperties": false,
        },
    });
    schema
}

fn profile_prop() -> Value {
    json!({ "type": "string", "description": "Profile id or name, from list_profiles. Leave out for the default profile." })
}

fn page_prop() -> Value {
    json!({ "type": "string", "description": "Page name. Leave out for the profile's first page." })
}

fn index_prop(description: &str) -> Value {
    json!({ "type": "integer", "minimum": 0, "description": description })
}

fn position_prop(description: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": "integer", "minimum": 0 },
        "minItems": 2,
        "maxItems": 2,
        "description": description,
    })
}

fn nullable_string(description: &str) -> Value {
    json!({ "type": ["string", "null"], "description": description })
}

fn variant(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    let mut props = props;
    props["type"] = json!({ "const": name });
    let mut req = vec!["type"];
    req.extend_from_slice(required);
    json!({
        "type": "object",
        "description": description,
        "properties": props,
        "required": req,
        "additionalProperties": false,
    })
}

fn strings() -> Value {
    json!({ "type": "array", "items": { "type": "string" } })
}

pub(crate) fn action_schema() -> Value {
    json!({
        "description": "An action. Examples: {\"type\":\"hotkey\",\"keys\":\"Ctrl+Shift+M\"}, {\"type\":\"media\",\"key\":\"playPause\"}, {\"type\":\"obs\",\"op\":\"scene\",\"scene\":\"Gameplay\"}.",
        "oneOf": [
            variant("hotkey", "Press a key combination.", json!({ "keys": { "type": "string", "description": "e.g. \"Ctrl+Shift+M\", \"Cmd+Space\", \"F13\"" } }), &["keys"]),
            variant("text", "Type text exactly as given.", json!({ "text": { "type": "string" } }), &["text"]),
            variant("media", "A media key.", json!({ "key": { "enum": ["playPause", "next", "previous", "stop", "volumeUp", "volumeDown", "mute"] } }), &["key"]),
            variant("openUrl", "Open a URL in the default browser (http, https and mailto only).", json!({ "url": { "type": "string" } }), &["url"]),
            variant("openPath", "Open a file or folder with its default app.", json!({ "path": { "type": "string" } }), &["path"]),
            variant("launch", "Start a program and return at once. Never goes through a shell.", json!({ "program": { "type": "string" }, "args": strings() }), &["program"]),
            variant("command", "Run a program and wait for it (default 10 s, at most 600000 ms); a non-zero exit shows an alert. Never goes through a shell: program and each argument are separate strings.", json!({
                "program": { "type": "string" },
                "args": strings(),
                "cwd": { "type": "string" },
                "timeoutMs": { "type": "integer", "minimum": 1, "maximum": 600000 },
            }), &["program"]),
            variant("multi", "Several actions in order, each optionally followed by a delay (at most 60000 ms).", json!({
                "steps": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": { "$ref": "#/$defs/action" },
                            "delayMs": { "type": "integer", "minimum": 0, "maximum": 60000 },
                        },
                        "required": ["action"],
                        "additionalProperties": false,
                    },
                },
            }), &["steps"]),
            variant("random", "Run one of these actions, picked at random on each press.", json!({
                "actions": { "type": "array", "items": { "$ref": "#/$defs/action" }, "minItems": 1 },
            }), &["actions"]),
            variant("obs", "Control OBS Studio (set up in PerritoDeck's integrations).", json!({
                "op": { "enum": ["scene", "toggleRecord", "startRecord", "stopRecord", "toggleStream", "startStream", "stopStream", "toggleMute", "toggleReplayBuffer", "saveReplayBuffer"] },
                "scene": { "type": "string", "description": "For op \"scene\"." },
                "input": { "type": "string", "description": "For op \"toggleMute\", e.g. \"Mic/Aux\"." },
            }), &["op"]),
            variant("homeAssistant", "Call a Home Assistant service (set up in PerritoDeck's integrations).", json!({
                "service": { "type": "string", "description": "domain.service, e.g. \"light.toggle\", \"scene.turn_on\"" },
                "entity": { "type": "string", "description": "e.g. \"light.desk\"" },
                "data": { "type": "object", "description": "Extra service data, e.g. {\"brightness_pct\": 40}" },
            }), &["service"]),
            variant("plugin", "An action from an installed PerritoDeck plugin.", json!({
                "plugin": { "type": "string" },
                "action": { "type": "string" },
                "settings": { "type": "object" },
            }), &["plugin", "action"]),
            variant("sdPlugin", "An action from an installed Stream Deck plugin (keys only).", json!({
                "plugin": { "type": "string", "description": "The plugin's UUID" },
                "action": { "type": "string", "description": "The action's UUID" },
                "settings": { "type": "object" },
            }), &["plugin", "action"]),
            variant("none", "Do nothing.", json!({}), &[]),
        ]
    })
}

fn slider_schema() -> Value {
    json!({
        "oneOf": [
            variant("volume", "The computer's main output volume.", json!({}), &[]),
            variant("homeAssistant", "A Home Assistant light's brightness.", json!({ "entity": { "type": "string" } }), &["entity"]),
            variant("obs", "An OBS audio source's volume.", json!({ "input": { "type": "string" } }), &["input"]),
            variant("command", "Run a program with {value} (0-100) replaced in its arguments.", json!({ "program": { "type": "string" }, "args": strings() }), &["program"]),
            variant("plugin", "A plugin action that takes a value 0-100.", json!({
                "plugin": { "type": "string" },
                "action": { "type": "string" },
                "settings": { "type": "object" },
                "follow": { "type": "string" },
            }), &["plugin", "action"]),
        ]
    })
}

// ---- Arguments ----

struct Args(Map<String, Value>);

impl Args {
    /// Refuse arguments the tool doesn't take, so typos don't silently do nothing.
    fn only(&self, schema: &Value) -> Result<(), String> {
        let props = schema["properties"].as_object();
        let unknown: Vec<&str> = self
            .0
            .keys()
            .filter(|k| !props.is_some_and(|p| p.contains_key(*k)))
            .map(String::as_str)
            .collect();
        if unknown.is_empty() {
            return Ok(());
        }
        let known: Vec<&str> = props
            .map(|p| p.keys().map(String::as_str).collect())
            .unwrap_or_default();
        Err(format!(
            "Unknown argument{} {}. This tool takes: {}.",
            if unknown.len() == 1 { "" } else { "s" },
            unknown.join(", "),
            if known.is_empty() {
                "nothing".to_owned()
            } else {
                known.join(", ")
            }
        ))
    }

    fn has(&self, k: &str) -> bool {
        self.0.contains_key(k)
    }

    fn opt_str(&self, k: &str) -> Result<Option<String>, String> {
        match self.0.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("\"{k}\" must be a string.")),
        }
    }

    fn req_str(&self, k: &str) -> Result<String, String> {
        self.opt_str(k)?
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("\"{k}\" is required."))
    }

    fn index(&self, k: &str) -> Result<u32, String> {
        let v = self
            .0
            .get(k)
            .ok_or_else(|| format!("\"{k}\" is required."))?;
        to_index(v).ok_or_else(|| format!("\"{k}\" must be a whole number from 0."))
    }

    fn position(&self, k: &str) -> Result<[u32; 2], String> {
        let bad = || format!("\"{k}\" must be [row, col], two whole numbers from 0.");
        let v = self
            .0
            .get(k)
            .ok_or_else(|| format!("\"{k}\" is required."))?;
        match v.as_array().map(Vec::as_slice) {
            Some([r, c]) => Ok([to_index(r).ok_or_else(bad)?, to_index(c).ok_or_else(bad)?]),
            _ => Err(bad()),
        }
    }

    fn opt_bool(&self, k: &str) -> Result<Option<bool>, String> {
        match self.0.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(format!("\"{k}\" must be true or false.")),
        }
    }

    fn typed<T: DeserializeOwned>(&self, k: &str) -> Result<Option<T>, String> {
        match self.0.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => serde_json::from_value(v.clone())
                .map(Some)
                .map_err(|e| format!("\"{k}\" isn't valid: {e}.")),
        }
    }
}

fn to_index(v: &Value) -> Option<u32> {
    v.as_u64()
        .or_else(|| {
            v.as_f64()
                .filter(|f| f.fract() == 0.0 && *f >= 0.0)
                .map(|f| f as u64)
        })
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n < 1000)
}

// ---- Looking things up ----

fn resolve_profile(b: &dyn Backend, arg: Option<String>) -> Result<ProfileInfo, String> {
    let all = b.list_profiles();
    let found = match &arg {
        None => all.iter().find(|p| p.is_default).or(all.first()),
        Some(want) => all.iter().find(|p| &p.id == want).or_else(|| {
            all.iter()
                .find(|p| p.name.eq_ignore_ascii_case(want) || p.id.eq_ignore_ascii_case(want))
        }),
    };
    found.cloned().ok_or_else(|| {
        let names: Vec<String> = all
            .iter()
            .map(|p| format!("{} (\"{}\")", p.id, p.name))
            .collect();
        match arg {
            Some(want) => format!(
                "There's no profile \"{want}\". Profiles: {}.",
                if names.is_empty() {
                    "none".into()
                } else {
                    names.join(", ")
                }
            ),
            None => "There are no profiles.".into(),
        }
    })
}

fn load_profile(b: &dyn Backend, arg: Option<String>) -> Result<ProfileDetail, String> {
    let info = resolve_profile(b, arg)?;
    b.get_profile(&info.id)
}

fn editable(p: &ProfileDetail) -> Result<(), String> {
    if p.companion {
        return Err(format!(
            "Profile \"{}\" shows Bitfocus Companion's buttons; those are set up in Companion, \
             not in PerritoDeck.",
            p.name
        ));
    }
    Ok(())
}

fn resolve_page<'a>(p: &'a ProfileDetail, arg: Option<&str>) -> Result<&'a PageDetail, String> {
    let found = match arg {
        None => p.pages.first(),
        Some(want) => p
            .pages
            .iter()
            .find(|pg| pg.name == want)
            .or_else(|| p.pages.iter().find(|pg| pg.name.eq_ignore_ascii_case(want))),
    };
    found.ok_or_else(|| {
        let names: Vec<String> = p
            .pages
            .iter()
            .map(|pg| format!("\"{}\"", pg.name))
            .collect();
        match arg {
            Some(want) => format!(
                "Profile \"{}\" has no page \"{want}\". Its pages: {}.",
                p.name,
                if names.is_empty() {
                    "none".into()
                } else {
                    names.join(", ")
                }
            ),
            None => format!("Profile \"{}\" has no pages.", p.name),
        }
    })
}

fn key_at(page: &PageDetail, at: [u32; 2]) -> Option<&KeyDetail> {
    page.keys.iter().find(|k| [k.row, k.col] == at)
}

fn resolve_tablet(b: &dyn Backend, arg: Option<String>) -> Result<TabletInfo, String> {
    let all = b.list_tablets();
    match arg {
        Some(want) => all
            .iter()
            .find(|t| t.id == want)
            .or_else(|| all.iter().find(|t| t.name.eq_ignore_ascii_case(&want)))
            .cloned()
            .ok_or_else(|| {
                let names: Vec<String> = all
                    .iter()
                    .map(|t| format!("\"{}\" ({})", t.name, t.id))
                    .collect();
                format!(
                    "There's no tablet \"{want}\". Tablets: {}.",
                    if names.is_empty() {
                        "none paired".into()
                    } else {
                        names.join(", ")
                    }
                )
            }),
        None => {
            let connected: Vec<&TabletInfo> = all.iter().filter(|t| t.connected).collect();
            match connected.as_slice() {
                [one] => Ok((*one).clone()),
                [] => Err("No tablet is connected.".into()),
                _ => Err(format!(
                    "Several tablets are connected; say which: {}.",
                    connected
                        .iter()
                        .map(|t| format!("\"{}\"", t.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
    }
}

// ---- Describing things ----

pub(crate) fn describe_action(a: &ActionSpec) -> String {
    match a {
        ActionSpec::Hotkey { keys } => format!("hotkey {keys}"),
        ActionSpec::Text { text } => format!("types {}", quote(text)),
        ActionSpec::Media { key } => format!("media {}", json_name(key)),
        ActionSpec::OpenUrl { url } => format!("opens {url}"),
        ActionSpec::OpenPath { path } => format!("opens {}", path.display()),
        ActionSpec::Launch { program, .. } => format!("starts {program}"),
        ActionSpec::Command { program, args, .. } => {
            format!(
                "runs {}",
                std::iter::once(program)
                    .chain(args)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
        ActionSpec::Multi { steps } => format!("{} steps", steps.len()),
        ActionSpec::Random { actions } => format!("one of {} at random", actions.len()),
        ActionSpec::Obs(op) => format!(
            "OBS {}",
            serde_json::to_value(op)
                .ok()
                .and_then(|v| v.get("op").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_default()
        ),
        ActionSpec::HomeAssistant {
            service, entity, ..
        } => match entity {
            Some(e) => format!("Home Assistant {service} {e}"),
            None => format!("Home Assistant {service}"),
        },
        ActionSpec::Plugin { plugin, action, .. } => format!("plugin {plugin}: {action}"),
        ActionSpec::SdPlugin { plugin, action, .. } => {
            format!("Stream Deck plugin {plugin}: {action}")
        }
        ActionSpec::None => "nothing".into(),
    }
}

fn json_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn quote(s: &str) -> String {
    let short: String = s.chars().take(40).collect();
    let more = if s.chars().count() > 40 { "…" } else { "" };
    format!("\"{}{more}\"", short.replace('\n', "\\n"))
}

fn describe_fields(f: &KeyFields) -> String {
    if let Some(s) = &f.slider {
        return format!("slider: {}", s.describe());
    }
    if let Some(nav) = &f.nav {
        return match nav {
            NavSpec::Page { page } => format!("opens page \"{page}\""),
            NavSpec::Profile {
                profile,
                page: Some(page),
            } => format!("switches the tablet to profile \"{profile}\", page \"{page}\""),
            NavSpec::Profile { profile, .. } => {
                format!("switches the tablet to profile \"{profile}\"")
            }
            NavSpec::Simple(s) => format!("{} page", json_name(s)),
        };
    }
    if f.states.len() >= 2 {
        return format!("toggle with {} states", f.states.len());
    }
    f.action
        .as_ref()
        .map(describe_action)
        .unwrap_or_else(|| "no action".into())
}

fn key_title(f: &KeyFields) -> String {
    f.title
        .as_ref()
        .or_else(|| f.states.first().and_then(|s| s.title.as_ref()))
        .map(|t| quote(t))
        .unwrap_or_else(|| "(no title)".into())
}

/// The key's settings as compact JSON, without empty fields.
fn fields_json(f: &KeyFields) -> Value {
    let mut v = serde_json::to_value(f).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.retain(|_, x| !(x.is_null() || x.as_array().is_some_and(Vec::is_empty)));
    }
    v
}

fn key_line(k: &KeyDetail) -> String {
    let summary = if k.summary.is_empty() {
        describe_fields(&k.fields)
    } else {
        k.summary.clone()
    };
    format!(
        "[{},{}] {} - {}  {}",
        k.row,
        k.col,
        key_title(&k.fields),
        summary,
        fields_json(&k.fields)
    )
}

fn where_(p: &ProfileDetail, page: &str, at: [u32; 2]) -> String {
    format!("[{},{}] on page \"{page}\" of \"{}\"", at[0], at[1], p.name)
}

// ---- Read tools ----

fn list_profiles(b: &dyn Backend) -> ToolOutput {
    let all = b.list_profiles();
    if all.is_empty() {
        return ToolOutput::ok("There are no profiles.").with(json!({ "profiles": [] }));
    }
    let mut text = String::from("Profiles:\n");
    for p in &all {
        let mut notes = vec![format!(
            "{} page{}",
            p.pages,
            if p.pages == 1 { "" } else { "s" }
        )];
        if p.is_default {
            notes.insert(0, "default".into());
        }
        if p.has_error {
            notes.push("its file has a mistake; the last good version is in use".into());
        }
        text.push_str(&format!(
            "- {}: \"{}\" ({})\n",
            p.id,
            p.name,
            notes.join(", ")
        ));
    }
    ToolOutput::ok(text.trim_end()).with(json!({ "profiles": all }))
}

fn get_profile(b: &dyn Backend, args: &Args) -> Result<ToolOutput, String> {
    let p = load_profile(b, args.opt_str("profile")?)?;
    let mut text = format!("Profile \"{}\" (id {}).\n", p.name, p.id);
    if p.fixed_grid {
        text.push_str(&format!(
            "Grid: {} rows x {} columns on every tablet (rows 0-{}, columns 0-{}).\n",
            p.rows,
            p.cols,
            p.rows.saturating_sub(1),
            p.cols.saturating_sub(1)
        ));
    } else {
        text.push_str(&format!(
            "Grid: sized to each tablet's screen (about {} rows x {} columns; keys outside a \
             tablet's grid aren't shown on it).\n",
            p.rows, p.cols
        ));
    }
    if let Some(e) = &p.reload_error {
        text.push_str(&format!(
            "Its file has a mistake, so the last good version is in use: {e}\n"
        ));
    }
    if p.companion {
        text.push_str("It shows Bitfocus Companion's buttons; its pages aren't used.\n");
    }
    text.push_str("Positions are [row,col] from 0 at the top left.\n");
    for page in &p.pages {
        text.push_str(&format!(
            "\nPage \"{}\" ({} key{}):\n",
            page.name,
            page.keys.len(),
            if page.keys.len() == 1 { "" } else { "s" }
        ));
        let mut keys: Vec<&KeyDetail> = page.keys.iter().collect();
        keys.sort_by_key(|k| (k.row, k.col));
        for k in keys {
            text.push_str(&format!("  {}\n", key_line(k)));
        }
    }
    Ok(ToolOutput::ok(text.trim_end()).with(serde_json::to_value(&p).unwrap_or(Value::Null)))
}

fn list_tablets(b: &dyn Backend) -> ToolOutput {
    let all = b.list_tablets();
    if all.is_empty() {
        return ToolOutput::ok("No tablets are paired yet.").with(json!({ "tablets": [] }));
    }
    let names: std::collections::HashMap<String, String> = b
        .list_profiles()
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect();
    let mut text = String::from("Tablets:\n");
    for t in &all {
        let profile = names
            .get(&t.profile)
            .map(|n| format!("\"{n}\" ({})", t.profile))
            .unwrap_or_else(|| t.profile.clone());
        let page = t
            .page
            .as_ref()
            .map(|p| format!(", page \"{p}\""))
            .unwrap_or_default();
        text.push_str(&format!(
            "- \"{}\" (id {}): {}, {} {profile}{page}\n",
            t.name,
            t.id,
            if t.connected {
                "connected"
            } else {
                "not connected"
            },
            if t.connected { "showing" } else { "will show" },
        ));
    }
    ToolOutput::ok(text.trim_end()).with(json!({ "tablets": all }))
}

fn list_live_values(b: &dyn Backend) -> ToolOutput {
    let values = b.live_values();
    let mut text = String::from("Always available: {time}, {time_s}, {date}.\n");
    if values.is_empty() {
        text.push_str("No other live values are known right now. {cpu}, {mem}, {ha:...}, {obs:...} and plugin values appear once a key uses them and their source is set up.");
    } else {
        text.push_str("Known now:\n");
        for v in &values {
            text.push_str(&format!(
                "- {{{}}} = {}\n",
                v.name,
                v.value
                    .as_deref()
                    .map(quote)
                    .unwrap_or_else(|| "(not known yet)".into())
            ));
        }
    }
    ToolOutput::ok(text.trim_end()).with(json!({ "values": values }))
}

// ---- Press tools ----

async fn press_key(b: &dyn Backend, args: &Args) -> Result<ToolOutput, String> {
    let p = load_profile(b, args.opt_str("profile")?)?;
    editable(&p)?;
    let page = resolve_page(&p, args.opt_str("page")?.as_deref())?;
    let at = [args.index("row")?, args.index("col")?];
    let key = key_at(page, at).ok_or_else(|| no_key(&p, page, at))?;
    if key.fields.slider.is_some() {
        return Err(format!(
            "The cell at {} is a slider; sliders can't be pressed.",
            where_(&p, &page.name, at)
        ));
    }
    let tablet = match args.opt_str("tablet")? {
        Some(t) => Some(resolve_tablet(b, Some(t))?.id),
        None => None,
    };
    let press = Press {
        profile: p.id.clone(),
        page: page.name.clone(),
        row: at[0],
        col: at[1],
        tablet,
    };
    let note = b.press_key(&press).await?;
    let what = if key.summary.is_empty() {
        describe_fields(&key.fields)
    } else {
        key.summary.clone()
    };
    let mut text = format!(
        "Pressed {} at {}: {what}.",
        key_title(&key.fields),
        where_(&p, &page.name, at)
    );
    if let Some(n) = note {
        text.push_str(&format!(" It ran, but reported a problem: {n}"));
    }
    Ok(ToolOutput::ok(text))
}

fn no_key(p: &ProfileDetail, page: &PageDetail, at: [u32; 2]) -> String {
    let mut used: Vec<String> = page
        .keys
        .iter()
        .map(|k| format!("[{},{}]", k.row, k.col))
        .collect();
    used.sort();
    format!(
        "There's no key at {}. Keys on that page: {}.",
        where_(p, &page.name, at),
        if used.is_empty() {
            "none".into()
        } else {
            used.join(" ")
        }
    )
}

fn show_profile(b: &dyn Backend, args: &Args) -> Result<ToolOutput, String> {
    let profile = resolve_profile(b, Some(args.req_str("profile")?))?;
    let tablet = resolve_tablet(b, args.opt_str("tablet")?)?;
    if !tablet.connected {
        return Err(format!("Tablet \"{}\" isn't connected.", tablet.name));
    }
    b.show_profile(&tablet.id, &profile.id)?;
    Ok(ToolOutput::ok(format!(
        "Tablet \"{}\" now shows \"{}\".",
        tablet.name, profile.name
    )))
}

// ---- Edit tools ----

fn edit(b: &dyn Backend, p: &ProfileDetail, edits: &[Edit]) -> Result<(), String> {
    editable(p)?;
    b.edit_profile(&p.id, edits)
}

const UNDO: &str = "The person can undo this in the PerritoDeck app.";

fn set_key(b: &dyn Backend, args: &Args, perms: Permissions) -> Result<ToolOutput, String> {
    let p = load_profile(b, args.opt_str("profile")?)?;
    editable(&p)?;
    let page = resolve_page(&p, args.opt_str("page")?.as_deref())?;
    let at = [args.index("row")?, args.index("col")?];
    let existing = key_at(page, at);
    let replace = args.opt_bool("replace")?.unwrap_or(false);
    let old = existing.map(|k| k.fields.clone()).unwrap_or_default();
    let mut f = if replace {
        KeyFields {
            icon: old.icon.clone(),
            ..Default::default()
        }
    } else {
        old.clone()
    };

    for (name, slot) in [
        ("title", &mut f.title),
        ("bg", &mut f.bg),
        ("fg", &mut f.fg),
    ] {
        if args.has(name) {
            *slot = args.opt_str(name)?;
        }
    }
    let behaviour = ["action", "nav", "slider", "states"]
        .iter()
        .any(|k| args.0.get(*k).is_some_and(|v| !v.is_null()));
    if behaviour {
        f.action = None;
        f.nav = None;
        f.slider = None;
        f.states.clear();
        f.follow = None;
    }
    if args.has("action") {
        f.action = args.typed::<ActionSpec>("action")?;
    }
    if args.has("nav") {
        f.nav = args.typed::<NavSpec>("nav")?;
    }
    if args.has("slider") {
        f.slider = args.typed::<SliderSpec>("slider")?;
    }
    if args.has("states") {
        f.states = args.typed::<Vec<StateDef>>("states")?.unwrap_or_default();
    }
    if args.has("follow") {
        f.follow = args.opt_str("follow")?;
    } else if behaviour {
        // Keep following only if the key is still the same kind of toggle.
        if f.states.len() == 2 && old.states.len() == 2 {
            f.follow = old.follow.clone();
        }
    }
    if f == old && existing.is_some() {
        return Ok(ToolOutput::ok(format!(
            "Nothing to change: the key at {} already looks like that.",
            where_(&p, &page.name, at)
        )));
    }
    check_fields(&f)?;
    if !perms.run {
        let before = risky(&old);
        if let Some(new) = risky(&f).into_iter().find(|r| !before.contains(r)) {
            return Err(format!(
                "Giving a key an action that runs programs or plugins ({new}) also needs \
                 PerritoDeck's \"run actions\" permission, which is off. The person can turn \
                 it on in PerritoDeck's settings, under AI assistants, or make this key in the \
                 app themselves."
            ));
        }
    }
    edit(
        b,
        &p,
        &[Edit::SetKey {
            page: page.name.clone(),
            at,
            key: f,
        }],
    )?;
    let after = b
        .get_profile(&p.id)
        .ok()
        .and_then(|np| {
            np.pages
                .iter()
                .find(|pg| pg.name == page.name)
                .and_then(|pg| key_at(pg, at).map(key_line))
        })
        .unwrap_or_default();
    Ok(ToolOutput::ok(format!(
        "{} the key at {}. {UNDO}\n{after}",
        if existing.is_some() {
            "Changed"
        } else {
            "Created"
        },
        where_(&p, &page.name, at)
    )))
}

/// Early checks with clearer messages than the profile's own validation.
fn check_fields(f: &KeyFields) -> Result<(), String> {
    if let Some(t) = &f.title {
        if t.chars().count() > 200 {
            return Err("That title is too long; keep titles to a few words.".into());
        }
    }
    let colours = [f.bg.as_ref(), f.fg.as_ref()]
        .into_iter()
        .chain(f.states.iter().flat_map(|s| [s.bg.as_ref(), s.fg.as_ref()]))
        .flatten();
    for c in colours {
        if !is_color(c) {
            return Err(format!("\"{c}\" isn't a colour like \"#1E1E2E\"."));
        }
    }
    if f.states.len() == 1 {
        return Err("A toggle needs at least two states.".into());
    }
    if f.follow.is_some() && f.states.len() != 2 {
        return Err("\"follow\" needs a toggle with exactly two states (off, then on).".into());
    }
    for s in &f.states {
        if let Some(icon) = &s.icon {
            let p = Path::new(icon);
            if p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
                return Err("State icons must be image files inside the profile's folder.".into());
            }
        }
    }
    let actions = f
        .action
        .iter()
        .chain(f.states.iter().filter_map(|s| s.action.as_ref()));
    for a in actions {
        check_action(a)?;
    }
    Ok(())
}

fn check_action(a: &ActionSpec) -> Result<(), String> {
    match a {
        ActionSpec::Hotkey { keys } => keys
            .parse::<Chord>()
            .map(|_| ())
            .map_err(|e| format!("{e}. {HOTKEYS}")),
        ActionSpec::Multi { steps } => steps.iter().try_for_each(|s| check_action(&s.action)),
        ActionSpec::Random { actions } => {
            if actions.is_empty() {
                return Err("A random action needs at least one action to pick from.".into());
            }
            actions.iter().try_for_each(check_action)
        }
        _ => Ok(()),
    }
}

/// Actions in the key that run programs or plugins, described.
fn risky(f: &KeyFields) -> Vec<String> {
    let mut out = Vec::new();
    for a in f
        .action
        .iter()
        .chain(f.states.iter().filter_map(|s| s.action.as_ref()))
    {
        risky_action(a, &mut out);
    }
    if let Some(s @ (SliderSpec::Command { .. } | SliderSpec::Plugin { .. })) = &f.slider {
        // The whole spec, so changing the program counts as new.
        out.push(format!(
            "slider: {} {}",
            s.describe(),
            serde_json::to_string(s).unwrap_or_default()
        ));
    }
    out
}

fn risky_action(a: &ActionSpec, out: &mut Vec<String>) {
    match a {
        ActionSpec::Command { .. }
        | ActionSpec::Launch { .. }
        | ActionSpec::OpenPath { .. }
        | ActionSpec::Plugin { .. }
        | ActionSpec::SdPlugin { .. } => {
            // The whole spec, so changing a command's arguments counts as new.
            out.push(format!(
                "{} {}",
                describe_action(a),
                serde_json::to_string(a).unwrap_or_default()
            ));
        }
        ActionSpec::Multi { steps } => steps.iter().for_each(|s| risky_action(&s.action, out)),
        ActionSpec::Random { actions } => actions.iter().for_each(|a| risky_action(a, out)),
        _ => {}
    }
}

fn clear_key(b: &dyn Backend, args: &Args) -> Result<ToolOutput, String> {
    let p = load_profile(b, args.opt_str("profile")?)?;
    let page = resolve_page(&p, args.opt_str("page")?.as_deref())?;
    let at = [args.index("row")?, args.index("col")?];
    let key = key_at(page, at).ok_or_else(|| no_key(&p, page, at))?;
    let title = key_title(&key.fields);
    edit(
        b,
        &p,
        &[Edit::ClearKey {
            page: page.name.clone(),
            at,
        }],
    )?;
    Ok(ToolOutput::ok(format!(
        "Removed {title} from {}. {UNDO}",
        where_(&p, &page.name, at)
    )))
}

fn move_key(b: &dyn Backend, args: &Args) -> Result<ToolOutput, String> {
    let p = load_profile(b, args.opt_str("profile")?)?;
    let page = resolve_page(&p, args.opt_str("page")?.as_deref())?;
    let from = args.position("from")?;
    let to = args.position("to")?;
    let key = key_at(page, from).ok_or_else(|| no_key(&p, page, from))?;
    let to_page = match args.opt_str("to_page")? {
        Some(name) => resolve_page(&p, Some(&name))?,
        None => page,
    };
    let swapped = key_at(to_page, to).map(|k| key_title(&k.fields));
    let e = if to_page.name == page.name {
        Edit::MoveKey {
            page: page.name.clone(),
            from,
            to,
        }
    } else {
        Edit::MoveKeyToPage {
            from_page: page.name.clone(),
            at: from,
            to_page: to_page.name.clone(),
            to,
        }
    };
    edit(b, &p, &[e])?;
    let mut text = format!(
        "Moved {} from {} to {}.",
        key_title(&key.fields),
        where_(&p, &page.name, from),
        where_(&p, &to_page.name, to)
    );
    if let Some(other) = swapped {
        text.push_str(&format!(
            " {other} was there, so it moved to [{},{}] on page \"{}\".",
            from[0], from[1], page.name
        ));
    }
    text.push(' ');
    text.push_str(UNDO);
    Ok(ToolOutput::ok(text))
}

fn add_page(b: &dyn Backend, args: &Args) -> Result<ToolOutput, String> {
    let p = load_profile(b, args.opt_str("profile")?)?;
    let name = args.req_str("name")?.trim().to_owned();
    edit(b, &p, &[Edit::AddPage { name: name.clone() }])?;
    Ok(ToolOutput::ok(format!(
        "Added page \"{name}\" to \"{}\". To reach it, give a key on another page nav {{\"page\": \"{name}\"}}. {UNDO}",
        p.name
    )))
}

fn rename_page(b: &dyn Backend, args: &Args) -> Result<ToolOutput, String> {
    let p = load_profile(b, args.opt_str("profile")?)?;
    let from = resolve_page(&p, Some(&args.req_str("from")?))?.name.clone();
    let to = args.req_str("to")?.trim().to_owned();
    edit(
        b,
        &p,
        &[Edit::RenamePage {
            from: from.clone(),
            to: to.clone(),
        }],
    )?;
    Ok(ToolOutput::ok(format!(
        "Renamed page \"{from}\" to \"{to}\" in \"{}\". {UNDO}",
        p.name
    )))
}

fn remove_page(b: &dyn Backend, args: &Args) -> Result<ToolOutput, String> {
    let p = load_profile(b, args.opt_str("profile")?)?;
    let page = resolve_page(&p, Some(&args.req_str("name")?))?;
    if p.pages.len() == 1 {
        return Err(format!(
            "\"{}\" is the only page of \"{}\"; a profile needs at least one.",
            page.name, p.name
        ));
    }
    let n = page.keys.len();
    edit(
        b,
        &p,
        &[Edit::RemovePage {
            name: page.name.clone(),
        }],
    )?;
    Ok(ToolOutput::ok(format!(
        "Removed page \"{}\" ({n} key{}) from \"{}\". {UNDO}",
        page.name,
        if n == 1 { "" } else { "s" },
        p.name
    )))
}

fn create_profile(b: &dyn Backend, args: &Args) -> Result<ToolOutput, String> {
    let name = args.req_str("name")?.trim().to_owned();
    let id = b.create_profile(&name)?;
    let pages = b
        .get_profile(&id)
        .map(|p| {
            p.pages
                .iter()
                .map(|pg| format!("\"{}\"", pg.name))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    Ok(ToolOutput::ok(format!(
        "Created profile \"{name}\" with id {id}{}. Add keys with set_key (profile \"{id}\"); show it on a tablet with show_profile.",
        if pages.is_empty() { String::new() } else { format!(" (pages: {pages})") }
    ))
    .with(json!({ "id": id })))
}

// ---- Run ----

async fn run_action(b: &dyn Backend, args: &Args) -> Result<ToolOutput, String> {
    let action = args
        .typed::<ActionSpec>("action")?
        .ok_or("\"action\" is required.")?;
    check_action(&action)?;
    let note = b.run_action(&action).await?;
    Ok(ToolOutput::ok(match note {
        None => format!("Done: {}.", describe_action(&action)),
        Some(n) => format!(
            "Ran {}, but it reported a problem: {n}",
            describe_action(&action)
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant the schema offers parses as an ActionSpec, and every ActionSpec
    /// variant is offered.
    #[test]
    fn action_schema_matches_action_spec() {
        let schema = action_schema();
        let variants = schema["oneOf"].as_array().unwrap();
        let samples = [
            json!({"type":"hotkey","keys":"Ctrl+C"}),
            json!({"type":"text","text":"hi"}),
            json!({"type":"media","key":"playPause"}),
            json!({"type":"openUrl","url":"https://perritodeck.dev"}),
            json!({"type":"openPath","path":"/tmp"}),
            json!({"type":"launch","program":"app","args":["a"]}),
            json!({"type":"command","program":"git","args":["status"],"cwd":"/","timeoutMs":5}),
            json!({"type":"multi","steps":[{"action":{"type":"none"},"delayMs":5}]}),
            json!({"type":"random","actions":[{"type":"media","key":"next"},{"type":"none"}]}),
            json!({"type":"obs","op":"scene","scene":"Main"}),
            json!({"type":"obs","op":"toggleMute","input":"Mic"}),
            json!({"type":"homeAssistant","service":"light.toggle","entity":"light.desk","data":{"brightness_pct":40}}),
            json!({"type":"plugin","plugin":"p","action":"a","settings":{"x":1}}),
            json!({"type":"sdPlugin","plugin":"com.x","action":"com.x.y","settings":{}}),
            json!({"type":"none"}),
        ];
        let mut seen = std::collections::HashSet::new();
        for s in &samples {
            serde_json::from_value::<ActionSpec>(s.clone()).unwrap_or_else(|e| panic!("{s}: {e}"));
            seen.insert(s["type"].as_str().unwrap().to_owned());
            let v = variants
                .iter()
                .find(|v| v["properties"]["type"]["const"] == s["type"])
                .unwrap_or_else(|| panic!("no schema for {}", s["type"]));
            for k in s.as_object().unwrap().keys() {
                assert!(
                    v["properties"].get(k).is_some(),
                    "{k} missing from {}",
                    s["type"]
                );
            }
        }
        assert_eq!(seen.len(), variants.len());
        // The OBS ops in the schema all parse.
        for op in variants
            .iter()
            .find(|v| v["properties"]["type"]["const"] == "obs")
            .unwrap()["properties"]["op"]["enum"]
            .as_array()
            .unwrap()
        {
            let mut a = json!({"type":"obs","op":op});
            match op.as_str().unwrap() {
                "scene" => a["scene"] = json!("S"),
                "toggleMute" => a["input"] = json!("Mic"),
                _ => {}
            }
            serde_json::from_value::<ActionSpec>(a).unwrap();
        }
        for key in [
            "playPause",
            "next",
            "previous",
            "stop",
            "volumeUp",
            "volumeDown",
            "mute",
        ] {
            serde_json::from_value::<ActionSpec>(json!({"type":"media","key":key})).unwrap();
        }
    }

    /// Every shape the nav schema offers parses as the NavSpec it means, and every
    /// NavSpec variant is offered.
    #[test]
    fn nav_schema_matches_nav_spec() {
        let schema = with_defs(json!({}));
        let shapes = schema["$defs"]["nav"]["anyOf"].as_array().unwrap();
        assert_eq!(shapes.len(), 3);
        let words: Vec<&str> = shapes[0]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_str().unwrap())
            .collect();
        for w in &words {
            assert!(matches!(
                serde_json::from_value::<NavSpec>(json!(w)).unwrap(),
                NavSpec::Simple(_)
            ));
        }
        assert_eq!(words.len(), 3);
        let page = json!({"page":"Media"});
        let profile = json!({"profile":"streaming"});
        let profile_page = json!({"profile":"streaming","page":"Scenes"});
        assert_eq!(
            serde_json::from_value::<NavSpec>(page.clone()).unwrap(),
            NavSpec::Page {
                page: "Media".into()
            }
        );
        assert_eq!(
            serde_json::from_value::<NavSpec>(profile.clone()).unwrap(),
            NavSpec::Profile {
                profile: "streaming".into(),
                page: None
            }
        );
        assert_eq!(
            serde_json::from_value::<NavSpec>(profile_page.clone()).unwrap(),
            NavSpec::Profile {
                profile: "streaming".into(),
                page: Some("Scenes".into())
            }
        );
        // Each object shape lists the fields of its variant and requires the right one.
        let props = |i: usize| -> Vec<String> {
            let mut v: Vec<String> = shapes[i]["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            v.sort();
            v
        };
        assert_eq!(props(1), ["page"]);
        assert_eq!(shapes[1]["required"], json!(["page"]));
        assert_eq!(props(2), ["page", "profile"]);
        assert_eq!(shapes[2]["required"], json!(["profile"]));
        // Serialized forms match the schema's shapes.
        for v in [page, profile, profile_page] {
            let back = serde_json::to_value(serde_json::from_value::<NavSpec>(v.clone()).unwrap());
            assert_eq!(back.unwrap(), v);
        }
    }

    #[test]
    fn slider_schema_matches_slider_spec() {
        for s in [
            json!({"type":"volume"}),
            json!({"type":"homeAssistant","entity":"light.desk"}),
            json!({"type":"obs","input":"Mic"}),
            json!({"type":"command","program":"x","args":["{value}"]}),
            json!({"type":"plugin","plugin":"p","action":"a","settings":{},"follow":"v"}),
        ] {
            serde_json::from_value::<SliderSpec>(s.clone()).unwrap_or_else(|e| panic!("{s}: {e}"));
        }
        assert_eq!(slider_schema()["oneOf"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn tool_names_follow_the_spec() {
        for d in defs() {
            assert!(d.name.len() <= 128);
            assert!(d
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)));
            assert_eq!(d.input["type"], "object");
        }
    }

    #[test]
    fn risky_actions_are_found_anywhere() {
        let cmd = ActionSpec::Command {
            program: "rm".into(),
            args: vec![],
            cwd: None,
            timeout_ms: None,
        };
        let f = KeyFields {
            action: Some(ActionSpec::Multi {
                steps: vec![perritodeck_actions::Step {
                    action: cmd.clone(),
                    delay_ms: 0,
                }],
            }),
            ..Default::default()
        };
        assert_eq!(risky(&f).len(), 1);
        let random = KeyFields {
            action: Some(ActionSpec::Random {
                actions: vec![
                    ActionSpec::None,
                    ActionSpec::Multi {
                        steps: vec![perritodeck_actions::Step {
                            action: cmd.clone(),
                            delay_ms: 0,
                        }],
                    },
                ],
            }),
            ..Default::default()
        };
        assert_eq!(risky(&random).len(), 1);
        let toggle = KeyFields {
            states: vec![
                StateDef {
                    action: Some(cmd),
                    ..Default::default()
                },
                StateDef::default(),
            ],
            ..Default::default()
        };
        assert_eq!(risky(&toggle).len(), 1);
        let hotkey = KeyFields {
            action: Some(ActionSpec::Hotkey {
                keys: "Ctrl+C".into(),
            }),
            slider: None,
            ..Default::default()
        };
        assert!(risky(&hotkey).is_empty());
        let slider = KeyFields {
            slider: Some(SliderSpec::Command {
                program: "x".into(),
                args: vec![],
            }),
            ..Default::default()
        };
        assert_eq!(risky(&slider).len(), 1);
    }
}
