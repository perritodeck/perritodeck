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

/// Wait for the first plugin's status to become `want`.
async fn until_status(host: &SdHost, want: Status) {
    for _ in 0..200 {
        if host.list().first().is_some_and(|p| p.status == want) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never saw {want:?}: {:?}", host.list());
}

/// The app's Restart button runs on a thread that isn't one of Tokio's (a Tauri command
/// on the UI thread): restarting from there must still start the plugin again.
#[tokio::test(flavor = "multi_thread")]
async fn restarts_from_a_thread_outside_the_runtime() {
    let dir = tempfile::tempdir().unwrap();
    // No Node.js needed: starting the plugin and finding Node missing is enough.
    let host = SdHost::start_with_node(
        dir.path().join("sdplugins"),
        "perritodeck-no-such-node".into(),
    )
    .await
    .unwrap();
    let uuid = host.install(&fixture()).unwrap();
    until_status(&host, Status::NeedsNode).await;

    let (sd, id) = (host.clone(), uuid.clone());
    let restarted = std::thread::spawn(move || sd.restart(&id)).join();
    assert!(
        matches!(restarted, Ok(Ok(()))),
        "restart from a plain thread: {restarted:?}"
    );
    // A new entry starts as Starting; only its supervisor sets NeedsNode.
    until_status(&host, Status::NeedsNode).await;

    let (sd, id) = (host.clone(), uuid.clone());
    let removed = std::thread::spawn(move || sd.remove(&id)).join();
    assert!(matches!(removed, Ok(Ok(()))), "remove: {removed:?}");
    assert!(host.list().is_empty());
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

#[tokio::test]
async fn web_pages_cant_pose_as_plugins() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let dir = tempfile::tempdir().unwrap();
    let host = SdHost::start(dir.path().join("sdplugins")).await.unwrap();
    let url = format!("ws://127.0.0.1:{}", host.port());
    let mut req = url.as_str().into_client_request().unwrap();
    req.headers_mut()
        .insert("Origin", "https://evil.example".parse().unwrap());
    assert!(
        tokio_tungstenite::connect_async(req).await.is_err(),
        "refused"
    );
    assert!(
        tokio_tungstenite::connect_async(url.as_str()).await.is_ok(),
        "plugins (no Origin) still can"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_pages_talk_to_their_plugin() {
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as Ws};
    if !node_available() {
        eprintln!("skipped: needs Node.js 22 or newer");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let host = SdHost::start(dir.path().join("sdplugins")).await.unwrap();
    host.install(&fixture()).unwrap();
    assert!(host.list()[0].actions[0].has_settings_page);
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
        &["keys".into()],
    );
    until(
        &sent,
        "title 0",
        |m| matches!(m, Message::SetTitle(t) if t.title == "0"),
    )
    .await;

    let key = deck
        .agent_key_for("default", "Main", 0, 0)
        .expect("a Stream Deck key");
    let url = host.settings_page(&key).unwrap();
    let rest = url.strip_prefix("http://127.0.0.1:").unwrap();
    let (port, path) = rest.split_once('/').unwrap();
    let get = |path: String| {
        let port = port.to_owned();
        async move {
            let mut s = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            s.write_all(format!("GET /{path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            out
        }
    };
    let page = get(path.to_owned()).await;
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(
        page.contains("registerPropertyInspector")
            && page.contains("connectElgatoStreamDeckSocket")
    );
    assert!(
        get(path.replacen("pi/", "pi/0", 1))
            .await
            .starts_with("HTTP/1.1 404"),
        "wrong secret"
    );
    assert!(get(path.replace("ui/up.html", "../../settings.json"))
        .await
        .starts_with("HTTP/1.1 404"));

    // Act as the page: connect from the settings origin, change a setting, talk to the plugin.
    let mut req = format!("ws://127.0.0.1:{}", host.port())
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "Origin",
        format!("http://127.0.0.1:{port}").parse().unwrap(),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let send = |v: serde_json::Value| Ws::Text(v.to_string().into());
    ws.send(send(
        serde_json::json!({ "event": "registerPropertyInspector", "uuid": key.context }),
    ))
    .await
    .unwrap();
    ws.send(send(serde_json::json!({ "event": "setSettings", "context": key.context, "payload": { "step": 3, "count": 10 } }))).await.unwrap();
    until(
        &sent,
        "title from new settings",
        |m| matches!(m, Message::SetTitle(t) if t.title == "10"),
    )
    .await;
    ws.send(send(serde_json::json!({ "event": "sendToPlugin", "action": key.action, "context": key.context, "payload": { "ping": 1 } }))).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(Ws::Text(t))) = ws.next().await {
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            if v["event"] == "sendToPropertyInspector" {
                return v;
            }
        }
        panic!("closed")
    })
    .await
    .unwrap();
    assert_eq!(reply["payload"]["pong"]["ping"], 1);

    // Pages from other origins still can't connect.
    let mut bad = format!("ws://127.0.0.1:{}", host.port())
        .into_client_request()
        .unwrap();
    bad.headers_mut()
        .insert("Origin", "http://127.0.0.1:1".parse().unwrap());
    assert!(tokio_tungstenite::connect_async(bad).await.is_err());
    host.shutdown();
}
