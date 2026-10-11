use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use perritodeck_protocol::{
    decode, DecodeError, Decoded, Envelope, ErrorCode, Grid, Hello, KeyAck, Layout, Message,
    Welcome, PROTOCOL_VERSION,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::timeout,
};
use tokio_tungstenite::{
    accept_async_with_config,
    tungstenite::{self, protocol::WebSocketConfig},
    WebSocketStream,
};
use tracing::{debug, info, warn};

use crate::auth::tokens_match;

/// Decides which layout a device gets when it connects.
pub type LayoutProvider = Arc<dyn Fn(&Hello) -> Layout + Send + Sync>;

/// Largest frame a client may send. Client messages are tiny; this stops abuse.
const MAX_CLIENT_MESSAGE: usize = 64 * 1024;
const MAX_DEVICE_ID_LEN: usize = 128;

#[derive(Clone)]
pub struct ServerConfig {
    pub host_name: String,
    pub host_version: String,
    /// When set, every `hello` must carry this token. Required whenever the
    /// server listens beyond loopback.
    pub token: Option<String>,
    pub heartbeat_interval: Duration,
    pub handshake_timeout: Duration,
    pub layout: LayoutProvider,
}

impl ServerConfig {
    pub fn new(host_name: impl Into<String>, layout: LayoutProvider) -> Self {
        Self {
            host_name: host_name.into(),
            host_version: env!("CARGO_PKG_VERSION").to_owned(),
            token: None,
            heartbeat_interval: Duration::from_secs(2),
            handshake_timeout: Duration::from_secs(10),
            layout,
        }
    }

    fn idle_timeout(&self) -> Duration {
        self.heartbeat_interval * 3
    }
}

/// Everything observable that happens on the server.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    Connected {
        device_id: String,
        device_name: String,
        peer: SocketAddr,
        grid: Grid,
    },
    KeyDown {
        device_id: String,
        key_index: u32,
    },
    KeyUp {
        device_id: String,
        key_index: u32,
    },
    Disconnected {
        device_id: String,
        reason: String,
    },
    Rejected {
        peer: SocketAddr,
        reason: String,
    },
}

enum Outbound {
    Message(Message),
    Kick(&'static str),
}

struct DeviceEntry {
    conn_id: u64,
    tx: mpsc::UnboundedSender<Outbound>,
}

#[derive(Default)]
struct Registry {
    devices: HashMap<String, DeviceEntry>,
}

/// Cheap, cloneable handle for pushing messages to connected devices.
#[derive(Clone, Default)]
pub struct ServerHandle {
    registry: Arc<Mutex<Registry>>,
}

impl ServerHandle {
    /// Send a message to one device. Returns false if it isn't connected.
    pub fn send(&self, device_id: &str, msg: Message) -> bool {
        let reg = self.registry.lock().expect("registry poisoned");
        reg.devices
            .get(device_id)
            .is_some_and(|d| d.tx.send(Outbound::Message(msg)).is_ok())
    }

    /// Send a message to every connected device.
    pub fn broadcast(&self, msg: Message) {
        let reg = self.registry.lock().expect("registry poisoned");
        for d in reg.devices.values() {
            let _ = d.tx.send(Outbound::Message(msg.clone()));
        }
    }

    /// Ids of connected devices, sorted.
    pub fn devices(&self) -> Vec<String> {
        let reg = self.registry.lock().expect("registry poisoned");
        let mut ids: Vec<_> = reg.devices.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn register(&self, device_id: &str, conn_id: u64) -> mpsc::UnboundedReceiver<Outbound> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut reg = self.registry.lock().expect("registry poisoned");
        if let Some(old) = reg
            .devices
            .insert(device_id.to_owned(), DeviceEntry { conn_id, tx })
        {
            let _ = old.tx.send(Outbound::Kick(
                "replaced by a newer connection from the same device",
            ));
        }
        rx
    }

    fn unregister(&self, device_id: &str, conn_id: u64) {
        let mut reg = self.registry.lock().expect("registry poisoned");
        if reg
            .devices
            .get(device_id)
            .is_some_and(|d| d.conn_id == conn_id)
        {
            reg.devices.remove(device_id);
        }
    }
}

pub struct Server {
    listener: TcpListener,
    config: Arc<ServerConfig>,
    handle: ServerHandle,
    events: mpsc::UnboundedSender<ServerEvent>,
}

impl Server {
    pub fn new(
        listener: TcpListener,
        config: ServerConfig,
    ) -> (Self, mpsc::UnboundedReceiver<ServerEvent>) {
        let (events, rx) = mpsc::unbounded_channel();
        let server = Self {
            listener,
            config: Arc::new(config),
            handle: ServerHandle::default(),
            events,
        };
        (server, rx)
    }

    pub fn handle(&self) -> ServerHandle {
        self.handle.clone()
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Accept connections until the task is dropped.
    pub async fn run(self) -> io::Result<()> {
        static NEXT_CONN: AtomicU64 = AtomicU64::new(1);
        loop {
            let (stream, peer) = self.listener.accept().await?;
            let _ = stream.set_nodelay(true);
            let conn = Connection {
                id: NEXT_CONN.fetch_add(1, Ordering::Relaxed),
                peer,
                config: self.config.clone(),
                handle: self.handle.clone(),
                events: self.events.clone(),
            };
            tokio::spawn(async move {
                if let Err(e) = conn.run(stream).await {
                    debug!(%peer, "connection ended with error: {e}");
                }
            });
        }
    }
}

type Ws = WebSocketStream<TcpStream>;

struct Connection {
    id: u64,
    peer: SocketAddr,
    config: Arc<ServerConfig>,
    handle: ServerHandle,
    events: mpsc::UnboundedSender<ServerEvent>,
}

impl Connection {
    async fn run(self, stream: TcpStream) -> Result<(), tungstenite::Error> {
        let ws_config = WebSocketConfig::default()
            .max_message_size(Some(MAX_CLIENT_MESSAGE))
            .max_frame_size(Some(MAX_CLIENT_MESSAGE));
        let mut ws = accept_async_with_config(stream, Some(ws_config)).await?;

        let hello = match self.handshake(&mut ws).await? {
            Some(h) => h,
            None => return Ok(()),
        };

        let layout = (self.config.layout)(&hello);
        let grid = layout.grid();
        let device_id = hello.device_id.clone();
        let mut outbound = self.handle.register(&device_id, self.id);

        info!(peer = %self.peer, device = %device_id, name = %hello.device_name,
              "device connected ({}×{})", grid.cols, grid.rows);
        let _ = self.events.send(ServerEvent::Connected {
            device_id: device_id.clone(),
            device_name: hello.device_name.clone(),
            peer: self.peer,
            grid,
        });

        let welcome = Message::Welcome(Welcome {
            host_name: self.config.host_name.clone(),
            host_version: self.config.host_version.clone(),
            protocol_version: PROTOCOL_VERSION,
            heartbeat_interval_ms: self.config.heartbeat_interval.as_millis() as u32,
        });
        let result = async {
            send(&mut ws, Envelope::new(welcome)).await?;
            send(&mut ws, Envelope::new(Message::Layout(layout))).await?;
            self.session(&mut ws, &device_id, grid, &mut outbound).await
        }
        .await;

        self.handle.unregister(&device_id, self.id);
        let reason = match &result {
            Ok(r) => r.clone(),
            Err(e) => e.to_string(),
        };
        info!(device = %device_id, "device disconnected: {reason}");
        let _ = self
            .events
            .send(ServerEvent::Disconnected { device_id, reason });
        let _ = ws.close(None).await;
        result.map(|_| ())
    }

    /// Wait for a valid `hello`. Returns `None` (after telling the client why) if it isn't one.
    async fn handshake(&self, ws: &mut Ws) -> Result<Option<Hello>, tungstenite::Error> {
        let text = match timeout(self.config.handshake_timeout, next_text(ws)).await {
            Ok(Some(t)) => t,
            Ok(None) => return Ok(None),
            Err(_) => {
                return self
                    .reject(
                        ws,
                        ErrorCode::ProtocolViolation,
                        "no hello within the handshake timeout",
                    )
                    .await
            }
        };
        let hello = match decode(&text) {
            Ok(Decoded::Message(Envelope {
                msg: Message::Hello(h),
                ..
            })) => h,
            Err(DecodeError::UnsupportedVersion(v)) => {
                let msg =
                    format!("this host speaks protocol v{PROTOCOL_VERSION}, client sent v{v}");
                return self.reject(ws, ErrorCode::UnsupportedVersion, &msg).await;
            }
            Err(e @ DecodeError::InvalidPayload { .. }) => {
                return self.reject(ws, ErrorCode::BadMessage, &e.to_string()).await
            }
            _ => {
                return self
                    .reject(
                        ws,
                        ErrorCode::ProtocolViolation,
                        "the first message must be hello",
                    )
                    .await
            }
        };
        if hello.device_id.is_empty() || hello.device_id.len() > MAX_DEVICE_ID_LEN {
            return self
                .reject(
                    ws,
                    ErrorCode::BadMessage,
                    "deviceId must be 1–128 characters",
                )
                .await;
        }
        if let Some(expected) = &self.config.token {
            let ok = hello
                .token
                .as_deref()
                .is_some_and(|t| tokens_match(expected, t));
            if !ok {
                return self
                    .reject(
                        ws,
                        ErrorCode::Unauthorized,
                        "missing or wrong session token; copy it from the host's console",
                    )
                    .await;
            }
        }
        Ok(Some(hello))
    }

    async fn reject<T>(
        &self,
        ws: &mut Ws,
        code: ErrorCode,
        reason: &str,
    ) -> Result<Option<T>, tungstenite::Error> {
        warn!(peer = %self.peer, ?code, "rejected connection: {reason}");
        let _ = self.events.send(ServerEvent::Rejected {
            peer: self.peer,
            reason: reason.to_owned(),
        });
        send(ws, Envelope::new(Message::error(code, reason))).await?;
        let _ = ws.close(None).await;
        Ok(None)
    }

    /// Main loop after the handshake. Returns the reason the session ended.
    async fn session(
        &self,
        ws: &mut Ws,
        device_id: &str,
        grid: Grid,
        outbound: &mut mpsc::UnboundedReceiver<Outbound>,
    ) -> Result<String, tungstenite::Error> {
        let idle = self.config.idle_timeout();
        loop {
            tokio::select! {
                frame = timeout(idle, ws.next()) => {
                    let received = Instant::now();
                    let text = match frame {
                        Err(_) => return Ok(format!("no traffic for {}s", idle.as_secs())),
                        Ok(None) => return Ok("socket closed".into()),
                        Ok(Some(Err(e))) => return Err(e),
                        Ok(Some(Ok(tungstenite::Message::Text(t)))) => t,
                        Ok(Some(Ok(tungstenite::Message::Close(_)))) => return Ok("client closed".into()),
                        Ok(Some(Ok(tungstenite::Message::Binary(_)))) => {
                            send(ws, Envelope::new(Message::error(ErrorCode::BadMessage, "binary frames are not used in protocol v1"))).await?;
                            continue;
                        }
                        Ok(Some(Ok(_))) => continue, // ping/pong handled by tungstenite
                    };
                    if let Some(reason) = self.on_text(ws, device_id, grid, text.as_str(), received).await? {
                        return Ok(reason);
                    }
                }
                out = outbound.recv() => match out {
                    Some(Outbound::Message(m)) => send(ws, Envelope::new(m)).await?,
                    Some(Outbound::Kick(reason)) => return Ok(reason.into()),
                    None => return Ok("server shutting down".into()),
                },
            }
        }
    }

    /// Handle one text frame. Returns `Some(reason)` if the session must end.
    async fn on_text(
        &self,
        ws: &mut Ws,
        device_id: &str,
        grid: Grid,
        text: &str,
        received: Instant,
    ) -> Result<Option<String>, tungstenite::Error> {
        let env = match decode(text) {
            Ok(Decoded::Message(env)) => env,
            Ok(Decoded::UnknownType { type_name, .. }) => {
                debug!(device = %device_id, "ignoring unknown message type `{type_name}`");
                return Ok(None);
            }
            Err(DecodeError::UnsupportedVersion(v)) => {
                let msg =
                    format!("this host speaks protocol v{PROTOCOL_VERSION}, client sent v{v}");
                send(
                    ws,
                    Envelope::new(Message::error(ErrorCode::UnsupportedVersion, &msg)),
                )
                .await?;
                return Ok(Some(msg));
            }
            Err(e) => {
                send(
                    ws,
                    Envelope::new(Message::error(ErrorCode::BadMessage, e.to_string())),
                )
                .await?;
                return Ok(None);
            }
        };

        match env.msg {
            Message::KeyDown(k) | Message::KeyUp(k) if k.key_index >= grid.key_count() => {
                let msg = format!(
                    "keyIndex {} is outside the {}×{} layout",
                    k.key_index, grid.cols, grid.rows
                );
                send(
                    ws,
                    Envelope::with_id(Message::error(ErrorCode::BadMessage, msg), env.id),
                )
                .await?;
            }
            Message::KeyDown(k) => {
                debug!(device = %device_id, key = k.key_index, "keyDown");
                let _ = self.events.send(ServerEvent::KeyDown {
                    device_id: device_id.to_owned(),
                    key_index: k.key_index,
                });
                self.ack(ws, env.id, k.key_index, received).await?;
            }
            Message::KeyUp(k) => {
                debug!(device = %device_id, key = k.key_index, "keyUp");
                let _ = self.events.send(ServerEvent::KeyUp {
                    device_id: device_id.to_owned(),
                    key_index: k.key_index,
                });
                self.ack(ws, env.id, k.key_index, received).await?;
            }
            Message::Heartbeat(hb) => {
                send(ws, Envelope::with_id(Message::HeartbeatAck(hb), env.id)).await?;
            }
            Message::Error(e) => {
                warn!(device = %device_id, code = ?e.code, "client reported error: {}", e.message);
            }
            other => {
                let msg = format!("`{}` is not a client-to-host message", other.type_name());
                send(
                    ws,
                    Envelope::with_id(Message::error(ErrorCode::ProtocolViolation, msg), env.id),
                )
                .await?;
            }
        }
        Ok(None)
    }

    async fn ack(
        &self,
        ws: &mut Ws,
        id: Option<u32>,
        key_index: u32,
        received: Instant,
    ) -> Result<(), tungstenite::Error> {
        let host_processing_us = received.elapsed().as_micros() as u64;
        send(
            ws,
            Envelope::with_id(
                Message::KeyAck(KeyAck {
                    key_index,
                    host_processing_us,
                }),
                id,
            ),
        )
        .await
    }
}

async fn send(ws: &mut Ws, env: Envelope) -> Result<(), tungstenite::Error> {
    ws.send(tungstenite::Message::text(env.encode())).await
}

/// Next text frame, skipping control frames. `None` on close or error.
async fn next_text(ws: &mut Ws) -> Option<String> {
    while let Some(frame) = ws.next().await {
        match frame.ok()? {
            tungstenite::Message::Text(t) => return Some(t.as_str().to_owned()),
            tungstenite::Message::Close(_) => return None,
            tungstenite::Message::Binary(_) => return Some(String::new()),
            _ => continue,
        }
    }
    None
}
