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
