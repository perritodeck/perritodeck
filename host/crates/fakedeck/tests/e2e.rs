//! End-to-end tests: a real host server on an ephemeral port, driven by the fakedeck client.

use std::{sync::Arc, time::Duration};

use perritodeck_core::{demo, Server, ServerConfig, ServerEvent, ServerHandle};
use perritodeck_fakedeck::{fake_hello, now_ms, Client};
use perritodeck_protocol::{Decoded, ErrorCode, Grid, Heartbeat, Message, SetTitle};
use tokio::{net::TcpListener, sync::mpsc::UnboundedReceiver, time::timeout};

const T: Duration = Duration::from_secs(5);

struct Host {
    url: String,
    handle: ServerHandle,
    events: UnboundedReceiver<ServerEvent>,
}

async fn start(token: Option<&str>, heartbeat: Duration) -> Host {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = ServerConfig::new("test-host", Arc::new(demo::layout_for));
    cfg.token = token.map(str::to_owned);
    cfg.heartbeat_interval = heartbeat;
    cfg.handshake_timeout = Duration::from_millis(500);
    let (server, events) = Server::new(listener, cfg);
    let url = format!("ws://{}", server.local_addr().unwrap());
    let handle = server.handle();
    tokio::spawn(server.run());
    Host {
        url,
        handle,
        events,
    }
}

async fn next_event(h: &mut Host) -> ServerEvent {
    timeout(T, h.events.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

fn grid(c: u32, r: u32) -> Option<Grid> {
    Some(Grid { cols: c, rows: r })
}

#[tokio::test]
async fn handshake_press_and_ack() {
    let mut h = start(None, Duration::from_secs(2)).await;
    let (mut c, welcome, layout) = Client::handshake(&h.url, fake_hello("dev-1", None, grid(4, 2)))
        .await
        .unwrap();
    assert_eq!(welcome.protocol_version, 1);
    assert_eq!(welcome.host_name, "test-host");
    assert_eq!((layout.cols, layout.rows, layout.keys.len()), (4, 2, 8));
    assert!(
        matches!(next_event(&mut h).await, ServerEvent::Connected { device_id, grid: g, .. }
        if device_id == "dev-1" && g == Grid { cols: 4, rows: 2 })
    );

    let id = c.key_down(3).await.unwrap();
    let ack = c.recv_msg(T).await.unwrap();
    assert_eq!(ack.id, Some(id), "keyAck echoes the keyDown id");
    assert!(matches!(ack.msg, Message::KeyAck(a) if a.key_index == 3));
    assert_eq!(
        next_event(&mut h).await,
        ServerEvent::KeyDown {
            device_id: "dev-1".into(),
            key_index: 3
        }
    );

    c.key_up(3).await.unwrap();
    c.recv_msg(T).await.unwrap();
    assert_eq!(
        next_event(&mut h).await,
        ServerEvent::KeyUp {
            device_id: "dev-1".into(),
            key_index: 3
        }
    );

    c.close().await;
    assert!(
        matches!(next_event(&mut h).await, ServerEvent::Disconnected { device_id, .. } if device_id == "dev-1")
    );
}

#[tokio::test]
async fn token_is_enforced() {
    let mut h = start(Some("s3cret"), Duration::from_secs(2)).await;
    for bad in [None, Some("wrong".to_owned())] {
        let err = Client::handshake(&h.url, fake_hello("dev", bad, None))
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("Unauthorized"), "{err}");
        assert!(matches!(
            next_event(&mut h).await,
            ServerEvent::Rejected { .. }
        ));
    }
    Client::handshake(&h.url, fake_hello("dev", Some("s3cret".into()), None))
        .await
        .unwrap();
}

#[tokio::test]
async fn first_message_must_be_hello() {
    let h = start(None, Duration::from_secs(2)).await;
    let mut c = Client::connect(&h.url).await.unwrap();
    c.send(Message::Heartbeat(Heartbeat {
        seq: 1,
        sent_at_ms: 0,
    }))
    .await
    .unwrap();
    let env = c.recv_msg(T).await.unwrap();
    assert!(matches!(env.msg, Message::Error(e) if e.code == ErrorCode::ProtocolViolation));
    assert!(
        c.recv().await.unwrap().is_none(),
        "host closes after a protocol violation"
    );
}

#[tokio::test]
async fn silent_client_times_out_in_handshake() {
    let h = start(None, Duration::from_secs(2)).await;
    let mut c = Client::connect(&h.url).await.unwrap();
    let env = c.recv_msg(T).await.unwrap();
    assert!(matches!(env.msg, Message::Error(e) if e.code == ErrorCode::ProtocolViolation));
}

#[tokio::test]
async fn wrong_major_version_is_refused() {
    let h = start(None, Duration::from_secs(2)).await;
    let mut c = Client::connect(&h.url).await.unwrap();
    c.send_raw(r#"{"v":2,"type":"hello","payload":{}}"#)
        .await
        .unwrap();
    let env = c.recv_msg(T).await.unwrap();
    assert!(matches!(env.msg, Message::Error(e) if e.code == ErrorCode::UnsupportedVersion));
}

#[tokio::test]
async fn heartbeat_unknown_types_and_bad_keys() {
    let h = start(None, Duration::from_secs(2)).await;
    let (mut c, _, _) = Client::handshake(&h.url, fake_hello("dev", None, grid(2, 2)))
        .await
        .unwrap();

    // Unknown types are ignored silently; the next reply is the heartbeat ack.
    c.send_raw(r#"{"v":1,"type":"dialRotate","payload":{"ticks":1}}"#)
        .await
        .unwrap();
    let hb = Heartbeat {
        seq: 9,
        sent_at_ms: now_ms(),
    };
    c.send(Message::Heartbeat(hb)).await.unwrap();
    assert_eq!(c.recv_msg(T).await.unwrap().msg, Message::HeartbeatAck(hb));

    // Out-of-range key → badMessage, connection stays up.
    let id = c.key_down(4).await.unwrap();
    let env = c.recv_msg(T).await.unwrap();
    assert_eq!(env.id, Some(id));
    assert!(matches!(env.msg, Message::Error(e) if e.code == ErrorCode::BadMessage));

    // Malformed payload → badMessage, connection stays up.
    c.send_raw(r#"{"v":1,"type":"keyDown","payload":{"keyIndex":"one"}}"#)
        .await
        .unwrap();
    assert!(
        matches!(c.recv_msg(T).await.unwrap().msg, Message::Error(e) if e.code == ErrorCode::BadMessage)
    );

    // Host-only message from a client → protocolViolation, connection stays up.
    c.send(Message::SetTitle(SetTitle {
        key_index: 0,
        title: "x".into(),
    }))
    .await
    .unwrap();
    assert!(
        matches!(c.recv_msg(T).await.unwrap().msg, Message::Error(e) if e.code == ErrorCode::ProtocolViolation)
    );

    c.key_down(1).await.unwrap();
    assert!(matches!(
        c.recv_msg(T).await.unwrap().msg,
        Message::KeyAck(_)
    ));
}

#[tokio::test]
async fn host_can_push_titles() {
    let mut h = start(None, Duration::from_secs(2)).await;
    let (mut c, _, _) = Client::handshake(&h.url, fake_hello("dev", None, None))
        .await
        .unwrap();
    next_event(&mut h).await;
    assert_eq!(h.handle.devices(), vec!["dev".to_owned()]);
    let msg = Message::SetTitle(SetTitle {
        key_index: 0,
        title: "12:00:00".into(),
    });
    assert!(h.handle.send("dev", msg.clone()));
    assert!(!h.handle.send("nobody", msg.clone()));
    assert_eq!(c.recv_msg(T).await.unwrap().msg, msg);
}

#[tokio::test]
async fn reconnect_with_same_device_id_replaces_old_connection() {
    let mut h = start(None, Duration::from_secs(2)).await;
    let (mut old, _, _) = Client::handshake(&h.url, fake_hello("same", None, None))
        .await
        .unwrap();
    next_event(&mut h).await;
    let (_new, _, _) = Client::handshake(&h.url, fake_hello("same", None, None))
        .await
        .unwrap();
    // The old socket is closed by the host.
    let closed = timeout(T, async {
        loop {
            match old.recv().await {
                Ok(None) | Err(_) => break,
                Ok(Some(Decoded::Message(_) | Decoded::UnknownType { .. })) => continue,
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "old connection should be closed");
    assert_eq!(
        h.handle.devices(),
        vec!["same".to_owned()],
        "new connection stays registered"
    );
}

#[tokio::test]
async fn idle_client_is_dropped() {
    let mut h = start(None, Duration::from_millis(100)).await;
    let (mut c, _, _) = Client::handshake(&h.url, fake_hello("idle", None, None))
        .await
        .unwrap();
    next_event(&mut h).await;
    assert!(
        matches!(next_event(&mut h).await, ServerEvent::Disconnected { reason, .. } if reason.contains("no traffic"))
    );
    assert!(timeout(T, c.recv())
        .await
        .unwrap()
        .map(|o| o.is_none())
        .unwrap_or(true));
}
