use std::{sync::Mutex, time::Duration};

use perritodeck_actions::{Chord, InputBackend, MediaKey, SystemBackend};
use perritodeck_protocol::Platform;

use super::*;

#[derive(Default)]
struct Sent(Mutex<Vec<(String, Message)>>);

impl Outbox for Sent {
    fn send(&self, device_id: &str, msg: Message) -> bool {
        self.0.lock().unwrap().push((device_id.to_owned(), msg));
        true
    }
}

impl Sent {
    fn take(&self) -> Vec<Message> {
        std::mem::take(&mut *self.0.lock().unwrap())
            .into_iter()
            .map(|(_, m)| m)
            .collect()
    }
}

#[derive(Default)]
struct Keys(Mutex<Vec<String>>);

impl InputBackend for Keys {
    fn chord(&self, chord: &Chord) -> Result<(), ActionError> {
        self.0.lock().unwrap().push(chord.to_string());
        Ok(())
    }
    fn text(&self, text: &str) -> Result<(), ActionError> {
        self.0.lock().unwrap().push(format!("text {text}"));
        Ok(())
    }
    fn media(&self, key: MediaKey) -> Result<(), ActionError> {
        self.0.lock().unwrap().push(format!("media {key:?}"));
        Ok(())
    }
}

impl SystemBackend for Keys {
    fn open(&self, target: &str) -> Result<(), ActionError> {
        self.0.lock().unwrap().push(format!("open {target}"));
        Ok(())
    }
}

const PROFILE: &str = r##"
schema_version = 1
name = "test"
[grid]
cols = 3
rows = 2

[[pages]]
name = "Main"
[[pages.keys]]
at = [0, 0]
title = "Copy"
action = { type = "hotkey", keys = "Ctrl+C" }
[[pages.keys]]
at = [0, 1]
states = [
  { title = "On", bg = "#00FF00" },
  { title = "Off", bg = "#FF0000" },
]
action = { type = "text", text = "toggle" }
[[pages.keys]]
at = [0, 2]
title = "More"
nav = { page = "More" }
[[pages.keys]]
at = [1, 0]
title = "{date}"

[[pages]]
name = "More"
[[pages.keys]]
at = [0, 0]
title = "Back"
nav = "back"
"##;

fn setup() -> (Arc<Deck>, Arc<Sent>, Arc<Keys>) {
    let sent = Arc::new(Sent::default());
    let keys = Arc::new(Keys::default());
    let deck = Deck::new(
        Profile::parse(PROFILE).unwrap(),
        ".",
        sent.clone(),
        Executor::new(keys.clone(), keys.clone()),
    );
    (deck, sent, keys)
}

fn hello() -> Hello {
    Hello {
        device_id: "tab".into(),
        device_name: "Tab".into(),
        app_version: "0".into(),
        platform: Platform::Android,
        os_version: "14".into(),
        screen: Screen {
            width_px: 1920,
            height_px: 1200,
            density_dpi: 224,
        },
        capabilities: vec!["keys".into()],
        requested_grid: None,
        token: None,
        device_secret: None,
    }
}

const GRID: Grid = Grid { cols: 3, rows: 2 };

fn connect(deck: &Deck, images: bool) {
    let caps: Vec<String> = if images {
        vec!["keys".into(), "images".into()]
    } else {
        vec!["keys".into()]
    };
    deck.connected("tab", GRID, hello().screen, &caps);
}

fn titles(layout: &Layout) -> Vec<&str> {
    layout.keys.iter().map(|k| k.title.as_str()).collect()
}

#[tokio::test]
async fn initial_layout_follows_profile_grid() {
    let (deck, _, _) = setup();
    let l = deck.initial_layout(&hello());
    assert_eq!((l.cols, l.rows), (3, 2));
    let t = titles(&l);
    assert_eq!(&t[..3], &["Copy", "On", "More"]);
    assert!(!t[3].contains('{'), "templates are rendered: {}", t[3]);
    assert_eq!(l.keys[4].bg_color, EMPTY_BG, "empty positions are blank");
}

#[tokio::test]
async fn pages_and_back() {
    let (deck, sent, _) = setup();
    connect(&deck, false);
    sent.take();
    deck.key_down("tab", 2); // More
    match sent.take().as_slice() {
        [Message::Layout(l)] => assert_eq!(l.keys[0].title, "Back"),
        other => panic!("expected the More page, got {other:?}"),
    }
    deck.key_down("tab", 0); // Back
    match sent.take().as_slice() {
        [Message::Layout(l)] => assert_eq!(l.keys[0].title, "Copy"),
        other => panic!("expected Main again, got {other:?}"),
    }
}

#[tokio::test]
async fn actions_run_and_toggles_switch() {
    let (deck, sent, keys) = setup();
    connect(&deck, false);
    sent.take();

    deck.key_down("tab", 0);
    deck.key_down("tab", 1);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        *keys.0.lock().unwrap(),
        vec!["Ctrl+C".to_owned(), "text toggle".to_owned()]
    );

    // The toggle changed colour, so a text-only client gets the page again with "Off".
    let msgs = sent.take();
    let layout = msgs.iter().find_map(|m| match m {
        Message::Layout(l) => Some(l),
        _ => None,
    });
    assert_eq!(layout.expect("page resent").keys[1].title, "Off");

    deck.key_down("tab", 1);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let back_on = sent
        .take()
        .into_iter()
        .any(|m| matches!(m, Message::Layout(l) if l.keys[1].title == "On"));
    assert!(back_on, "second press returns to the first state");
}

#[tokio::test]
async fn image_clients_get_rendered_keys() {
    let (deck, sent, _) = setup();
    connect(&deck, true);
    let images: Vec<u32> = sent
        .take()
        .into_iter()
        .filter_map(|m| match m {
            Message::SetImage(SetImage {
                key_index,
                image: Some(url),
            }) if url.starts_with("data:image/png") => Some(key_index),
            _ => None,
        })
        .collect();
    assert_eq!(
        images,
        vec![0, 1, 2, 3],
        "every defined key, none for empty positions"
    );

    deck.key_down("tab", 1); // toggle → new image + title for just that key
    tokio::time::sleep(Duration::from_millis(50)).await;
    let msgs = sent.take();
    assert!(msgs
        .iter()
        .any(|m| matches!(m, Message::SetTitle(t) if t.key_index == 1 && t.title == "Off")));
    assert!(msgs
        .iter()
        .any(|m| matches!(m, Message::SetImage(i) if i.key_index == 1)));
    assert!(
        !msgs.iter().any(|m| matches!(m, Message::Layout(_))),
        "image clients don't need the whole page"
    );
}

#[tokio::test]
async fn command_results_flash_on_the_key() {
    let profile = r##"
schema_version = 1
name = "cmd"
[grid]
cols = 2
rows = 1
[[pages]]
name = "P"
[[pages.keys]]
at = [0, 0]
action = { type = "command", program = "definitely-not-a-real-program-xyz" }
[[pages.keys]]
at = [0, 1]
action = { type = "multi", steps = [ { action = { type = "text", text = "a" } } ] }
"##;
    let sent = Arc::new(Sent::default());
    let keys = Arc::new(Keys::default());
    let deck = Deck::new(
        Profile::parse(profile).unwrap(),
        ".",
        sent.clone(),
        Executor::new(keys.clone(), keys),
    );
    deck.connected("tab", Grid { cols: 2, rows: 1 }, hello().screen, &[]);
    sent.take();
    deck.key_down("tab", 0);
    deck.key_down("tab", 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let msgs = sent.take();
    assert!(msgs
        .iter()
        .any(|m| matches!(m, Message::ShowAlert(f) if f.key_index == 0 && f.message.is_some())));
    assert!(msgs
        .iter()
        .any(|m| matches!(m, Message::ShowOk(f) if f.key_index == 1)));
}

#[tokio::test]
async fn reload_redraws_and_resets_missing_pages() {
    let (deck, sent, _) = setup();
    connect(&deck, false);
    deck.key_down("tab", 2); // go to "More" (page 1)
    sent.take();
    let one_page = PROFILE
        .split("[[pages]]\nname = \"More\"")
        .next()
        .unwrap()
        .replace("Copy", "Copied");
    let mut p = Profile::parse(&one_page).unwrap();
    p.pages[0].keys.retain(|k| k.nav.is_none());
    deck.reload(p);
    match sent.take().as_slice() {
        [Message::Layout(l)] => {
            assert_eq!(l.keys[0].title, "Copied", "device falls back to page 1")
        }
        other => panic!("expected a redraw, got {other:?}"),
    }
}

#[tokio::test]
async fn tick_sends_nothing_when_nothing_changed() {
    let (deck, sent, _) = setup();
    connect(&deck, false);
    sent.take();
    deck.tick(); // {date} hasn't changed since connect
    assert!(sent.take().is_empty());
}

#[tokio::test]
async fn presses_on_empty_or_unknown_keys_are_ignored() {
    let (deck, sent, keys) = setup();
    connect(&deck, false);
    sent.take();
    deck.key_down("tab", 5);
    deck.key_down("nobody", 0);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(sent.take().is_empty());
    assert!(keys.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn devices_can_show_different_profiles() {
    let (deck, sent, _) = setup();
    connect(&deck, false);
    let other = Profile::parse(&PROFILE.replace("Copy", "Other copy")).unwrap();
    deck.load_profile("streaming", other, None);
    assert_eq!(deck.profile_ids(), ["default", "streaming"]);
    sent.take();

    // Assigning switches the tablet now and on later connects.
    assert!(deck.assign("tab", Some("streaming")));
    match sent.take().as_slice() {
        [Message::Layout(l)] => assert!(l.keys.iter().any(|k| k.title == "Other copy")),
        other => panic!("expected a redraw, got {other:?}"),
    }
    assert_eq!(deck.device_profile("tab"), "streaming");
    assert_eq!(
        deck.initial_layout(&hello())
            .keys
            .iter()
            .filter(|k| k.title == "Other copy")
            .count(),
        1
    );
    assert!(!deck.assign("tab", Some("nope")));

    // Editing the other profile redraws only devices showing it.
    deck.reload(Profile::parse(&PROFILE.replace("Copy", "Default copy")).unwrap());
    assert!(sent.take().is_empty());

    // Removing it sends the tablet back to the default.
    assert!(deck.remove_profile("streaming"));
    assert!(!deck.remove_profile("default"));
    match sent.take().as_slice() {
        [Message::Layout(l)] => assert!(l.keys.iter().any(|k| k.title == "Default copy")),
        other => panic!("expected a redraw, got {other:?}"),
    }
    assert_eq!(deck.device_profile("tab"), "default");
}

#[tokio::test]
async fn paused_keys_do_nothing_but_say_so() {
    let (deck, sent, keys) = setup();
    connect(&deck, false);
    sent.take();
    deck.set_paused(true);
    deck.key_down("tab", 0);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(keys.0.lock().unwrap().is_empty());
    assert!(matches!(sent.take().as_slice(), [Message::ShowAlert(f)] if f.key_index == 0));
    deck.set_paused(false);
    deck.key_down("tab", 0);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!keys.0.lock().unwrap().is_empty());
}

#[test]
fn profile_ids_come_from_file_names() {
    use crate::watch::profile_id;
    use std::path::Path;
    assert_eq!(
        profile_id(Path::new("/x/streaming.toml")).as_deref(),
        Some("streaming")
    );
    assert_eq!(profile_id(Path::new("/x/.default.toml.saving")), None);
    assert_eq!(profile_id(Path::new("/x/.hidden.toml")), None);
    assert_eq!(profile_id(Path::new("/x/notes.txt")), None);
}

#[derive(Default)]
struct Volume(std::sync::Mutex<Vec<u8>>);

impl perritodeck_actions::SystemBackend for Volume {
    fn open(&self, _: &str) -> Result<(), perritodeck_actions::ActionError> {
        Ok(())
    }
    fn set_volume(&self, p: u8) -> Result<(), perritodeck_actions::ActionError> {
        std::thread::sleep(Duration::from_millis(30));
        self.0.lock().unwrap().push(p);
        Ok(())
    }
    fn volume(&self) -> Option<u8> {
        Some(33)
    }
}

const SLIDERS: &str = r##"
schema_version = 1
name = "S"
[grid]
cols = 2
rows = 1
[[pages]]
name = "Main"
[[pages.keys]]
at = [0, 0]
title = "Copy"
action = { type = "hotkey", keys = "Ctrl+C" }
[[pages.keys]]
at = [0, 1]
title = "Volume"
slider = { type = "volume" }
"##;

#[tokio::test]
async fn sliders_set_the_volume_and_follow_it() {
    let sent = Arc::new(Sent::default());
    let keys = Arc::new(Keys::default());
    let vol = Arc::new(Volume::default());
    let deck = Deck::new(
        Profile::parse(SLIDERS).unwrap(),
        ".",
        sent.clone(),
        Executor::new(keys, vol.clone()),
    );
    let grid = Grid { cols: 2, rows: 1 };
    let caps = ["keys".to_string(), "sliders".to_string()];
    deck.connected("a", grid, hello().screen, &caps);
    deck.connected("b", grid, hello().screen, &caps);
    deck.connected("old", grid, hello().screen, &["keys".to_string()]);

    // Slider-capable tablets get a slider; others an ordinary key.
    let mut h = hello();
    h.capabilities = caps.to_vec();
    let l = deck.initial_layout(&h);
    assert_eq!(
        l.keys[1].control,
        Some(perritodeck_protocol::Control::Slider)
    );
    assert_eq!(l.keys[1].value, Some(50));
    h.capabilities = vec!["keys".into()];
    assert_eq!(deck.initial_layout(&h).keys[1].control, None);
    sent.take();

    // A fast drag: values coalesce; the last one always lands.
    for v in [10u8, 20, 30, 40, 64] {
        deck.slider_change("a", 1, v);
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let set = vol.0.lock().unwrap().clone();
    assert_eq!(set.last(), Some(&64));
    assert!(set.len() < 5, "coalesced: {set:?}");
    // The other slider tablet follows; the one without sliders isn't told.
    let msgs = sent.take();
    assert!(msgs
        .iter()
        .any(|m| matches!(m, Message::SetValue(v) if v.value == 64)));
    assert!(!msgs
        .iter()
        .any(|m| matches!(m, Message::SetValue(v) if v.value == 200)));

    // Changes on the computer reach the tablets.
    deck.poll_sliders().await;
    let msgs = sent.take();
    assert_eq!(
        msgs.iter()
            .filter(|m| matches!(m, Message::SetValue(v) if v.value == 33))
            .count(),
        2
    );
    deck.poll_sliders().await;
    assert!(sent.take().is_empty(), "nothing new");
}

#[tokio::test]
async fn live_values_and_followed_toggles() {
    let profile = r##"
schema_version = 1
name = "L"
[grid]
cols = 2
rows = 1
[[pages]]
name = "Main"
[[pages.keys]]
at = [0, 0]
title = "CPU\n{cpu}"
[[pages.keys]]
at = [0, 1]
follow = "obs:recording"
states = [
  { title = "Rec", action = { type = "text", text = "r" } },
  { title = "Stop", bg = "#F38BA8", action = { type = "text", text = "s" } },
]
"##;
    let sent = Arc::new(Sent::default());
    let keys = Arc::new(Keys::default());
    let deck = Deck::new(
        Profile::parse(profile).unwrap(),
        ".",
        sent.clone(),
        Executor::new(keys.clone(), keys.clone()),
    );
    assert_eq!(deck.live_tokens(), ["cpu", "obs:recording"]);
    deck.connected("tab", Grid { cols: 2, rows: 1 }, hello().screen, &[]);
    sent.take();

    deck.set_live([
        ("cpu".to_string(), "7%".to_string()),
        ("obs:recording".into(), "on".into()),
    ]);
    deck.tick();
    let msgs = sent.take();
    assert!(
        msgs.iter()
            .any(|m| matches!(m, Message::SetTitle(t) if t.title == "CPU\n7%")),
        "{msgs:?}"
    );
    // The toggle shows its second state because OBS is recording; text-only clients
    // get a fresh page when colours change.
    assert!(
        msgs.iter()
            .any(|m| matches!(m, Message::Layout(l) if l.keys[1].title == "Stop")),
        "{msgs:?}"
    );

    // Pressing runs the current state's action; the state then comes from OBS again.
    deck.key_down("tab", 1);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        keys.0.lock().unwrap().last().map(String::as_str),
        Some("text s")
    );
    deck.set_live([("obs:recording".to_string(), "off".to_string())]);
    deck.tick();
    assert!(sent
        .take()
        .iter()
        .any(|m| matches!(m, Message::Layout(l) if l.keys[1].title == "Rec")));
}

#[test]
fn gif_icons_animate() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("spin.gif"), perritodeck_render::test_gif()).unwrap();
    let profile = r##"
schema_version = 1
name = "anim"
[grid]
cols = 3
rows = 2
[[pages]]
name = "Main"
[[pages.keys]]
at = [0, 0]
icon = "spin.gif"
[[pages.keys]]
at = [0, 1]
title = "Still"
"##;
    let sent = Arc::new(Sent::default());
    let keys = Arc::new(Keys::default());
    let deck = Deck::new(
        Profile::parse(profile).unwrap(),
        dir.path(),
        sent.clone(),
        Executor::new(keys.clone(), keys),
    );
    deck.set_clock_ms(0);
    connect(&deck, true);
    let images = |msgs: Vec<Message>| -> Vec<(u32, String)> {
        msgs.into_iter()
            .filter_map(|m| match m {
                Message::SetImage(s) => Some((s.key_index, s.image?)),
                _ => None,
            })
            .collect()
    };
    let first = images(sent.take());
    assert_eq!(first.len(), 2, "both keys drawn once on connect");
    let frame0 = first.iter().find(|(i, _)| *i == 0).unwrap().1.clone();

    deck.animate();
    assert!(sent.take().is_empty(), "same frame: nothing sent");

    deck.set_clock_ms(u64::from(perritodeck_render::MIN_FRAME_MS) + 5);
    deck.animate();
    let next = images(sent.take());
    assert_eq!(next.len(), 1, "only the animated key changes");
    assert_eq!(next[0].0, 0);
    assert_ne!(next[0].1, frame0);

    deck.set_clock_ms(perritodeck_render::MIN_FRAME_MS as u64 + 500 + 5);
    deck.animate();
    assert_eq!(
        images(sent.take()),
        vec![(0, frame0)],
        "it loops, from cached frames"
    );
}

#[derive(Default)]
struct Plugins(Mutex<Vec<(String, String, serde_json::Value)>>);

impl perritodeck_actions::PluginRunner for Plugins {
    fn run_plugin<'a>(
        &'a self,
        plugin: &'a str,
        action: &'a str,
        settings: &'a serde_json::Value,
    ) -> perritodeck_actions::BoxFuture<'a, Result<Outcome, ActionError>> {
        self.0
            .lock()
            .unwrap()
            .push((plugin.into(), action.into(), settings.clone()));
        Box::pin(async { Ok(Outcome::Ok) })
    }
}

#[tokio::test]
async fn plugin_sliders_send_values_and_follow_the_plugin() {
    let profile = r##"
schema_version = 1
name = "p"
[grid]
cols = 1
rows = 1
[[pages]]
name = "Main"
[[pages.keys]]
at = [0, 0]
title = "Lamp"
slider = { type = "plugin", plugin = "dev.x", action = "dim", settings = { lamp = "desk" }, follow = "level" }
"##;
    let sent = Arc::new(Sent::default());
    let keys = Arc::new(Keys::default());
    let plugins = Arc::new(Plugins::default());
    let deck = Deck::new(
        Profile::parse(profile).unwrap(),
        ".",
        sent.clone(),
        Executor::new(keys.clone(), keys).with_plugins(plugins.clone()),
    );
    let grid = Grid { cols: 1, rows: 1 };
    deck.connected(
        "a",
        grid,
        hello().screen,
        &["keys".into(), "sliders".into()],
    );
    sent.take();

    deck.slider_change("a", 0, 42);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let runs = plugins.0.lock().unwrap().clone();
    assert_eq!(
        runs,
        vec![(
            "dev.x".to_string(),
            "dim".to_string(),
            serde_json::json!({ "lamp": "desk", "value": 42 })
        )]
    );

    deck.set_live([("plugin:dev.x:level".to_string(), "87.4".to_string())]);
    deck.poll_sliders().await;
    assert!(sent
        .take()
        .iter()
        .any(|m| matches!(m, Message::SetValue(v) if v.value == 87)));
}

#[derive(Default)]
struct FakeCompanion(Mutex<Vec<String>>);

impl RemoteSurfaces for FakeCompanion {
    fn attach(&self, device_id: &str, grid: Grid, image_px: Option<u32>) {
        self.0.lock().unwrap().push(format!(
            "attach {device_id} {}x{} {}",
            grid.cols,
            grid.rows,
            image_px.is_some()
        ));
    }
    fn detach(&self, device_id: &str) {
        self.0.lock().unwrap().push(format!("detach {device_id}"));
    }
    fn press(&self, device_id: &str, key_index: u32, down: bool) {
        self.0
            .lock()
            .unwrap()
            .push(format!("press {device_id} {key_index} {down}"));
    }
}

#[test]
fn companion_profiles_show_companions_buttons() {
    let (deck, sent, keys) = setup();
    deck.load_profile(
        "companion",
        Profile::parse("schema_version = 1\nname = \"Companion\"\ncompanion = true\n").unwrap(),
        None,
    );
    deck.assign("tab", Some("companion"));
    let mut h = hello();
    h.capabilities = vec!["keys".into(), "images".into()];
    let l = deck.initial_layout(&h);
    assert_eq!(l.keys[0].title, "Set up\nCompanion", "says what's missing");
    connect(&deck, true);
    deck.key_down("tab", 0);
    assert!(keys.0.lock().unwrap().is_empty(), "nothing runs");
    sent.take();

    let fake = Arc::new(FakeCompanion::default());
    deck.set_remote(fake.clone());
    assert_eq!(fake.0.lock().unwrap().as_slice(), ["attach tab 3x2 true"]);
    let Some(Message::Layout(l)) = sent.take().into_iter().next() else {
        panic!("a layout first")
    };
    assert_eq!(l.keys[0].title, "Connecting to\nCompanion");

    deck.remote_status(None);
    deck.remote_key(
        "tab",
        4,
        RemoteKey {
            png: None,
            title: "Cam 2".into(),
            bg: Some("#ff0000".into()),
            fg: Some("#ffffff".into()),
        },
    );
    let msgs = sent.take();
    let Some(Message::Layout(l)) = msgs.first() else {
        panic!("the first key redraws the page: {msgs:?}")
    };
    assert_eq!(
        (l.keys[4].title.as_str(), l.keys[4].bg_color.as_str()),
        ("Cam 2", "#ff0000")
    );
    assert_eq!(l.keys[0].title, "", "the note is gone");
    assert!(msgs
        .iter()
        .any(|m| matches!(m, Message::SetImage(i) if i.key_index == 4)));

    // A second change only sends that key.
    deck.remote_key(
        "tab",
        4,
        RemoteKey {
            title: "Cam 3".into(),
            bg: Some("#ff0000".into()),
            ..Default::default()
        },
    );
    let msgs = sent.take();
    assert!(
        matches!(&msgs[..], [Message::SetTitle(t), Message::SetImage(_)] if t.title == "Cam 3"),
        "{msgs:?}"
    );

    deck.key_down("tab", 4);
    deck.key_up("tab", 4);
    deck.set_paused(true);
    deck.key_down("tab", 4);
    assert!(sent
        .take()
        .iter()
        .any(|m| matches!(m, Message::ShowAlert(_))));
    deck.set_paused(false);
    deck.tick();
    deck.animate();
    assert!(
        sent.take().is_empty(),
        "the deck doesn't draw over Companion"
    );

    // Companion goes away: the keys say so.
    deck.remote_status(Some("Companion\noffline"));
    let msgs = sent.take();
    assert!(matches!(&msgs[0], Message::Layout(l) if l.keys[0].title == "Companion\noffline"));

    deck.assign("tab", None);
    deck.connected("other", GRID, hello().screen, &["keys".to_string()]);
    deck.disconnected("tab");
    assert_eq!(
        fake.0.lock().unwrap().as_slice(),
        [
            "attach tab 3x2 true",
            "press tab 4 true",
            "press tab 4 false",
            "detach tab"
        ]
    );
}

#[derive(Default)]
struct FakeAgents(Mutex<Vec<String>>);

impl KeyAgents for FakeAgents {
    fn device(&self, device_id: &str, grid: Option<Grid>) {
        self.0
            .lock()
            .unwrap()
            .push(format!("device {device_id} {}", grid.is_some()));
    }
    fn appear(&self, key: &AgentKey) {
        self.0.lock().unwrap().push(format!(
            "appear {} {} {}",
            key.context, key.action, key.settings
        ));
    }
    fn disappear(&self, key: &AgentKey) {
        self.0
            .lock()
            .unwrap()
            .push(format!("disappear {}", key.context));
    }
    fn press(&self, key: &AgentKey, down: bool) {
        self.0
            .lock()
            .unwrap()
            .push(format!("press {} {down}", key.context));
    }
}

const SD_PROFILE: &str = r##"
schema_version = 1
name = "sd"
[grid]
cols = 3
rows = 2
[[pages]]
name = "Main"
[[pages.keys]]
at = [0, 1]
title = "Count"
action = { type = "sdPlugin", plugin = "com.example.counter", action = "com.example.counter.up", settings = { step = 2 } }
[[pages.keys]]
at = [1, 2]
title = "More"
nav = { page = "Two" }
[[pages]]
name = "Two"
"##;

#[test]
fn stream_deck_plugin_keys_are_run_by_agents() {
    let sent = Arc::new(Sent::default());
    let keys = Arc::new(Keys::default());
    let deck = Deck::new(
        Profile::parse(SD_PROFILE).unwrap(),
        ".",
        sent.clone(),
        Executor::new(keys.clone(), keys.clone()),
    );
    let agents = Arc::new(FakeAgents::default());
    deck.set_agents(agents.clone());
    connect(&deck, true);
    deck.connected("tab2", GRID, hello().screen, &["keys".to_string()]);
    let log = |a: &FakeAgents| std::mem::take(&mut *a.0.lock().unwrap());
    assert_eq!(
        log(&agents),
        [
            "device tab true",
            r#"appear default/0/0/1 com.example.counter.up {"step":2}"#,
            "device tab2 true",
        ],
        "one instance, however many tablets show it"
    );
    sent.take();

    deck.key_down("tab", 1);
    deck.key_up("tab", 1);
    assert_eq!(
        log(&agents),
        ["press default/0/0/1 true", "press default/0/0/1 false"]
    );
    assert!(keys.0.lock().unwrap().is_empty());

    // The plugin draws: every tablet showing the key updates.
    let mut png = Vec::new();
    perritodeck_render::Pixmap::new(4, 4)
        .unwrap()
        .encode_png()
        .map(|p| png = p)
        .unwrap();
    deck.agent_draw("default/0/0/1", Some(Some("7".into())), Some(Some(png)));
    let msgs = sent.take();
    assert_eq!(
        msgs.iter()
            .filter(|m| matches!(m, Message::SetTitle(t) if t.title == "7" && t.key_index == 1))
            .count(),
        2
    );
    assert_eq!(
        msgs.iter()
            .filter(|m| matches!(m, Message::SetImage(_)))
            .count(),
        1,
        "only the image tablet"
    );
    deck.agent_feedback("default/0/0/1", true);
    assert_eq!(
        sent.take()
            .iter()
            .filter(|m| matches!(m, Message::ShowOk(_)))
            .count(),
        2
    );

    // Leaving the page on one tablet keeps it; the last one leaving ends it.
    deck.key_down("tab", 5);
    assert!(log(&agents).is_empty());
    deck.disconnected("tab2");
    assert_eq!(
        log(&agents),
        ["disappear default/0/0/1", "device tab2 false"]
    );
    let mut h = hello();
    h.capabilities = vec!["keys".into()];
    deck.key_down("tab", 5); // "More" isn't on page Two; nothing happens
    deck.reload(Profile::parse(SD_PROFILE).unwrap());
    assert!(log(&agents).is_empty(), "tab is on page Two");
}

/// The test profile with profile-switching keys on Main's bottom row and More's top row.
fn switching_profile() -> Profile {
    let keys = r##"
[[pages.keys]]
at = [1, 1]
title = "Stream"
nav = { profile = "streaming" }
[[pages.keys]]
at = [1, 2]
title = "Stream more"
nav = { profile = "streaming", page = "More" }

[[pages]]
name = "More"
[[pages.keys]]
at = [0, 1]
title = "Nowhere"
nav = { profile = "nope" }
[[pages.keys]]
at = [0, 2]
title = "No page"
nav = { profile = "streaming", page = "Nowhere" }
[[pages.keys]]
at = [1, 0]
title = "Here"
nav = { profile = "default", page = "Main" }
"##;
    Profile::parse(&PROFILE.replacen("\n[[pages]]\nname = \"More\"", keys, 1)).unwrap()
}

/// `(device, profile)` for each switch a key made.
type Switches = Arc<Mutex<Vec<(String, String)>>>;

fn switching_setup() -> (Arc<Deck>, Arc<Sent>, Switches) {
    let sent = Arc::new(Sent::default());
    let keys = Arc::new(Keys::default());
    let deck = Deck::new(
        switching_profile(),
        ".",
        sent.clone(),
        Executor::new(keys.clone(), keys),
    );
    let streaming = PROFILE
        .replace("Copy", "Other copy")
        .replacen("\n[[pages]]\nname = \"More\"", "\n[[pages.keys]]\nat = [1, 1]\ntitle = \"Home\"\nnav = { profile = \"default\" }\n\n[[pages]]\nname = \"More\"", 1);
    deck.load_profile("streaming", Profile::parse(&streaming).unwrap(), None);
    let switched = Arc::new(Mutex::new(Vec::new()));
    let log = switched.clone();
    deck.on_profile_switch(Arc::new(move |d: &str, p: &str| {
        log.lock().unwrap().push((d.to_owned(), p.to_owned()));
    }));
    connect(&deck, false);
    sent.take();
    (deck, sent, switched)
}

fn layout_titles(sent: &Sent) -> Vec<String> {
    match sent.take().as_slice() {
        [Message::Layout(l)] => l.keys.iter().map(|k| k.title.clone()).collect(),
        other => panic!("expected a redraw, got {other:?}"),
    }
}

#[tokio::test]
async fn a_key_switches_its_tablet_to_another_profile() {
    let (deck, sent, switched) = switching_setup();
    let rev = deck.revision();

    deck.key_down("tab", 4); // [1, 1]: streaming
    assert!(layout_titles(&sent).contains(&"Other copy".to_owned()));
    assert_eq!(deck.device_profile("tab"), "streaming");
    assert!(deck.revision() > rev, "UIs hear about it");
    assert_eq!(
        *switched.lock().unwrap(),
        [("tab".to_owned(), "streaming".to_owned())]
    );
    // Remembered like a manual choice: it's what the tablet gets when it reconnects.
    deck.disconnected("tab");
    assert!(titles(&deck.initial_layout(&hello())).contains(&"Other copy"));
    connect(&deck, false);
    sent.take();

    // And back to the default, which forgets the assignment.
    deck.key_down("tab", 4); // streaming's [1, 1]: default
    assert!(layout_titles(&sent).contains(&"Copy".to_owned()));
    assert_eq!(deck.device_profile("tab"), "default");
    assert_eq!(switched.lock().unwrap().len(), 2);
    assert_eq!(switched.lock().unwrap()[1].1, "default");
    deck.disconnected("tab");
    assert!(titles(&deck.initial_layout(&hello())).contains(&"Copy"));
}

#[tokio::test]
async fn a_profile_switch_can_open_a_page() {
    let (deck, sent, _) = switching_setup();
    deck.key_down("tab", 5); // [1, 2]: streaming, page More
    assert_eq!(deck.device_profile("tab"), "streaming");
    assert!(layout_titles(&sent).contains(&"Back".to_owned()));
    // Back history starts afresh in the new profile: Back goes to its first page.
    deck.key_down("tab", 0);
    assert!(layout_titles(&sent).contains(&"Other copy".to_owned()));

    // Switching to the profile already showing, with a page, is a page change.
    deck.key_down("tab", 4); // home to default
    sent.take();
    deck.key_down("tab", 2); // More
    sent.take();
    deck.key_down("tab", 3); // [1, 0]: default, page Main
    assert!(layout_titles(&sent).contains(&"Copy".to_owned()));
    assert_eq!(deck.device_profile("tab"), "default");
}

#[tokio::test]
async fn switching_to_a_missing_profile_or_page_flashes_an_alert() {
    let (deck, sent, switched) = switching_setup();
    deck.key_down("tab", 2); // More
    sent.take();

    deck.key_down("tab", 1); // nowhere
    match sent.take().as_slice() {
        [Message::ShowAlert(f)] => {
            assert_eq!(f.key_index, 1);
            assert!(f.message.as_deref().unwrap().contains("\"nope\""), "{f:?}");
        }
        other => panic!("expected an alert, got {other:?}"),
    }
    deck.key_down("tab", 2); // streaming, page Nowhere
    match sent.take().as_slice() {
        [Message::ShowAlert(f)] => {
            assert_eq!(f.key_index, 2);
            assert!(f.message.as_deref().unwrap().contains("Nowhere"), "{f:?}");
        }
        other => panic!("expected an alert, got {other:?}"),
    }
    assert_eq!(deck.device_profile("tab"), "default");
    assert!(switched.lock().unwrap().is_empty());
}

#[tokio::test]
async fn profile_keys_wait_while_paused() {
    let (deck, sent, switched) = switching_setup();
    deck.set_paused(true);
    sent.take();
    deck.key_down("tab", 4);
    assert!(matches!(sent.take().as_slice(), [Message::ShowAlert(_)]));
    assert_eq!(deck.device_profile("tab"), "default");
    assert!(switched.lock().unwrap().is_empty());
}

#[tokio::test]
async fn random_keys_run_one_action() {
    let sent = Arc::new(Sent::default());
    let keys = Arc::new(Keys::default());
    let profile = PROFILE.replace(
        r#"action = { type = "hotkey", keys = "Ctrl+C" }"#,
        r#"action = { type = "random", actions = [{ type = "text", text = "a" }, { type = "text", text = "b" }] }"#,
    );
    let deck = Deck::new(
        Profile::parse(&profile).unwrap(),
        ".",
        sent.clone(),
        Executor::new(keys.clone(), keys.clone()),
    );
    connect(&deck, false);
    for _ in 0..5 {
        deck.key_down("tab", 0);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let ran = keys.0.lock().unwrap().clone();
    assert_eq!(ran.len(), 5, "{ran:?}");
    assert!(
        ran.iter().all(|r| r == "text a" || r == "text b"),
        "{ran:?}"
    );
    // Instant actions don't get a ✓.
    assert!(!sent.take().iter().any(|m| matches!(m, Message::ShowOk(_))));
}
