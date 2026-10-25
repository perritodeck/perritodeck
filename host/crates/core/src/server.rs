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
    decode, DecodeError, Decoded, Envelope, ErrorCode, Grid, Hello, KeyAck, Layout, Message, Pair,
    Paired, Screen, Welcome, PROTOCOL_VERSION,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::{sleep, timeout, timeout_at},
};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::{
    accept_hdr_async_with_config,
    tungstenite::{
        self,
        handshake::server::{ErrorResponse, Request, Response},
        http,
        protocol::WebSocketConfig,
    },
    WebSocketStream,
};
use tracing::{debug, info, warn};

use crate::pairing::{PairError, PairingStore};

/// Decides which layout a device gets when it connects.
pub type LayoutProvider = Arc<dyn Fn(&Hello) -> Layout + Send + Sync>;

/// Largest frame a client may send. Client messages are tiny; this stops abuse.
const MAX_CLIENT_MESSAGE: usize = 64 * 1024;
const MAX_DEVICE_ID_LEN: usize = 128;
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a session that ended may take to send its close frame.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// Why a device's older connection is closed when it connects again.
const REPLACED: &str = "replaced by a newer connection from the same device";

/// How connections are secured.
#[derive(Clone)]
pub enum Security {
    /// TLS with the host's certificate; every device must be paired. The default.
    Paired {
        tls: TlsAcceptor,
        store: Arc<PairingStore>,
        /// Certificate fingerprint, sent to devices in `paired`.
        host_id: String,
    },
    /// Plain `ws://`, no authentication. Only for development on loopback.
    Insecure,
}

#[derive(Clone)]
pub struct ServerConfig {
    pub host_name: String,
    pub host_version: String,
    pub security: Security,
    pub heartbeat_interval: Duration,
    pub handshake_timeout: Duration,
    pub layout: LayoutProvider,
}

impl ServerConfig {
    pub fn new(host_name: impl Into<String>, security: Security, layout: LayoutProvider) -> Self {
        Self {
            host_name: host_name.into(),
            host_version: env!("CARGO_PKG_VERSION").to_owned(),
            security,
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
        screen: Screen,
        capabilities: Vec<String>,
        /// What the tablet said about itself, for diagnostics.
        platform: perritodeck_protocol::Platform,
        app_version: String,
        os_version: String,
        /// It came over the USB cable after this computer's network addresses didn't
        /// answer it (see `Hello::network_failed`).
        network_failed: bool,
    },
    Paired {
        device_id: String,
        device_name: String,
    },
    KeyDown {
        device_id: String,
        key_index: u32,
    },
    KeyUp {
        device_id: String,
        key_index: u32,
    },
    /// A slider on the tablet moved (0–100).
    SliderChange {
        device_id: String,
        key_index: u32,
        value: u8,
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

    /// Disconnect a device (e.g. after revoking it). Returns false if it isn't connected.
    pub fn kick(&self, device_id: &str) -> bool {
        let reg = self.registry.lock().expect("registry poisoned");
        reg.devices.get(device_id).is_some_and(|d| {
            d.tx.send(Outbound::Kick("disconnected by the host"))
                .is_ok()
        })
    }

    /// Ids of connected devices, sorted.
    pub fn devices(&self) -> Vec<String> {
        let reg = self.registry.lock().expect("registry poisoned");
        let mut ids: Vec<_> = reg.devices.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Make `conn_id` the device's connection and report it with `connected`. A connection
    /// it replaces is kicked and reported gone first. Both happen under the lock, so the
    /// events for a device always end with the connection it has now, whichever of the
    /// two finishes first.
    fn register(
        &self,
        device_id: &str,
        conn_id: u64,
        events: &mpsc::UnboundedSender<ServerEvent>,
        connected: ServerEvent,
    ) -> mpsc::UnboundedReceiver<Outbound> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut reg = self.registry.lock().expect("registry poisoned");
        if let Some(old) = reg
            .devices
            .insert(device_id.to_owned(), DeviceEntry { conn_id, tx })
        {
            let _ = old.tx.send(Outbound::Kick(REPLACED));
            let _ = events.send(ServerEvent::Disconnected {
                device_id: device_id.to_owned(),
                reason: REPLACED.to_owned(),
            });
        }
        let _ = events.send(connected);
        rx
    }

    /// Forget `conn_id` and report the device gone, unless a newer connection has taken
    /// its place: that was reported when it happened, and the device is still connected.
    fn unregister(
        &self,
        device_id: &str,
        conn_id: u64,
        events: &mpsc::UnboundedSender<ServerEvent>,
        reason: String,
    ) {
        let mut reg = self.registry.lock().expect("registry poisoned");
        if reg
            .devices
            .get(device_id)
            .is_some_and(|d| d.conn_id == conn_id)
        {
            reg.devices.remove(device_id);
            let _ = events.send(ServerEvent::Disconnected {
                device_id: device_id.to_owned(),
                reason,
            });
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
                if let Err(e) = conn.start(stream).await {
                    debug!(%peer, "connection ended with error: {e}");
                }
            });
        }
    }
}

struct Connection {
    id: u64,
    peer: SocketAddr,
    config: Arc<ServerConfig>,
    handle: ServerHandle,
    events: mpsc::UnboundedSender<ServerEvent>,
}

/// Browsers always send `Origin`; our clients never do. Refusing it stops any web page
/// from talking to the host, even on loopback.
#[allow(clippy::result_large_err)] // signature fixed by tungstenite's callback trait
fn reject_browsers(req: &Request, resp: Response) -> Result<Response, ErrorResponse> {
    if req.headers().contains_key(http::header::ORIGIN) {
        let mut err = ErrorResponse::new(Some("browser connections are not allowed".into()));
        *err.status_mut() = http::StatusCode::FORBIDDEN;
        return Err(err);
    }
    Ok(resp)
}

impl Connection {
    async fn start(self, stream: TcpStream) -> Result<(), tungstenite::Error> {
        match self.config.security.clone() {
            Security::Insecure => self.serve(stream).await,
            Security::Paired { tls, .. } => {
                match timeout(TLS_HANDSHAKE_TIMEOUT, tls.accept(stream)).await {
                    Ok(Ok(tls_stream)) => self.serve(tls_stream).await,
                    Ok(Err(e)) => {
                        debug!(peer = %self.peer, "TLS handshake failed: {e}");
                        Ok(())
                    }
                    Err(_) => Ok(()),
                }
            }
        }
    }

    async fn serve<S>(self, stream: S) -> Result<(), tungstenite::Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let ws_config = WebSocketConfig::default()
            .max_message_size(Some(MAX_CLIENT_MESSAGE))
            .max_frame_size(Some(MAX_CLIENT_MESSAGE));
        let mut ws =
            match accept_hdr_async_with_config(stream, reject_browsers, Some(ws_config)).await {
                Ok(ws) => ws,
                Err(e) => {
                    let _ = self.events.send(ServerEvent::Rejected {
                        peer: self.peer,
                        reason: format!("WebSocket handshake refused: {e}"),
                    });
                    return Ok(());
                }
            };

        let hello = match self.handshake(&mut ws).await? {
            Some(h) => h,
            None => return Ok(()),
        };

        let layout = (self.config.layout)(&hello);
        let grid = layout.grid();
        let device_id = hello.device_id.clone();
        let connected = ServerEvent::Connected {
            device_id: device_id.clone(),
            device_name: hello.device_name.clone(),
            peer: self.peer,
            grid,
            screen: hello.screen,
            capabilities: hello.capabilities.clone(),
            platform: hello.platform,
            app_version: hello.app_version.clone(),
            os_version: hello.os_version.clone(),
            network_failed: hello.network_failed,
        };
        let mut outbound = self
            .handle
            .register(&device_id, self.id, &self.events, connected);

        info!(peer = %self.peer, device = %device_id, name = %hello.device_name,
              "device connected ({}×{})", grid.cols, grid.rows);

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

        let reason = match &result {
            Ok(r) => r.clone(),
            Err(e) => e.to_string(),
        };
        info!(device = %device_id, "device disconnected: {reason}");
        // Reported only if this is still the device's connection: one that was replaced
        // must not take the device away from the connection that replaced it.
        self.handle
            .unregister(&device_id, self.id, &self.events, reason);
        // A tablet that vanished can't take a close frame either; don't wait for the OS.
        let _ = timeout(CLOSE_TIMEOUT, ws.close(None)).await;
        result.map(|_| ())
    }

    /// Wait for the first message: `hello` (returned if valid) or `pair` (handled, then `None`).
    async fn handshake<S>(
        &self,
        ws: &mut WebSocketStream<S>,
    ) -> Result<Option<Hello>, tungstenite::Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
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
            Ok(Decoded::Message(Envelope {
                msg: Message::Pair(p),
                ..
            })) => {
                self.pair(ws, p).await?;
                return Ok(None);
            }
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
                        "the first message must be hello or pair",
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
        if let Security::Paired { store, .. } = &self.config.security {
            let ok = hello
                .device_secret
                .as_deref()
                .is_some_and(|s| store.verify(&hello.device_id, s));
            if !ok {
                return self
                    .reject(
                        ws,
                        ErrorCode::NotPaired,
                        "this device isn't paired with this computer; pair it again",
                    )
                    .await;
            }
        }
        Ok(Some(hello))
    }

    async fn pair<S>(&self, ws: &mut WebSocketStream<S>, p: Pair) -> Result<(), tungstenite::Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let Security::Paired { store, host_id, .. } = &self.config.security else {
            self.reject::<(), _>(
                ws,
                ErrorCode::ProtocolViolation,
                "this host isn't accepting pairing",
            )
            .await?;
            return Ok(());
        };
        match store.redeem(&p.code, &p.device_id, &p.device_name) {
            Ok(device_secret) => {
                info!(peer = %self.peer, device = %p.device_id, name = %p.device_name, "device paired");
                let _ = self.events.send(ServerEvent::Paired {
                    device_id: p.device_id.clone(),
                    device_name: p.device_name.clone(),
                });
                let paired = Message::Paired(Paired {
                    device_secret,
                    host_id: host_id.clone(),
                    host_name: self.config.host_name.clone(),
                });
                send(ws, Envelope::new(paired)).await?;
                let _ = ws.close(None).await;
            }
            Err(e) => {
                let code = match e {
                    PairError::InvalidCode => ErrorCode::PairingFailed,
                    PairError::BadDeviceId => ErrorCode::BadMessage,
                };
                self.reject::<(), _>(ws, code, &e.to_string()).await?;
            }
        }
        Ok(())
    }

    async fn reject<T, S>(
        &self,
        ws: &mut WebSocketStream<S>,
        code: ErrorCode,
        reason: &str,
    ) -> Result<Option<T>, tungstenite::Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
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
    async fn session<S>(
        &self,
        ws: &mut WebSocketStream<S>,
        device_id: &str,
        grid: Grid,
        outbound: &mut mpsc::UnboundedReceiver<Outbound>,
    ) -> Result<String, tungstenite::Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let idle = self.config.idle_timeout();
        let gone = || format!("no traffic for {}s", idle.as_secs());
        // Only a frame from the tablet moves the deadline. What the host sends (a clock, live
        // values, GIF frames) says nothing about whether the tablet is still there, and a
        // tablet that vanished would otherwise stay connected until the OS gave up on the
        // socket. Every await below is bounded by it, sends too: one to a tablet that stopped
        // reading waits for room in the socket's buffers, which may never come.
        let deadline = sleep(idle);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                () = &mut deadline => return Ok(gone()),
                frame = ws.next() => {
                    let received = Instant::now();
                    deadline.as_mut().reset(tokio::time::Instant::now() + idle);
                    let text = match frame {
                        None => return Ok("socket closed".into()),
                        Some(Err(e)) => return Err(e),
                        Some(Ok(tungstenite::Message::Text(t))) => t,
                        Some(Ok(tungstenite::Message::Close(_))) => return Ok("client closed".into()),
                        Some(Ok(tungstenite::Message::Binary(_))) => {
                            let refusal = Envelope::new(Message::error(ErrorCode::BadMessage, "binary frames are not used in protocol v1"));
                            match timeout_at(deadline.deadline(), send(ws, refusal)).await {
                                Ok(sent) => sent?,
                                Err(_) => return Ok(gone()),
                            }
                            continue;
                        }
                        Some(Ok(_)) => continue, // ping/pong handled by tungstenite
                    };
                    let handled = self.on_text(ws, device_id, grid, text.as_str(), received);
                    match timeout_at(deadline.deadline(), handled).await {
                        Ok(handled) => if let Some(reason) = handled? {
                            return Ok(reason);
                        },
                        Err(_) => return Ok(gone()),
                    }
                }
                out = outbound.recv() => match out {
                    Some(Outbound::Message(m)) => {
                        match timeout_at(deadline.deadline(), send(ws, Envelope::new(m))).await {
                            Ok(sent) => sent?,
                            Err(_) => return Ok(gone()),
                        }
                    }
                    Some(Outbound::Kick(reason)) => return Ok(reason.into()),
                    None => return Ok("server shutting down".into()),
                },
            }
        }
    }

    /// Handle one text frame. Returns `Some(reason)` if the session must end.
    async fn on_text<S>(
        &self,
        ws: &mut WebSocketStream<S>,
        device_id: &str,
        grid: Grid,
        text: &str,
        received: Instant,
    ) -> Result<Option<String>, tungstenite::Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
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
            // Logs count keys from 1, as the editor counts rows and columns and the host's
            // "key 3 pressed" counts keys: keyIndex 2 is key 3.
            Message::KeyDown(k) => {
                debug!(device = %device_id, key = k.key_index + 1, "keyDown");
                let _ = self.events.send(ServerEvent::KeyDown {
                    device_id: device_id.to_owned(),
                    key_index: k.key_index,
                });
                ack(ws, env.id, k.key_index, received).await?;
            }
            Message::KeyUp(k) => {
                debug!(device = %device_id, key = k.key_index + 1, "keyUp");
                let _ = self.events.send(ServerEvent::KeyUp {
                    device_id: device_id.to_owned(),
                    key_index: k.key_index,
                });
                ack(ws, env.id, k.key_index, received).await?;
            }
            Message::SliderChange(v) if v.key_index >= grid.key_count() || v.value > 100 => {
                let msg = format!("slider {} = {} is out of range", v.key_index, v.value);
                send(
                    ws,
                    Envelope::with_id(Message::error(ErrorCode::BadMessage, msg), env.id),
                )
                .await?;
            }
            Message::SliderChange(v) => {
                debug!(device = %device_id, key = v.key_index + 1, value = v.value, "sliderChange");
                let _ = self.events.send(ServerEvent::SliderChange {
                    device_id: device_id.to_owned(),
                    key_index: v.key_index,
                    value: v.value,
                });
            }
            Message::Heartbeat(hb) => {
                send(ws, Envelope::with_id(Message::HeartbeatAck(hb), env.id)).await?;
            }
            Message::Error(e) => {
                warn!(device = %device_id, code = ?e.code, "client reported error: {}", e.message);
            }
            other => {
                let msg = format!("`{}` is not allowed here", other.type_name());
                send(
                    ws,
                    Envelope::with_id(Message::error(ErrorCode::ProtocolViolation, msg), env.id),
                )
                .await?;
            }
        }
        Ok(None)
    }
}

async fn ack<S>(
    ws: &mut WebSocketStream<S>,
    id: Option<u32>,
    key_index: u32,
    received: Instant,
) -> Result<(), tungstenite::Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
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

async fn send<S>(ws: &mut WebSocketStream<S>, env: Envelope) -> Result<(), tungstenite::Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    ws.send(tungstenite::Message::text(env.encode())).await
}

/// Next text frame, skipping control frames. `None` on close or error.
async fn next_text<S>(ws: &mut WebSocketStream<S>) -> Option<String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
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
