//! End-to-end tests: a real host server on an ephemeral port, driven by the fakedeck client.

use std::{sync::Arc, time::Duration};

use perritodeck_core::{
    demo, HostIdentity, PairingStore, Security, Server, ServerConfig, ServerEvent, ServerHandle,
};
use perritodeck_fakedeck::{fake_hello, now_ms, Client, ClientSecurity, Endpoint};
use perritodeck_protocol::{Decoded, ErrorCode, Grid, Heartbeat, Message, SetTitle};
use tokio::{net::TcpListener, sync::mpsc::UnboundedReceiver, time::timeout};

const T: Duration = Duration::from_secs(5);

struct Host {
    addr: String,
    handle: ServerHandle,
    events: UnboundedReceiver<ServerEvent>,
    /// Present in paired (TLS) mode.
    store: Option<Arc<PairingStore>>,
    fingerprint: Option<String>,
}

impl Host {
    fn insecure_ep(&self) -> Endpoint {
        Endpoint {
            addr: self.addr.clone(),
            security: ClientSecurity::Insecure,
        }
    }
    fn tofu_ep(&self) -> Endpoint {
        Endpoint {
            addr: self.addr.clone(),
            security: ClientSecurity::TrustOnFirstUse,
        }
    }
    fn pinned_ep(&self) -> Endpoint {
        Endpoint {
            addr: self.addr.clone(),
            security: ClientSecurity::Pinned(self.fingerprint.clone().unwrap()),
        }
    }
}

async fn start(security: Security, heartbeat: Duration) -> Host {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (store, fingerprint) = match &security {
        Security::Paired { store, host_id, .. } => (Some(store.clone()), Some(host_id.clone())),
        Security::Insecure => (None, None),
    };
    let mut cfg = ServerConfig::new("test-host", security, Arc::new(demo::layout_for));
    cfg.heartbeat_interval = heartbeat;
    cfg.handshake_timeout = Duration::from_millis(500);
    let (server, events) = Server::new(listener, cfg);
    let addr = server.local_addr().unwrap().to_string();
    let handle = server.handle();
    tokio::spawn(server.run());
    Host {
        addr,
        handle,
        events,
        store,
        fingerprint,
    }
}

async fn insecure() -> Host {
    start(Security::Insecure, Duration::from_secs(2)).await
}

async fn paired_mode() -> Host {
    let id = HostIdentity::generate();
    start(
        Security::Paired {
            tls: id.tls_acceptor(),
            store: Arc::new(PairingStore::in_memory()),
            host_id: id.fingerprint().to_owned(),
        },
        Duration::from_secs(2),
    )
    .await
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

// ---------- protocol behaviour (insecure mode keeps these focused) ----------

#[tokio::test]
async fn handshake_press_and_ack() {
    let mut h = insecure().await;
    let (mut c, welcome, layout) =
        Client::handshake(&h.insecure_ep(), fake_hello("dev-1", None, grid(4, 2)))
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
async fn first_message_must_be_hello_or_pair() {
    let h = insecure().await;
    let mut c = Client::connect(&h.insecure_ep()).await.unwrap();
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
    let h = insecure().await;
    let mut c = Client::connect(&h.insecure_ep()).await.unwrap();
    let env = c.recv_msg(T).await.unwrap();
    assert!(matches!(env.msg, Message::Error(e) if e.code == ErrorCode::ProtocolViolation));
}

#[tokio::test]
async fn wrong_major_version_is_refused() {
    let h = insecure().await;
    let mut c = Client::connect(&h.insecure_ep()).await.unwrap();
    c.send_raw(r#"{"v":2,"type":"hello","payload":{}}"#)
        .await
        .unwrap();
    let env = c.recv_msg(T).await.unwrap();
    assert!(matches!(env.msg, Message::Error(e) if e.code == ErrorCode::UnsupportedVersion));
}

#[tokio::test]
async fn browsers_are_refused() {
    let h = insecure().await;
    let result = Client::connect_with(&h.insecure_ep(), |req| {
        req.headers_mut()
            .insert("Origin", "https://evil.example".parse().unwrap());
    })
    .await;
    assert!(
        result.is_err(),
        "a request with an Origin header must be rejected"
    );
}

#[tokio::test]
async fn heartbeat_unknown_types_and_bad_keys() {
    let h = insecure().await;
    let (mut c, _, _) = Client::handshake(&h.insecure_ep(), fake_hello("dev", None, grid(2, 2)))
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
    let mut h = insecure().await;
    let (mut c, _, _) = Client::handshake(&h.insecure_ep(), fake_hello("dev", None, None))
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
    let mut h = insecure().await;
    let (mut old, _, _) = Client::handshake(&h.insecure_ep(), fake_hello("same", None, None))
        .await
        .unwrap();
    next_event(&mut h).await;
    let (_new, _, _) = Client::handshake(&h.insecure_ep(), fake_hello("same", None, None))
        .await
        .unwrap();
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
    let mut h = start(Security::Insecure, Duration::from_millis(100)).await;
    let (mut c, _, _) = Client::handshake(&h.insecure_ep(), fake_hello("idle", None, None))
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

// ---------- pairing + TLS ----------

#[tokio::test]
async fn pair_then_connect_over_pinned_tls() {
    let mut h = paired_mode().await;
    h.store
        .as_ref()
        .unwrap()
        .add_offer("135790".into(), Duration::from_secs(60));

    let paired = Client::pair(&h.tofu_ep(), "135790", "tab-1", "Tablet")
        .await
        .unwrap();
    assert_eq!(
        Some(&paired.host_id),
        h.fingerprint.as_ref(),
        "paired reports the cert fingerprint"
    );
    assert_eq!(paired.host_name, "test-host");
    assert!(
        matches!(next_event(&mut h).await, ServerEvent::Paired { device_id, .. } if device_id == "tab-1")
    );

    let hello = fake_hello("tab-1", Some(paired.device_secret.clone()), None);
    let (mut c, welcome, _) = Client::handshake(&h.pinned_ep(), hello).await.unwrap();
    assert_eq!(welcome.host_name, "test-host");
    c.key_down(0).await.unwrap();
    assert!(matches!(
        c.recv_msg(T).await.unwrap().msg,
        Message::KeyAck(_)
    ));
}

#[tokio::test]
async fn wrong_code_and_reused_code_fail() {
    let h = paired_mode().await;
    h.store
        .as_ref()
        .unwrap()
        .add_offer("111111".into(), Duration::from_secs(60));
    let err = Client::pair(&h.tofu_ep(), "222222", "tab", "T")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("PairingFailed"), "{err}");
    Client::pair(&h.tofu_ep(), "111111", "tab", "T")
        .await
        .unwrap();
    let err = Client::pair(&h.tofu_ep(), "111111", "tab2", "T")
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("PairingFailed"),
        "codes are single use: {err}"
    );
}

#[tokio::test]
async fn unpaired_or_wrong_secret_is_refused() {
    let h = paired_mode().await;
    for secret in [None, Some("not-the-secret".to_owned())] {
        let err = Client::handshake(&h.pinned_ep(), fake_hello("stranger", secret, None))
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("NotPaired"), "{err}");
    }
}

#[tokio::test]
async fn wrong_pinned_fingerprint_is_refused_by_the_client() {
    let h = paired_mode().await;
    let impostor_pin = Endpoint {
        addr: h.addr.clone(),
        security: ClientSecurity::Pinned(HostIdentity::generate().fingerprint().to_owned()),
    };
    let err = Client::connect(&impostor_pin)
        .await
        .err()
        .expect("must not connect");
    assert!(format!("{err:#}").contains("does not match"), "{err:#}");
}

#[tokio::test]
async fn plain_websocket_to_tls_host_fails() {
    let h = paired_mode().await;
    assert!(
        Client::handshake(&h.insecure_ep(), fake_hello("x", None, None))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn revoked_device_is_kicked_and_cannot_return() {
    let mut h = paired_mode().await;
    let store = h.store.clone().unwrap();
    store.add_offer("246810".into(), Duration::from_secs(60));
    let paired = Client::pair(&h.tofu_ep(), "246810", "tab", "T")
        .await
        .unwrap();
    next_event(&mut h).await; // Paired
    let hello = fake_hello("tab", Some(paired.device_secret.clone()), None);
    let (mut c, _, _) = Client::handshake(&h.pinned_ep(), hello.clone())
        .await
        .unwrap();
    next_event(&mut h).await; // Connected

    assert!(store.revoke("tab"));
    assert!(h.handle.kick("tab"));
    let closed = timeout(T, async { while let Ok(Some(_)) = c.recv().await {} }).await;
    assert!(closed.is_ok(), "kicked connection closes");

    let err = Client::handshake(&h.pinned_ep(), hello)
        .await
        .err()
        .unwrap();
    assert!(err.to_string().contains("NotPaired"), "{err}");
}
