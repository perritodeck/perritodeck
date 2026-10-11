//! A real Stream Deck plugin (a small Node.js fixture) driving a key.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use perritodeck_actions::{ActionError, Chord, Executor, InputBackend, MediaKey, SystemBackend};
use perritodeck_deck::{Deck, Outbox};
use perritodeck_profile::Profile;
use perritodeck_protocol::{Grid, Message, Screen};
use perritodeck_sdcompat::{SdHost, Status};

#[derive(Default)]
struct Sent(Mutex<Vec<Message>>);

impl Outbox for Sent {
    fn send(&self, _device: &str, msg: Message) -> bool {
        self.0.lock().unwrap().push(msg);
        true
    }
}

struct Nothing;
impl InputBackend for Nothing {
    fn chord(&self, _: &Chord) -> Result<(), ActionError> {
        Ok(())
    }
    fn text(&self, _: &str) -> Result<(), ActionError> {
        Ok(())
    }
    fn media(&self, _: MediaKey) -> Result<(), ActionError> {
        Ok(())
    }
}
impl SystemBackend for Nothing {
    fn open(&self, _: &str) -> Result<(), ActionError> {
        Ok(())
    }
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/com.perritodeck.test-counter.sdPlugin")
}

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("-e")
        .arg("if (typeof WebSocket !== 'function') process.exit(1)")
        .status()
        .is_ok_and(|s| s.success())
}

async fn until(sent: &Sent, what: &str, f: impl Fn(&Message) -> bool) {
    for _ in 0..200 {
        if sent.0.lock().unwrap().iter().any(&f) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never saw {what}: {:?}", sent.0.lock().unwrap());
}

const PROFILE: &str = r##"
schema_version = 1
name = "sd"
[grid]
cols = 3
rows = 2
[[pages]]
name = "Main"
[[pages.keys]]
at = [0, 0]
action = { type = "sdPlugin", plugin = "com.perritodeck.test-counter", action = "com.perritodeck.test-counter.up", settings = { step = 2 } }
"##;

#[tokio::test(flavor = "multi_thread")]
async fn a_node_plugin_runs_a_key() {
    if !node_available() {
        eprintln!("skipped: needs Node.js 22 or newer");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let host = SdHost::start(dir.path().join("sdplugins")).await.unwrap();
    let uuid = host.install(&fixture()).unwrap();
    assert_eq!(uuid, "com.perritodeck.test-counter");
    let list = host.list();
    assert_eq!(list[0].actions.len(), 1, "dial-only actions aren't offered");
    assert!(
        list[0]
            .icon
            .as_deref()
            .unwrap()
            .starts_with("data:image/png;base64,"),
        "the SVG icon is drawn"
    );

    let sent = Arc::new(Sent::default());
    let deck = Deck::new(
        Profile::parse(PROFILE).unwrap(),
        dir.path(),
        sent.clone(),
        Executor::new(Arc::new(Nothing), Arc::new(Nothing)),
    );
    host.attach(&deck);
    deck.connected(
        "tab",
        Grid { cols: 3, rows: 2 },
        Screen {
            width_px: 1024,
            height_px: 600,
            density_dpi: 160,
        },
        &["keys".into(), "images".into()],
    );

    // The plugin starts, registers, gets willAppear and sets the title.
    until(
        &sent,
        "title 0",
        |m| matches!(m, Message::SetTitle(t) if t.key_index == 0 && t.title == "0"),
    )
    .await;
    assert!(host.list()[0].status == Status::Running);

    deck.key_down("tab", 0);
    deck.key_up("tab", 0);
    until(
        &sent,
        "title 2",
        |m| matches!(m, Message::SetTitle(t) if t.title == "2"),
    )
    .await;
    until(&sent, "✓", |m| matches!(m, Message::ShowOk(_))).await;

    // Settings it saved survive a restart of the plugin.
    for _ in 0..100 {
        let saved =
            std::fs::read_to_string(host.folder().join("settings.json")).unwrap_or_default();
        if saved.contains("\"count\": 2") && saved.contains("\"presses\": 2") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    sent.0.lock().unwrap().clear();
    host.restart(&uuid).unwrap();
    until(
        &sent,
        "title 2 after restart",
        |m| matches!(m, Message::SetTitle(t) if t.title == "2"),
    )
    .await;

    host.remove(&uuid).unwrap();
    assert!(host.list().is_empty());
    host.shutdown();
}

#[tokio::test]
async fn refuses_things_that_arent_plugins() {
    let dir = tempfile::tempdir().unwrap();
    let host = SdHost::start(dir.path().join("sdplugins")).await.unwrap();
    std::fs::write(dir.path().join("x.streamDeckPlugin"), b"not a zip").unwrap();
    assert!(host
        .install(&dir.path().join("x.streamDeckPlugin"))
        .unwrap_err()
        .contains("isn't a Stream Deck plugin"));
    std::fs::create_dir_all(host.folder().join("junk.sdPlugin")).unwrap();
    host.scan();
    assert_eq!(host.broken().len(), 1);
}
