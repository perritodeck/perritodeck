use std::sync::Mutex;

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::mpsc,
};

use super::*;

fn surface() -> Surface {
    Surface {
        id: "tab-1".into(),
        name: "PerritoDeck Fire 7".into(),
        cols: 5,
        rows: 3,
        bitmap_px: Some(36),
    }
}

/// A fake Companion: hands each accepted connection to the test.
async fn companion() -> (u16, mpsc::UnboundedReceiver<tokio::net::TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            let _ = tx.send(s);
        }
    });
    (port, rx)
}

struct Conn {
    lines: tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
    write: tokio::net::tcp::OwnedWriteHalf,
}

impl Conn {
    fn new(s: tokio::net::TcpStream) -> Self {
        let (r, w) = s.into_split();
        Self {
            lines: BufReader::new(r).lines(),
            write: w,
        }
    }
    async fn say(&mut self, text: &str) {
        self.write.write_all(text.as_bytes()).await.unwrap();
        self.write.write_all(b"\n").await.unwrap();
    }
    /// The next line that isn't a PING.
    async fn hear(&mut self) -> Line {
        loop {
            let l = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
                .await
                .expect("the client said something")
                .unwrap()
                .expect("connection open");
            let l = line::parse(&l);
            if l.command != "PING" {
                return l;
            }
        }
    }
}

fn recorder() -> (EventSink, Arc<Mutex<Vec<Event>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    (Arc::new(move |e| s.lock().unwrap().push(e)), seen)
}

async fn wait_for(seen: &Mutex<Vec<Event>>, f: impl Fn(&Event) -> bool) -> Event {
    for _ in 0..100 {
        if let Some(e) = seen.lock().unwrap().iter().find(|e| f(e)).cloned() {
            return e;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("event never came: {:?}", seen.lock().unwrap());
}

#[tokio::test]
async fn registers_tablets_shows_buttons_and_sends_presses() {
    let (port, mut accepted) = companion().await;
    let (sink, seen) = recorder();
    let client = Client::start("127.0.0.1".into(), port, sink);
    client.add(surface());

    let mut c = Conn::new(accepted.recv().await.unwrap());
    c.say("BEGIN CompanionVersion=4.1.0 ApiVersion=1.12.0")
        .await;
    c.say("CAPS SUBSCRIPTIONS=1 BITMAP_FORMATS=rgb,png").await;
    let add = c.hear().await;
    assert_eq!(add.command, "ADD-DEVICE");
    assert_eq!(add.get("DEVICEID"), Some("tab-1"));
    assert_eq!(add.get("PRODUCT_NAME"), Some("PerritoDeck Fire 7"));
    assert_eq!(add.get("KEYS_TOTAL"), Some("15"));
    assert_eq!(add.get("KEYS_PER_ROW"), Some("5"));
    assert_eq!(add.get("BITMAPS"), Some("36"));
    assert_eq!(add.get("BITMAP_FORMAT"), Some("png"));
    assert_eq!(add.get("SERIAL"), Some("perritodeck:tab-1"));
    c.say("ADD-DEVICE OK DEVICEID=tab-1").await;
    wait_for(&seen, |e| {
        matches!(e, Event::Status(Status::Connected { .. }))
    })
    .await;

    // Raw RGB (older Companions) is turned into PNG; text is base64.
    let rgb = STANDARD.encode(vec![255u8; 36 * 36 * 3]);
    let text = STANDARD.encode("Live\nCam 2");
    c.say(&format!(
        "KEY-STATE DEVICEID=tab-1 KEY=4 TYPE=BUTTON BITMAP={rgb} COLOR=rgb(255,0,16) TEXTCOLOR=#FFFFFF TEXT={text} PRESSED=0"
    ))
    .await;
    let Event::Key { key, state, .. } = wait_for(&seen, |e| matches!(e, Event::Key { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(key, 4);
    assert_eq!(state.color.as_deref(), Some("#ff0010"));
    assert_eq!(state.text_color.as_deref(), Some("#ffffff"));
    assert_eq!(state.text.as_deref(), Some("Live\nCam 2"));
    let png = image::load_from_memory(&state.png.unwrap()).unwrap();
    assert_eq!((png.width(), png.height()), (36, 36));

    // PNG data URLs pass straight through.
    let mut tiny = Vec::new();
    image::RgbImage::from_pixel(2, 2, image::Rgb([0, 0, 0]))
        .write_to(
            &mut std::io::Cursor::new(&mut tiny),
            image::ImageFormat::Png,
        )
        .unwrap();
    c.say(&format!(
        "KEY-STATE DEVICEID=tab-1 KEY=5 BITMAP=data:image/png;base64,{}",
        STANDARD.encode(&tiny)
    ))
    .await;
    let e = wait_for(&seen, |e| matches!(e, Event::Key { key: 5, .. })).await;
    assert!(matches!(e, Event::Key { state: KeyState { png: Some(p), .. }, .. } if p == tiny));

    // Presses go to Companion; its pings are answered.
    client.press("tab-1", 4, true);
    let p = c.hear().await;
    assert_eq!(p.command, "KEY-PRESS");
    assert_eq!((p.get("KEY"), p.get("PRESSED")), (Some("4"), Some("true")));
    client.press("tab-1", 4, false);
    assert_eq!(c.hear().await.get("PRESSED"), Some("false"));
    client.press("unknown", 1, true); // not registered: dropped
    c.say("PING abc").await;
    let pong = c.hear().await;
    assert_eq!(
        (pong.command.as_str(), pong.words.clone()),
        ("PONG", vec!["abc".to_string()])
    );

    c.say("KEYS-CLEAR DEVICEID=tab-1").await;
    wait_for(&seen, |e| matches!(e, Event::Clear { .. })).await;

    // Companion restarts: the client reconnects and registers the tablet again.
    drop(c);
    let mut c = Conn::new(accepted.recv().await.unwrap());
    c.say("BEGIN CompanionVersion=3.4.0 ApiVersion=1.5.0").await;
    let add = c.hear().await;
    assert_eq!(add.command, "ADD-DEVICE");
    assert_eq!(add.get("BITMAP_FORMAT"), None, "no png without CAPS");
    assert_eq!(add.get("COLORS"), Some("true"), "older API: a boolean");
    assert_eq!(add.get("SERIAL"), None);

    client.remove("tab-1");
    let rm = c.hear().await;
    assert_eq!(
        (rm.command.as_str(), rm.get("DEVICEID")),
        ("REMOVE-DEVICE", Some("tab-1"))
    );
}

#[tokio::test]
async fn says_why_it_cant_connect() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let (sink, seen) = recorder();
    let _client = Client::start("127.0.0.1".into(), port, sink);
    let e = wait_for(&seen, |e| {
        matches!(e, Event::Status(Status::Disconnected { .. }))
    })
    .await;
    let Event::Status(Status::Disconnected { reason }) = e else {
        unreachable!()
    };
    assert!(reason.contains("Couldn't reach Companion"), "{reason}");
}

#[test]
fn colours_and_versions() {
    assert_eq!(css_hex("#A0B1C2").as_deref(), Some("#a0b1c2"));
    assert_eq!(css_hex("rgb(1,2,3)").as_deref(), Some("#010203"));
    assert_eq!(css_hex("red"), None);
    assert_eq!(parse_version("1.12.0"), (1, 12, 0));
    assert_eq!(parse_version("1.5"), (1, 5, 0));
}

#[tokio::test]
async fn notices_a_companion_that_goes_silent() {
    let (port, mut accepted) = companion().await;
    let (sink, seen) = recorder();
    let _client = Client::start("127.0.0.1".into(), port, sink);
    let mut c = Conn::new(accepted.recv().await.unwrap());
    c.say("BEGIN CompanionVersion=4.1.0 ApiVersion=1.12.0")
        .await;
    c.say("CAPS BITMAP_FORMATS=rgb").await;
    wait_for(&seen, |e| {
        matches!(e, Event::Status(Status::Connected { .. }))
    })
    .await;
    // Keep the connection open but never answer the pings.
    for _ in 0..300 {
        if seen.lock().unwrap().iter().any(|e| matches!(e, Event::Status(Status::Disconnected { reason }) if reason.contains("stopped answering"))) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("silence went unnoticed: {:?}", seen.lock().unwrap());
}

#[tokio::test]
async fn refuses_endless_lines() {
    let (port, mut accepted) = companion().await;
    let (sink, seen) = recorder();
    let _client = Client::start("127.0.0.1".into(), port, sink);
    let mut c = Conn::new(accepted.recv().await.unwrap());
    c.say("BEGIN CompanionVersion=4.1.0 ApiVersion=1.5.0").await;
    let chunk = vec![b'A'; 1024 * 1024];
    for _ in 0..5 {
        if c.write.write_all(&chunk).await.is_err() {
            break;
        }
    }
    wait_for(&seen, |e| matches!(e, Event::Status(Status::Disconnected { reason }) if reason.contains("too long"))).await;
}

// ---- probe (Test connection) ----

const WAIT: Duration = Duration::from_millis(300);

/// The next connection the fake Companion took, within 5 s.
async fn next_conn(
    accepted: &mut mpsc::UnboundedReceiver<tokio::net::TcpStream>,
) -> tokio::net::TcpStream {
    tokio::time::timeout(Duration::from_secs(5), accepted.recv())
        .await
        .expect("the probe connected")
        .unwrap()
}

#[tokio::test]
async fn probe_reads_begin_and_hangs_up() {
    let (port, mut accepted) = companion().await;
    let probe = tokio::spawn(probe("127.0.0.1", port, WAIT));
    let mut c = Conn::new(next_conn(&mut accepted).await);
    c.say("BEGIN CompanionVersion=4.1.0 ApiVersion=1.12.0")
        .await;
    c.say("CAPS SUBSCRIPTIONS=1 BITMAP_FORMATS=rgb,png").await;
    assert_eq!(
        probe.await.unwrap().unwrap(),
        Greeting {
            companion_version: "4.1.0".into(),
            api: "1.12.0".into()
        }
    );
    // It registers nothing: the next thing Companion hears is goodbye.
    assert_eq!(c.hear().await.command, "QUIT");
}

#[tokio::test]
async fn probe_names_a_port_that_isnt_satellite() {
    let (port, mut accepted) = companion().await;
    let probe = tokio::spawn(probe("127.0.0.1", port, WAIT));
    let mut c = Conn::new(next_conn(&mut accepted).await);
    c.say("HTTP/1.1 400 Bad Request").await;
    let e = probe.await.unwrap().unwrap_err();
    assert!(
        matches!(&e, ProbeError::NotSatellite(w) if w == "HTTP/1.1"),
        "{e:?}"
    );
}

#[tokio::test]
async fn probe_notices_silence_and_hang_ups() {
    let (port, mut accepted) = companion().await;
    let probe1 = tokio::spawn(probe("127.0.0.1", port, WAIT));
    let _quiet = next_conn(&mut accepted).await;
    let e = probe1.await.unwrap().unwrap_err();
    assert!(matches!(e, ProbeError::Silent), "{e:?}");
    let probe2 = tokio::spawn(probe("127.0.0.1", port, WAIT));
    drop(next_conn(&mut accepted).await);
    let e = probe2.await.unwrap().unwrap_err();
    assert!(
        matches!(e, ProbeError::Closed | ProbeError::Lost(_)),
        "{e:?}"
    );
}

#[tokio::test]
async fn probe_refused() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    // Windows takes about 2 s to give up on a port nobody listens on.
    let e = probe("127.0.0.1", port, Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(
        matches!(&e, ProbeError::Io(io) if io.kind() == std::io::ErrorKind::ConnectionRefused),
        "{e:?}"
    );
}
