//! OBS Studio via obs-websocket 5 (built into OBS 28+).
//!
//! One connection is opened on first use and reused; if it drops, the next request
//! reconnects. Live values and sliders poll with `Get…` requests; while OBS can't be
//! reached they wait between attempts (5 s, doubling up to 30 s) instead of trying on
//! every poll, and never wait for someone else's attempt. Key presses and slider moves
//! always try. A request OBS may already have received is sent again only if doing it
//! twice changes nothing (`repeatable`). Starting a stream opens a connection of its own,
//! to hear whether the stream went live.
//! Protocol: <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>

use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{FutureExt, SinkExt, StreamExt};
use perritodeck_actions::{ActionError, ObsAction, Outcome};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{
    net::TcpStream,
    sync::{Mutex, MutexGuard},
    time::timeout,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error as WsError, Message as WsMessage},
    MaybeTlsStream, WebSocketStream,
};
use tracing::debug;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const RPC_VERSION: u64 = 1;
const TIMEOUT: Duration = Duration::from_secs(5);
/// How long polls wait after a failed connection attempt; doubles each time.
const BACKOFF_FIRST: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// How long starting a stream waits to hear whether it went live, well within the 10 s
/// a key's action may take. OBS gives up on a server that refuses after about 2 s.
const STREAM_START_WAIT: Duration = Duration::from_secs(6);
/// How often it asks meanwhile whether the stream is active, for an OBS with a Stream
/// Delay, which says STARTED only when the delay is over.
const STREAM_ASK_EVERY: Duration = Duration::from_millis(250);

// EventSubscription bits for Identify: the shared connection takes no events; starting
// a stream listens for Outputs (StreamStateChanged and the like).
const NO_EVENTS: u64 = 0;
const OUTPUT_EVENTS: u64 = 1 << 6;
// outputState in StreamStateChanged.
const OUTPUT_STARTING: &str = "OBS_WEBSOCKET_OUTPUT_STARTING";
const OUTPUT_STARTED: &str = "OBS_WEBSOCKET_OUTPUT_STARTED";
const OUTPUT_STOPPED: &str = "OBS_WEBSOCKET_OUTPUT_STOPPED";

// WebSocketCloseCode: why OBS closed the connection.
const CLOSE_AUTH_FAILED: u16 = 4009;
const CLOSE_UNSUPPORTED_RPC: u16 = 4010;

// RequestStatus codes OBS sends, often without a comment.
const NOT_READY: u64 = 207;
const UNKNOWN_REQUEST_TYPE: u64 = 204;
const OUTPUT_RUNNING: u64 = 500;
const OUTPUT_NOT_RUNNING: u64 = 501;
const OUTPUT_PAUSED: u64 = 502;
const OUTPUT_NOT_PAUSED: u64 = 503;
const OUTPUT_DISABLED: u64 = 504;
const STUDIO_MODE_ACTIVE: u64 = 505;
const STUDIO_MODE_NOT_ACTIVE: u64 = 506;
const RESOURCE_NOT_FOUND: u64 = 600;
const INVALID_RESOURCE_STATE: u64 = 604;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObsConfig {
    #[serde(default = "default_url")]
    pub url: String,
    #[serde(default)]
    pub password: Option<String>,
}

fn default_url() -> String {
    "ws://127.0.0.1:4455".into()
}

pub struct ObsClient {
    config: ObsConfig,
    /// The open connection. Locked while a request is on the wire, never while
    /// connecting, so a slow or failing connection attempt doesn't hold up others.
    conn: Mutex<Option<Ws>>,
    /// One connection attempt at a time.
    connecting: Mutex<()>,
    backoff: std::sync::Mutex<Backoff>,
    timeout: Duration,
    stream_wait: Duration,
    next_id: std::sync::atomic::AtomicU64,
}

fn err(reason: impl Into<String>) -> ActionError {
    ActionError::Integration {
        service: "OBS".into(),
        reason: reason.into(),
    }
}

/// A warning on the key, worded like an error: "OBS: not recording".
fn alert(reason: impl Into<String>) -> Outcome {
    Outcome::Alert(err(reason).to_string())
}

/// What to suggest when connecting failed. Nothing answering at the address (refused,
/// or no answer at all) means OBS is closed or its WebSocket server is off; other
/// failures (a name that doesn't resolve, something else on that port) say what they are.
fn connect_hint(e: &WsError) -> &'static str {
    match e {
        WsError::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::TimedOut
            ) =>
        {
            " (is the WebSocket server enabled?)"
        }
        _ => "",
    }
}

/// What starting a stream came to, from the StreamStateChanged events OBS sends: live
/// (`Some(true)`), or back to stopped (`Some(false)`). A STOPPED counts only after
/// STARTING, so one that ends an earlier stream can't pass for this start failing.
/// With a Stream Delay set in OBS (Settings → Advanced) the stream is active at once,
/// but STARTED comes only when the delay is over; so an answer to GetStreamStatus that
/// says it's active counts as live too. While OBS is still connecting it isn't active.
#[derive(Default)]
struct StreamStart {
    starting: bool,
    went_live: Option<bool>,
}

impl StreamStart {
    fn heard(&mut self, msg: &Value) {
        let d = &msg["d"];
        if msg["op"] == 7 && d["requestType"] == "GetStreamStatus" {
            if d["responseData"]["outputActive"] == true {
                self.went_live = self.went_live.or(Some(true));
            }
            return;
        }
        if msg["op"] != 5 || d["eventType"] != "StreamStateChanged" {
            return;
        }
        match d["eventData"]["outputState"].as_str() {
            Some(OUTPUT_STARTING) => self.starting = true,
            Some(OUTPUT_STARTED) => self.went_live = self.went_live.or(Some(true)),
            Some(OUTPUT_STOPPED) if self.starting => {
                self.went_live = self.went_live.or(Some(false))
            }
            _ => {}
        }
    }
}

/// `base64(sha256(base64(sha256(password + salt)) + challenge))`
pub fn auth_string(password: &str, salt: &str, challenge: &str) -> String {
    let secret = STANDARD.encode(Sha256::digest(format!("{password}{salt}").as_bytes()));
    STANDARD.encode(Sha256::digest(format!("{secret}{challenge}").as_bytes()))
}

/// The obs-websocket request for an action.
pub fn request_for(action: &ObsAction) -> (&'static str, Value) {
    match action {
        ObsAction::Scene { scene } => ("SetCurrentProgramScene", json!({ "sceneName": scene })),
        ObsAction::ToggleRecord => ("ToggleRecord", json!({})),
        ObsAction::StartRecord => ("StartRecord", json!({})),
        ObsAction::StopRecord => ("StopRecord", json!({})),
        ObsAction::ToggleStream => ("ToggleStream", json!({})),
        ObsAction::StartStream => ("StartStream", json!({})),
        ObsAction::StopStream => ("StopStream", json!({})),
        ObsAction::ToggleMute { input } => ("ToggleInputMute", json!({ "inputName": input })),
        ObsAction::ToggleReplayBuffer => ("ToggleReplayBuffer", json!({})),
        ObsAction::SaveReplayBuffer => ("SaveReplayBuffer", json!({})),
    }
}

/// Requests that come out the same however often OBS gets them, so one whose answer
/// went missing can be sent again. Everything else (`Toggle…`, `Start…`, `Stop…`,
/// `Save…`) goes at most once per press: a slow answer must never toggle twice.
fn repeatable(request_type: &str) -> bool {
    request_type.starts_with("Get") || request_type.starts_with("Set")
}

/// Live values and sliders read OBS with `Get…` requests, every few seconds.
fn is_poll(request_type: &str) -> bool {
    request_type.starts_with("Get")
}

/// What to say when OBS refuses a request: its own comment, or for the status codes
/// it sends without one, a plain explanation. Read after "OBS: ".
fn refusal(request_type: &str, data: &Value, status: &Value) -> String {
    let code = status["code"].as_u64().unwrap_or(0);
    let replay = request_type.contains("ReplayBuffer");
    // OBS answers "Replay buffer is not available." when it's turned off in the
    // settings; say where to turn it on.
    if replay && matches!(code, OUTPUT_DISABLED | INVALID_RESOURCE_STATE) {
        return "the replay buffer is turned off (Settings → Output → Replay Buffer)".into();
    }
    // While starting or quitting, OBS answers "OBS is not ready to perform the request.",
    // which doesn't say to try again (and would read "OBS: OBS is…").
    if code == NOT_READY {
        return "it isn't ready yet; try again in a moment".into();
    }
    if let Some(comment) = status["comment"].as_str().filter(|c| !c.trim().is_empty()) {
        return comment.to_owned();
    }
    let output = if request_type.contains("Record") {
        Some("recording")
    } else if request_type.contains("Stream") {
        Some("streaming")
    } else {
        None
    };
    match (code, output) {
        (OUTPUT_RUNNING, Some(doing)) => format!("already {doing}"),
        (OUTPUT_RUNNING, None) if replay => "the replay buffer is already running".into(),
        (OUTPUT_RUNNING, None) => "it's already running".into(),
        (OUTPUT_NOT_RUNNING, Some(doing)) => format!("not {doing}"),
        (OUTPUT_NOT_RUNNING, None) if replay => "the replay buffer isn't running".into(),
        (OUTPUT_NOT_RUNNING, None) => "it isn't running".into(),
        (OUTPUT_PAUSED, _) => "the recording is already paused".into(),
        (OUTPUT_NOT_PAUSED, _) => "the recording isn't paused".into(),
        (OUTPUT_DISABLED, _) => "that's turned off in OBS's settings".into(),
        (STUDIO_MODE_ACTIVE, _) => "Studio Mode is on".into(),
        (STUDIO_MODE_NOT_ACTIVE, _) => "Studio Mode is off".into(),
        (RESOURCE_NOT_FOUND, _) => match (data["sceneName"].as_str(), data["inputName"].as_str()) {
            (Some(scene), _) => format!("there's no scene called “{scene}”"),
            (_, Some(input)) => format!("there's no source called “{input}”"),
            _ => "it couldn't find that".into(),
        },
        (INVALID_RESOURCE_STATE, _) => "it can't do that right now".into(),
        (UNKNOWN_REQUEST_TYPE, _) => "this version of OBS can't do that; update OBS".into(),
        _ => format!("it refused {request_type} (error {code})"),
    }
}

/// After a failed connection attempt, polls wait before trying again: 5 s, then twice
/// as long after each further failure, up to 30 s. A connection resets it.
#[derive(Debug, Default)]
struct Backoff {
    last: Option<Failed>,
}

#[derive(Debug)]
struct Failed {
    /// When the failed attempt began.
    started: Instant,
    /// When it failed; the wait counts from here.
    at: Instant,
    wait: Duration,
    reason: String,
}

impl Backoff {
    fn failed(&mut self, started: Instant, now: Instant, reason: String) {
        let wait = match &self.last {
            Some(f) => (f.wait * 2).min(BACKOFF_MAX),
            None => BACKOFF_FIRST,
        };
        self.last = Some(Failed {
            started,
            at: now,
            wait,
            reason,
        });
    }

    fn succeeded(&mut self) {
        self.last = None;
    }

    /// Why not to try yet, while still waiting after a failure.
    fn waiting(&self, now: Instant) -> Option<String> {
        self.last
            .as_ref()
            .filter(|f| now < f.at + f.wait)
            .map(|f| f.reason.clone())
    }

    /// The failure of an attempt someone else began at or after `since`, while we
    /// waited. One that began earlier may have started before OBS was listening, so
    /// it says nothing about now.
    fn failed_since(&self, since: Instant) -> Option<String> {
        self.last
            .as_ref()
            .filter(|f| f.started >= since)
            .map(|f| f.reason.clone())
    }
}

/// Why reading from OBS stopped.
enum ReadError {
    TimedOut,
    /// The connection closed; OBS's close code and reason, when it sent them.
    Closed(Option<(u16, String)>),
    Broken(String),
    Garbled(String),
}

impl ReadError {
    fn describe(&self) -> String {
        match self {
            ReadError::TimedOut => "OBS didn't answer in time".into(),
            ReadError::Closed(Some((_, why))) if !why.is_empty() => {
                format!("OBS closed the connection: {why}")
            }
            ReadError::Closed(_) => "OBS closed the connection".into(),
            ReadError::Broken(e) => format!("connection problem: {e}"),
            ReadError::Garbled(e) => format!("unexpected reply: {e}"),
        }
    }

    /// The same, for a request OBS may already have acted on.
    fn maybe_happened(&self) -> String {
        match self {
            ReadError::TimedOut => {
                "OBS didn't answer in time, so it may or may not have happened".into()
            }
            ReadError::Closed(_) => {
                "OBS closed the connection before answering, so it may or may not have happened"
                    .into()
            }
            ReadError::Broken(_) | ReadError::Garbled(_) => {
                format!("{}; it may or may not have happened", self.describe())
            }
        }
    }
}

/// Why a request got no answer, and whether OBS may have acted on it.
enum Failure {
    /// It never left: the connection had already closed, or sending failed.
    NotSent(String),
    /// It was sent, but no answer came.
    MaybeSent(ReadError),
}

async fn next_json(ws: &mut Ws, wait: Duration) -> Result<Value, ReadError> {
    loop {
        let frame = timeout(wait, ws.next())
            .await
            .map_err(|_| ReadError::TimedOut)?
            .ok_or(ReadError::Closed(None))?
            .map_err(|e| ReadError::Broken(e.to_string()))?;
        match frame {
            WsMessage::Text(t) => {
                return serde_json::from_str(t.as_str())
                    .map_err(|e| ReadError::Garbled(e.to_string()))
            }
            WsMessage::Close(c) => {
                return Err(ReadError::Closed(
                    c.map(|c| (u16::from(c.code), c.reason.to_string())),
                ))
            }
            _ => continue,
        }
    }
}

/// A request (op 6), as obs-websocket takes it.
fn request_message(id: &str, request_type: &str, data: &Value) -> WsMessage {
    let req = json!({ "op": 6, "d": { "requestType": request_type, "requestId": id, "requestData": data } });
    WsMessage::text(req.to_string())
}

/// Whether the connection has already closed (OBS quit or restarted since the last
/// request), from what's waiting on it, without waiting. A late answer to an earlier
/// request is skipped.
fn closed_already(ws: &mut Ws) -> bool {
    loop {
        match ws.next().now_or_never() {
            None => return false,
            Some(None | Some(Err(_)) | Some(Ok(WsMessage::Close(_)))) => return true,
            Some(Some(Ok(_))) => continue,
        }
    }
}

impl ObsClient {
    pub fn new(config: ObsConfig) -> Self {
        Self {
            config,
            conn: Mutex::new(None),
            connecting: Mutex::new(()),
            backoff: Default::default(),
            timeout: TIMEOUT,
            stream_wait: STREAM_START_WAIT,
            next_id: Default::default(),
        }
    }

    /// A new connection that hears the `events` (EventSubscription bits) OBS sends.
    async fn connect(&self, events: u64) -> Result<Ws, ActionError> {
        let url = &self.config.url;
        // obs-websocket listens on plain ws://. For anything else the WebSocket library
        // says only "URL scheme not supported" or "TLS support not compiled in".
        if url.starts_with("wss://") {
            return Err(err("OBS's WebSocket server uses ws://, so the address in \
                            integrations.toml must start with ws://, not wss://"));
        }
        if !url.starts_with("ws://") {
            return Err(err(format!(
                "the address {url} in integrations.toml must start with ws://, like \
                 ws://127.0.0.1:4455"
            )));
        }
        let (mut ws, _) = timeout(self.timeout, connect_async(url))
            .await
            .map_err(|_| err(format!("can't reach OBS at {url} (is OBS open?)")))?
            .map_err(|e| err(format!("can't reach OBS at {url}: {e}{}", connect_hint(&e))))?;
        // Hello (op 0) → Identify (op 1) → Identified (op 2)
        let refused = |e: ReadError| match e {
            ReadError::Closed(Some((_, why))) if !why.is_empty() => {
                err(format!("OBS refused the connection: {why}"))
            }
            other => err(other.describe()),
        };
        let hello = next_json(&mut ws, self.timeout).await.map_err(refused)?;
        if hello["op"] != 0 {
            return Err(err("unexpected greeting from OBS"));
        }
        let mut identify =
            json!({ "op": 1, "d": { "rpcVersion": RPC_VERSION, "eventSubscriptions": events } });
        let auth = hello["d"].get("authentication");
        if let Some(auth) = auth {
            let password = self
                .config
                .password
                .as_deref()
                .ok_or_else(|| err("OBS needs a password; add it to integrations.toml"))?;
            let (salt, challenge) = (
                auth["salt"].as_str().unwrap_or(""),
                auth["challenge"].as_str().unwrap_or(""),
            );
            identify["d"]["authentication"] = json!(auth_string(password, salt, challenge));
        }
        ws.send(WsMessage::text(identify.to_string()))
            .await
            .map_err(|e| err(e.to_string()))?;
        let wrong_password = || err("OBS rejected the password in integrations.toml");
        // OBS closes with 4009 "Authentication failed." for a wrong password.
        let identified = next_json(&mut ws, self.timeout)
            .await
            .map_err(|e| match e {
                ReadError::Closed(Some((CLOSE_AUTH_FAILED, _))) => wrong_password(),
                ReadError::Closed(Some((CLOSE_UNSUPPORTED_RPC, _))) => {
                    err("this OBS speaks a different obs-websocket version; update OBS")
                }
                ReadError::Closed(Some((_, why))) if !why.is_empty() => {
                    err(format!("OBS refused the connection: {why}"))
                }
                ReadError::Closed(_) if auth.is_some() => wrong_password(),
                other => err(other.describe()),
            })?;
        if identified["op"] != 2 {
            return Err(err("OBS didn't accept the connection"));
        }
        debug!("connected to OBS at {}", self.config.url);
        Ok(ws)
    }

    /// The open connection, connecting first if there's none. Polls don't try while
    /// waiting after a failure, or while someone else is connecting.
    async fn connection(&self, poll: bool) -> Result<MutexGuard<'_, Option<Ws>>, ActionError> {
        {
            let guard = self.conn.lock().await;
            if guard.is_some() {
                return Ok(guard);
            }
        }
        let backoff = || self.backoff.lock().expect("backoff poisoned");
        let started = Instant::now();
        let _gate = if poll {
            if let Some(why) = backoff().waiting(started) {
                return Err(err(why));
            }
            self.connecting
                .try_lock()
                .map_err(|_| err("still connecting to OBS"))?
        } else {
            self.connecting.lock().await
        };
        // Someone else may have connected, or failed to, while we waited.
        {
            let guard = self.conn.lock().await;
            if guard.is_some() {
                return Ok(guard);
            }
        }
        if let Some(why) = backoff().failed_since(started) {
            return Err(err(why));
        }
        let ws = self.connect_noted(NO_EVENTS).await?;
        let mut guard = self.conn.lock().await;
        *guard = Some(ws);
        Ok(guard)
    }

    /// [`connect`](Self::connect), and tell the polls how it went: after a failure they
    /// wait before trying again, and after a connection (OBS is back) they needn't.
    async fn connect_noted(&self, events: u64) -> Result<Ws, ActionError> {
        let attempt = Instant::now();
        let connected = self.connect(events).await;
        let mut backoff = self.backoff.lock().expect("backoff poisoned");
        match &connected {
            Ok(_) => backoff.succeeded(),
            Err(e) => {
                let reason = match e {
                    ActionError::Integration { reason, .. } => reason.clone(),
                    other => other.to_string(),
                };
                backoff.failed(attempt, Instant::now(), reason);
            }
        }
        connected
    }

    /// Send a request and wait for its answer; anything else that arrives first goes to
    /// `heard`.
    async fn request(
        ws: &mut Ws,
        wait: Duration,
        id: &str,
        request_type: &str,
        data: &Value,
        mut heard: impl FnMut(&Value),
    ) -> Result<Value, Failure> {
        if closed_already(ws) {
            return Err(Failure::NotSent("OBS closed the connection".into()));
        }
        ws.send(request_message(id, request_type, data))
            .await
            .map_err(|e| Failure::NotSent(e.to_string()))?;
        loop {
            let msg = next_json(ws, wait).await.map_err(Failure::MaybeSent)?;
            if msg["op"] == 7 && msg["d"]["requestId"] == id {
                return Ok(msg["d"].clone());
            }
            heard(&msg);
        }
    }

    fn next_id(&self) -> String {
        self.next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .to_string()
    }

    /// Send any request; resolves to its `responseData` (or `{}`), or an error saying
    /// why OBS refused.
    pub async fn call(&self, request_type: &str, data: Value) -> Result<Value, ActionError> {
        let resp = self
            .send(request_type, &data, is_poll(request_type))
            .await?;
        let status = &resp["requestStatus"];
        if status["result"].as_bool().unwrap_or(false) {
            Ok(resp
                .get("responseData")
                .cloned()
                .unwrap_or_else(|| json!({})))
        } else {
            Err(err(refusal(request_type, &data, status)))
        }
    }

    pub async fn run(&self, action: &ObsAction) -> Result<Outcome, ActionError> {
        let (request_type, data) = request_for(action);
        if matches!(action, ObsAction::StartStream | ObsAction::ToggleStream) {
            return self.start_stream(request_type, &data).await;
        }
        let resp = self.send(request_type, &data, false).await?;
        let status = &resp["requestStatus"];
        if status["result"].as_bool().unwrap_or(false) {
            Ok(Outcome::Ok)
        } else {
            Ok(alert(refusal(request_type, &data, status)))
        }
    }

    /// `StartStream`, or `ToggleStream`, which may start one. OBS answers at once and
    /// connects to the server afterwards; one that can't connect goes back to stopped
    /// (about 2 s later for a server that refuses) and OBS opens its own "Failed to
    /// connect" dialog. So the request goes on a connection of its own that hears
    /// output events, and the key answers as soon as the stream goes live or stops.
    /// The shared connection stays free for polls and other keys meanwhile. In several
    /// steps, the steps after it wait for that answer too, as they do for a command: a
    /// "go live" key switches scene once the stream is live, and still warns if it isn't.
    async fn start_stream(&self, request_type: &str, data: &Value) -> Result<Outcome, ActionError> {
        let mut ws = self.connect_noted(OUTPUT_EVENTS).await?;
        let mut stream = StreamStart::default();
        let heard = |msg: &Value| stream.heard(msg);
        let resp = Self::request(
            &mut ws,
            self.timeout,
            &self.next_id(),
            request_type,
            data,
            heard,
        )
        .await
        .map_err(|failure| match failure {
            Failure::NotSent(e) => err(e),
            Failure::MaybeSent(e) => err(e.maybe_happened()),
        })?;
        let status = &resp["requestStatus"];
        if !status["result"].as_bool().unwrap_or(false) {
            return Ok(alert(refusal(request_type, data, status)));
        }
        // ToggleStream answers with the stream's new state; off means it's stopping.
        if resp["responseData"]["outputActive"] == false {
            return Ok(Outcome::Ok);
        }
        let until = Instant::now() + self.stream_wait;
        let mut ask_at = Instant::now() + STREAM_ASK_EVERY;
        while stream.went_live.is_none() {
            let now = Instant::now();
            if now >= until {
                break;
            }
            // Whether it's active, for a Stream Delay. The answer comes in with the
            // events, so none of them is missed while waiting for it.
            if now >= ask_at {
                let ask = request_message(&self.next_id(), "GetStreamStatus", &json!({}));
                if ws.send(ask).await.is_err() {
                    break;
                }
                ask_at = now + STREAM_ASK_EVERY;
            }
            let wait = ask_at.min(until).saturating_duration_since(Instant::now());
            match next_json(&mut ws, wait).await {
                Ok(msg) => stream.heard(&msg),
                Err(ReadError::TimedOut) => {}
                // OBS closed the connection: no word either way.
                Err(_) => break,
            }
        }
        let _ = ws.close(None).await;
        Ok(match stream.went_live {
            Some(true) => Outcome::Ok,
            Some(false) => alert(
                "the stream didn't start; see the message OBS shows, or check Settings → \
                 Stream in OBS",
            ),
            None => alert(
                "the stream hasn't started yet; if it can't connect, OBS shows a message \
                 saying why",
            ),
        })
    }

    async fn send(
        &self,
        request_type: &str,
        data: &Value,
        poll: bool,
    ) -> Result<Value, ActionError> {
        let id = self.next_id();
        // Reuse the connection. If it turns out to have closed, reconnect once and send
        // again, unless OBS may already have the request and it isn't repeatable.
        for attempt in 0..2 {
            let mut guard = self.connection(poll).await?;
            let ws = guard.as_mut().expect("connected");
            match Self::request(ws, self.timeout, &id, request_type, data, |_| {}).await {
                Ok(resp) => return Ok(resp),
                Err(failure) => {
                    *guard = None;
                    match failure {
                        Failure::NotSent(e) if attempt == 0 => {
                            debug!("OBS connection lost ({e}); reconnecting");
                        }
                        Failure::NotSent(e) => return Err(err(e)),
                        Failure::MaybeSent(e) if attempt == 0 && repeatable(request_type) => {
                            debug!("OBS connection lost ({}); reconnecting", e.describe());
                        }
                        Failure::MaybeSent(e) if repeatable(request_type) => {
                            return Err(err(e.describe()))
                        }
                        Failure::MaybeSent(e) => {
                            debug!("no answer to {request_type}: {}", e.describe());
                            return Err(err(e.maybe_happened()));
                        }
                    }
                }
            }
        }
        unreachable!("loop returns on the second attempt")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};

    impl ObsClient {
        fn with_timeout(mut self, wait: Duration) -> Self {
            self.timeout = wait;
            self
        }

        fn with_stream_wait(mut self, wait: Duration) -> Self {
            self.stream_wait = wait;
            self
        }
    }

    fn client(url: &str, password: Option<&str>) -> ObsClient {
        ObsClient::new(ObsConfig {
            url: url.to_owned(),
            password: password.map(str::to_owned),
        })
    }

    #[test]
    fn auth_matches_the_obs_websocket_reference() {
        // Worked example from the obs-websocket protocol docs.
        assert_eq!(
            auth_string(
                "supersecretpassword",
                "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=",
                "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY="
            ),
            "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4="
        );
    }

    #[test]
    fn actions_map_to_requests() {
        assert_eq!(
            request_for(&ObsAction::Scene {
                scene: "BRB".into()
            }),
            ("SetCurrentProgramScene", json!({ "sceneName": "BRB" }))
        );
        assert_eq!(
            request_for(&ObsAction::ToggleMute {
                input: "Mic".into()
            })
            .0,
            "ToggleInputMute"
        );
    }

    #[test]
    fn refusals_without_a_comment_are_explained() {
        let says = |request: &str, data: Value, code: u64| {
            refusal(request, &data, &json!({ "result": false, "code": code }))
        };
        assert_eq!(says("StartRecord", json!({}), 500), "already recording");
        assert_eq!(says("StartStream", json!({}), 500), "already streaming");
        assert_eq!(says("StopRecord", json!({}), 501), "not recording");
        assert_eq!(says("StopStream", json!({}), 501), "not streaming");
        assert_eq!(
            says("SaveReplayBuffer", json!({}), 501),
            "the replay buffer isn't running"
        );
        assert_eq!(
            says("StartVirtualCam", json!({}), 500),
            "it's already running"
        );
        assert_eq!(
            says("PauseRecord", json!({}), 502),
            "the recording is already paused"
        );
        assert_eq!(
            says("ToggleRecord", json!({}), 503),
            "the recording isn't paused"
        );
        assert_eq!(
            says("TriggerStudioModeTransition", json!({}), 506),
            "Studio Mode is off"
        );
        assert_eq!(
            says("SetCurrentProgramScene", json!({ "sceneName": "BRB" }), 600),
            "there's no scene called “BRB”"
        );
        assert_eq!(
            says("ToggleInputMute", json!({ "inputName": "Mic" }), 600),
            "there's no source called “Mic”"
        );
        assert_eq!(
            says("ToggleRecord", json!({}), 604),
            "it can't do that right now"
        );
        assert_eq!(
            says("SplitRecordFile", json!({}), 204),
            "this version of OBS can't do that; update OBS"
        );
        assert_eq!(
            says("ToggleRecord", json!({}), 702),
            "it refused ToggleRecord (error 702)"
        );
        // A replay buffer turned off in OBS: where to turn it on, even over OBS's comment.
        let off =
            json!({ "result": false, "code": 604, "comment": "Replay buffer is not available." });
        assert_eq!(
            refusal("ToggleReplayBuffer", &json!({}), &off),
            "the replay buffer is turned off (Settings → Output → Replay Buffer)"
        );
        assert_eq!(
            says("SaveReplayBuffer", json!({}), 504),
            "the replay buffer is turned off (Settings → Output → Replay Buffer)"
        );
        // Starting or quitting: to try again, over what OBS says.
        let starting = json!({ "result": false, "code": 207, "comment": "OBS is not ready to perform the request." });
        assert_eq!(
            refusal("SetCurrentProgramScene", &json!({}), &starting),
            "it isn't ready yet; try again in a moment"
        );
        // Otherwise OBS's own comment comes first.
        let comment = json!({ "result": false, "code": 600, "comment": "No source was found by the name of `X`." });
        assert_eq!(
            refusal(
                "SetCurrentProgramScene",
                &json!({ "sceneName": "X" }),
                &comment
            ),
            "No source was found by the name of `X`."
        );
    }

    #[test]
    fn only_requests_that_do_the_same_twice_are_sent_again() {
        for r in [
            "GetRecordStatus",
            "SetCurrentProgramScene",
            "SetInputVolume",
        ] {
            assert!(repeatable(r), "{r}");
        }
        for r in [
            "ToggleRecord",
            "ToggleStream",
            "ToggleInputMute",
            "ToggleReplayBuffer",
            "StartRecord",
            "SaveReplayBuffer",
        ] {
            assert!(!repeatable(r), "{r}");
        }
    }

    #[test]
    fn polls_back_off_after_failures_and_start_over_after_a_connection() {
        let t0 = Instant::now();
        let mut b = Backoff::default();
        assert_eq!(b.waiting(t0), None);
        b.failed(t0, t0, "can't reach OBS".into());
        assert_eq!(
            b.waiting(t0 + Duration::from_secs(4)).as_deref(),
            Some("can't reach OBS")
        );
        assert_eq!(b.waiting(t0 + Duration::from_secs(5)), None);
        // 5 s, 10 s, 20 s, then 30 s at most.
        let mut at = t0;
        for wait in [10, 20, 30, 30] {
            at += Duration::from_secs(60);
            b.failed(at, at, "still closed".into());
            assert!(b.waiting(at + Duration::from_secs(wait - 1)).is_some());
            assert!(b.waiting(at + Duration::from_secs(wait)).is_none());
        }
        assert!(b.failed_since(at).is_some());
        assert!(b.failed_since(at + Duration::from_millis(1)).is_none());
        b.succeeded();
        assert_eq!(b.waiting(at), None);
        b.failed(at, at, "closed again".into());
        assert!(
            b.waiting(at + Duration::from_secs(5)).is_none(),
            "back to 5 s"
        );
        // An attempt that began before a press, and failed while it waited, isn't the
        // press's answer: it may have begun before OBS was listening.
        let press = at + Duration::from_secs(1);
        b.failed(at, press + Duration::from_secs(1), "can't reach OBS".into());
        assert!(b.failed_since(press).is_none());
        assert!(b.failed_since(at).is_some());
    }

    /// How the fake OBS behaves.
    #[derive(Clone, Copy, Default)]
    struct Behaviour {
        password: Option<&'static str>,
        /// Close the connection on the first request instead of answering it, as when
        /// OBS quits mid-request.
        drop_first_request: bool,
        /// The close code and reason for that, if any.
        drop_with: Option<(u16, &'static str)>,
        /// Read requests but never answer them, as an OBS that's busy or stuck.
        unanswered: bool,
        /// Close the connection after each answer, as when OBS restarts between presses.
        close_after_answer: bool,
        /// Never greet, as an OBS that's hanging.
        silent: bool,
        /// Refuse every request with 207, as OBS does while it starts or quits.
        not_ready: bool,
        stream_server: StreamServer,
        /// Say STARTING before answering a stream start, not after. obs-websocket
        /// answers on one thread and sends events from another, so either may come first.
        starting_first: bool,
    }

    /// The server a stream goes to. OBS answers the start and says STARTING at once;
    /// what follows a moment later (`STREAM_CONNECTS`) depends on the server.
    #[derive(Clone, Copy, Default)]
    enum StreamServer {
        /// It takes the stream: STARTED.
        #[default]
        TakesIt,
        /// Nothing listens there: STOPPED. Real OBS takes about 2 s and opens its own
        /// "Failed to connect" dialog.
        Refuses,
        /// It never answers: no more events for a while.
        NeverAnswers,
        /// OBS has a Stream Delay set: the stream is active at once (GetStreamStatus
        /// says so), and STARTED comes when the delay is over, 20 s by default, so not
        /// within a test.
        Delayed,
    }

    /// How long the fake's stream server takes to take a stream or refuse it.
    const STREAM_CONNECTS: Duration = Duration::from_millis(100);

    /// StreamStateChanged, as obs-websocket sends it.
    fn stream_event(state: &str, active: bool) -> WsMessage {
        let event = json!({ "op": 5, "d": {
            "eventType": "StreamStateChanged", "eventIntent": OUTPUT_EVENTS,
            "eventData": { "outputActive": active, "outputState": state },
        }});
        WsMessage::text(event.to_string())
    }

    struct Fake {
        url: String,
        requests: tokio::sync::mpsc::UnboundedReceiver<Value>,
        connections: Arc<AtomicUsize>,
    }

    /// A tiny obs-websocket server that records requests and answers success, except
    /// for a scene called "Missing" (600, with OBS's comment), StartRecord (500, no
    /// comment), SaveReplayBuffer (604, replay buffer turned off) and StartStream while
    /// streaming (500, no comment). Starting a stream goes as `stream_server` says.
    async fn fake_obs(behaviour: Behaviour) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (tx, requests) = tokio::sync::mpsc::unbounded_channel();
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Whether the stream is live: OBS's, so the same on every connection.
        let live = Arc::new(std::sync::atomic::AtomicBool::new(false));
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                let (tx, dropped, live) = (tx.clone(), dropped.clone(), live.clone());
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    if behaviour.silent {
                        let _ = ws.next().await;
                        return;
                    }
                    // Whether this session asked for output events.
                    let mut outputs = false;
                    let password = behaviour.password;
                    let (salt, challenge) = ("c2FsdA==", "Y2hhbGxlbmdl");
                    let mut hello = json!({ "op": 0, "d": { "obsWebSocketVersion": "5.5.0", "rpcVersion": 1 } });
                    if password.is_some() {
                        hello["d"]["authentication"] =
                            json!({ "salt": salt, "challenge": challenge });
                    }
                    ws.send(WsMessage::text(hello.to_string())).await.unwrap();
                    while let Some(Ok(WsMessage::Text(t))) = ws.next().await {
                        let msg: Value = serde_json::from_str(t.as_str()).unwrap();
                        match msg["op"].as_u64() {
                            Some(1) => {
                                if let Some(pw) = password {
                                    if msg["d"]["authentication"]
                                        != auth_string(pw, salt, challenge)
                                    {
                                        // What obs-websocket sends.
                                        let _ = ws
                                            .close(Some(CloseFrame {
                                                code: CloseCode::from(CLOSE_AUTH_FAILED),
                                                reason: "Authentication failed.".into(),
                                            }))
                                            .await;
                                        return;
                                    }
                                }
                                let events = msg["d"]["eventSubscriptions"].as_u64();
                                outputs = events.unwrap_or(0) & OUTPUT_EVENTS != 0;
                                ws.send(WsMessage::text(
                                    json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}).to_string(),
                                ))
                                .await
                                .unwrap();
                            }
                            Some(6) => {
                                let d = &msg["d"];
                                let _ = tx.send(d.clone());
                                if behaviour.drop_first_request
                                    && !dropped.swap(true, Ordering::SeqCst)
                                {
                                    let frame = behaviour.drop_with.map(|(code, why)| CloseFrame {
                                        code: CloseCode::from(code),
                                        reason: why.into(),
                                    });
                                    let _ = ws.close(frame).await;
                                    return;
                                }
                                if behaviour.unanswered {
                                    continue;
                                }
                                let request_type = d["requestType"].as_str();
                                let streaming = live.load(Ordering::SeqCst);
                                let (code, comment) = match request_type {
                                    _ if behaviour.not_ready => {
                                        (207, json!("OBS is not ready to perform the request."))
                                    }
                                    _ if d["requestData"]["sceneName"] == "Missing" => (
                                        600,
                                        json!("No source was found by the name of `Missing`."),
                                    ),
                                    Some("StartRecord") => (500, Value::Null),
                                    Some("StartStream") if streaming => (500, Value::Null),
                                    Some("SaveReplayBuffer") => {
                                        (604, json!("Replay buffer is not available."))
                                    }
                                    _ => (100, Value::Null),
                                };
                                let mut resp = json!({ "op": 7, "d": {
                                    "requestType": d["requestType"], "requestId": d["requestId"],
                                    "requestStatus": { "result": code == 100, "code": code, "comment": comment },
                                    "responseData": { "outputActive": true },
                                }});
                                // ToggleStream answers with the stream's new state, and
                                // StartStream with nothing.
                                match request_type {
                                    Some("ToggleStream") => {
                                        resp["d"]["responseData"]["outputActive"] =
                                            json!(!streaming)
                                    }
                                    Some("StartStream") => {
                                        resp["d"].as_object_mut().unwrap().remove("responseData");
                                    }
                                    // Not while still connecting: the pass's polls of real
                                    // OBS never saw it active between STARTING and STOPPED.
                                    Some("GetStreamStatus") => {
                                        resp["d"]["responseData"]["outputActive"] = json!(streaming)
                                    }
                                    _ => {}
                                }
                                let toggle = request_type == Some("ToggleStream");
                                let starts = code == 100
                                    && (request_type == Some("StartStream")
                                        || toggle && !streaming);
                                if starts && outputs && behaviour.starting_first {
                                    let _ = ws.send(stream_event(OUTPUT_STARTING, false)).await;
                                }
                                ws.send(WsMessage::text(resp.to_string())).await.unwrap();
                                if code == 100 && toggle && streaming {
                                    live.store(false, Ordering::SeqCst);
                                    // The key may have hung up by now; events needn't land.
                                    if outputs {
                                        for state in
                                            ["OBS_WEBSOCKET_OUTPUT_STOPPING", OUTPUT_STOPPED]
                                        {
                                            let _ = ws.send(stream_event(state, false)).await;
                                        }
                                    }
                                } else if starts {
                                    if outputs && !behaviour.starting_first {
                                        let starting = stream_event(OUTPUT_STARTING, false);
                                        let _ = ws.send(starting).await;
                                    }
                                    let ends = match behaviour.stream_server {
                                        StreamServer::TakesIt => Some((OUTPUT_STARTED, true)),
                                        StreamServer::Refuses => Some((OUTPUT_STOPPED, false)),
                                        StreamServer::NeverAnswers => None,
                                        StreamServer::Delayed => {
                                            live.store(true, Ordering::SeqCst);
                                            None
                                        }
                                    };
                                    if let Some((state, active)) = ends {
                                        // OBS answers other requests meanwhile; this fake
                                        // doesn't, which only the waiting key would notice.
                                        tokio::time::sleep(STREAM_CONNECTS).await;
                                        live.store(active, Ordering::SeqCst);
                                        if outputs {
                                            let _ = ws.send(stream_event(state, active)).await;
                                        }
                                    }
                                }
                                if behaviour.close_after_answer {
                                    let _ = ws.close(None).await;
                                    // Let the close reach the client before the next press.
                                    let _ = ws.next().await;
                                    return;
                                }
                            }
                            _ => {}
                        }
                    }
                });
            }
        });
        Fake {
            url,
            requests,
            connections,
        }
    }

    impl Fake {
        /// The requests OBS got, but for the GetStreamStatus a starting stream sends now
        /// and then, which a slow test machine may or may not get to.
        fn seen(&mut self) -> Vec<String> {
            let mut out = Vec::new();
            while let Ok(r) = self.requests.try_recv() {
                let request_type = r["requestType"].as_str().unwrap_or("");
                if request_type != "GetStreamStatus" {
                    out.push(request_type.to_owned());
                }
            }
            out
        }
    }

    fn didnt_start() -> Outcome {
        Outcome::Alert(
            "OBS: the stream didn't start; see the message OBS shows, or check Settings → \
             Stream in OBS"
                .into(),
        )
    }

    #[tokio::test]
    async fn runs_requests_with_password() {
        let mut fake = fake_obs(Behaviour {
            password: Some("hunter2"),
            ..Default::default()
        })
        .await;
        let obs = client(&fake.url, Some("hunter2"));
        assert_eq!(
            obs.run(&ObsAction::Scene {
                scene: "Gameplay".into()
            })
            .await
            .unwrap(),
            Outcome::Ok
        );
        assert_eq!(
            obs.run(&ObsAction::ToggleRecord).await.unwrap(),
            Outcome::Ok
        );
        let first = fake.requests.recv().await.unwrap();
        assert_eq!(first["requestType"], "SetCurrentProgramScene");
        assert_eq!(first["requestData"]["sceneName"], "Gameplay");
        assert_eq!(
            fake.requests.recv().await.unwrap()["requestType"],
            "ToggleRecord"
        );
        assert_eq!(
            fake.connections.load(Ordering::SeqCst),
            1,
            "one connection, reused"
        );
    }

    #[tokio::test]
    async fn obs_errors_become_alerts() {
        let fake = fake_obs(Behaviour::default()).await;
        let obs = client(&fake.url, None);
        let out = obs
            .run(&ObsAction::Scene {
                scene: "Missing".into(),
            })
            .await
            .unwrap();
        assert!(matches!(out, Outcome::Alert(m) if m.contains("No source")));
        // No comment from OBS: what the code means.
        assert_eq!(
            obs.run(&ObsAction::StartRecord).await.unwrap(),
            Outcome::Alert("OBS: already recording".into())
        );
        assert_eq!(
            obs.run(&ObsAction::SaveReplayBuffer).await.unwrap(),
            Outcome::Alert(
                "OBS: the replay buffer is turned off (Settings → Output → Replay Buffer)".into()
            )
        );
        // Requests from sliders and live values say it once.
        let e = obs
            .call("SetCurrentProgramScene", json!({ "sceneName": "Missing" }))
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(e, "OBS: No source was found by the name of `Missing`.");
    }

    #[tokio::test]
    async fn wrong_or_missing_password_is_explained() {
        let fake = fake_obs(Behaviour {
            password: Some("right"),
            ..Default::default()
        })
        .await;
        let wrong = client(&fake.url, Some("wrong"));
        assert_eq!(
            wrong
                .run(&ObsAction::ToggleStream)
                .await
                .unwrap_err()
                .to_string(),
            "OBS rejected the password in integrations.toml"
        );
        let none = client(&fake.url, None);
        assert_eq!(
            none.run(&ObsAction::ToggleStream)
                .await
                .unwrap_err()
                .to_string(),
            "OBS needs a password; add it to integrations.toml"
        );
    }

    #[tokio::test]
    async fn obs_not_running_is_explained() {
        let obs = client("ws://127.0.0.1:1", None);
        let e = obs
            .run(&ObsAction::ToggleRecord)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            e.starts_with("OBS: can't reach OBS at ws://127.0.0.1:1: "),
            "{e}"
        );
        assert!(e.ends_with(" (is the WebSocket server enabled?)"), "{e}");
    }

    #[tokio::test]
    async fn an_address_obs_cant_have_says_what_it_needs() {
        let fake = fake_obs(Behaviour::default()).await;
        let bare = fake.url.trim_start_matches("ws://").to_owned();
        for url in [bare.as_str(), "localhost:4455", "http://127.0.0.1:4455"] {
            let e = client(url, None)
                .run(&ObsAction::ToggleRecord)
                .await
                .unwrap_err()
                .to_string();
            assert_eq!(
                e,
                format!(
                    "OBS: the address {url} in integrations.toml must start with ws://, like \
                     ws://127.0.0.1:4455"
                )
            );
        }
        let e = client("wss://127.0.0.1:4455", None)
            .run(&ObsAction::ToggleRecord)
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "OBS's WebSocket server uses ws://, so the address in integrations.toml must start \
             with ws://, not wss://"
        );
        assert_eq!(fake.connections.load(Ordering::SeqCst), 0, "not even tried");
    }

    #[test]
    fn only_nothing_answering_asks_about_the_websocket_server() {
        let io = |kind| WsError::Io(std::io::Error::from(kind));
        let hint = " (is the WebSocket server enabled?)";
        assert_eq!(
            connect_hint(&io(std::io::ErrorKind::ConnectionRefused)),
            hint
        );
        assert_eq!(connect_hint(&io(std::io::ErrorKind::TimedOut)), hint);
        // Such as a name that doesn't resolve, which says so itself.
        assert_eq!(connect_hint(&io(std::io::ErrorKind::Other)), "");
    }

    #[tokio::test]
    async fn something_else_on_the_port_says_what_it_answered() {
        // Something that isn't OBS on the port: a web server answering 404.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).await.unwrap();
                request.extend_from_slice(&buf[..n]);
            }
            stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            let _ = stream.read(&mut buf).await;
        });
        let e = client(&url, None)
            .run(&ObsAction::ToggleRecord)
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            format!("OBS: can't reach OBS at {url}: HTTP error: 404 Not Found")
        );
    }

    #[tokio::test]
    async fn obs_not_ready_says_to_try_again() {
        let fake = fake_obs(Behaviour {
            not_ready: true,
            ..Default::default()
        })
        .await;
        let obs = client(&fake.url, None);
        assert_eq!(
            obs.run(&ObsAction::Scene {
                scene: "Gameplay".into()
            })
            .await
            .unwrap(),
            Outcome::Alert("OBS: it isn't ready yet; try again in a moment".into())
        );
        let e = obs
            .call("SetInputVolume", json!({ "inputName": "Mic" }))
            .await
            .unwrap_err();
        assert_eq!(
            e.to_string(),
            "OBS: it isn't ready yet; try again in a moment"
        );
    }

    #[test]
    fn only_a_stopped_after_this_start_says_it_failed() {
        let event = |kind: &str, state: &str| json!({ "op": 5, "d": { "eventType": kind, "eventData": { "outputState": state } } });
        let stream = |state| event("StreamStateChanged", state);
        let mut start = StreamStart::default();
        // An earlier stream ending, then a recording stopping.
        start.heard(&stream(OUTPUT_STOPPED));
        start.heard(&stream(OUTPUT_STARTING));
        start.heard(&event("RecordStateChanged", OUTPUT_STOPPED));
        assert_eq!(start.went_live, None);
        start.heard(&stream(OUTPUT_STOPPED));
        assert_eq!(start.went_live, Some(false));

        let mut start = StreamStart::default();
        start.heard(&stream(OUTPUT_STARTING));
        start.heard(&stream(OUTPUT_STARTED));
        assert_eq!(start.went_live, Some(true));
    }

    /// OBS takes a stream start at once, then can't connect: it goes back to stopped
    /// and opens its own "Failed to connect" dialog. The key says so, not ✓.
    #[tokio::test]
    async fn a_stream_that_cant_connect_says_it_didnt_start() {
        let mut fake = fake_obs(Behaviour {
            stream_server: StreamServer::Refuses,
            ..Default::default()
        })
        .await;
        let obs = client(&fake.url, None);
        assert_eq!(
            obs.run(&ObsAction::StartStream).await.unwrap(),
            didnt_start()
        );
        assert_eq!(
            obs.run(&ObsAction::ToggleStream).await.unwrap(),
            didnt_start()
        );
        assert_eq!(
            fake.seen(),
            ["StartStream", "ToggleStream"],
            "each sent once"
        );
    }

    /// obs-websocket answers a request on one thread and sends events from another, so
    /// STARTING can come before the answer. It still counts: the STOPPED after it says
    /// the stream didn't start.
    #[tokio::test]
    async fn starting_before_the_answer_still_counts() {
        let fake = fake_obs(Behaviour {
            stream_server: StreamServer::Refuses,
            starting_first: true,
            ..Default::default()
        })
        .await;
        let obs = client(&fake.url, None).with_stream_wait(Duration::from_secs(1));
        assert_eq!(
            obs.run(&ObsAction::StartStream).await.unwrap(),
            didnt_start()
        );
        assert_eq!(
            obs.run(&ObsAction::ToggleStream).await.unwrap(),
            didnt_start()
        );
    }

    /// With a Stream Delay set in OBS, STARTED comes only when the delay is over, but
    /// the stream is active at once. The key answers then, not with a warning at the
    /// end of the wait.
    #[tokio::test]
    async fn a_stream_with_obs_stream_delay_answers_once_its_active() {
        for action in [ObsAction::StartStream, ObsAction::ToggleStream] {
            let fake = fake_obs(Behaviour {
                stream_server: StreamServer::Delayed,
                ..Default::default()
            })
            .await;
            let obs = client(&fake.url, None);
            let started = Instant::now();
            assert_eq!(obs.run(&action).await.unwrap(), Outcome::Ok);
            assert!(started.elapsed() < STREAM_START_WAIT / 3, "{action:?}");
        }
    }

    /// Pressed as OBS comes back, a stream key lets the polls in again as other keys
    /// do, so live values don't wait out the rest of a 30 s backoff.
    #[tokio::test]
    async fn a_stream_press_after_obs_came_back_lets_the_polls_in() {
        for action in [ObsAction::StartStream, ObsAction::ToggleStream] {
            let fake = fake_obs(Behaviour::default()).await;
            let obs = client(&fake.url, None);
            // The polls failed while OBS was closed, and are waiting to try again.
            let now = Instant::now();
            obs.backoff
                .lock()
                .unwrap()
                .failed(now, now, "can't reach OBS (closed)".into());
            assert!(obs.call("GetStreamStatus", json!({})).await.is_err());
            assert_eq!(obs.run(&action).await.unwrap(), Outcome::Ok, "{action:?}");
            let status = obs.call("GetStreamStatus", json!({})).await.unwrap();
            assert_eq!(status["outputActive"], true, "{action:?}");
        }
    }

    /// And one that can't connect makes the polls wait, as other keys do.
    #[tokio::test]
    async fn a_stream_press_that_cant_connect_makes_the_polls_wait() {
        let fake = fake_obs(Behaviour {
            password: Some("right"),
            ..Default::default()
        })
        .await;
        let obs = client(&fake.url, Some("wrong"));
        assert!(obs.run(&ObsAction::StartStream).await.is_err());
        let e = obs.call("GetStreamStatus", json!({})).await.unwrap_err();
        assert!(e.to_string().contains("password"), "{e}");
        assert_eq!(
            fake.connections.load(Ordering::SeqCst),
            1,
            "the poll waited"
        );
    }

    #[tokio::test]
    async fn a_stream_that_goes_live_answers_as_soon_as_it_does() {
        let mut fake = fake_obs(Behaviour::default()).await;
        let obs = client(&fake.url, None);
        let started = Instant::now();
        assert_eq!(obs.run(&ObsAction::StartStream).await.unwrap(), Outcome::Ok);
        assert!(
            started.elapsed() < STREAM_START_WAIT / 3,
            "answered when it went live, not at the end of the wait"
        );
        assert_eq!(
            obs.run(&ObsAction::StartStream).await.unwrap(),
            Outcome::Alert("OBS: already streaming".into())
        );
        // Toggling stops it, without waiting for anything.
        let stopping = Instant::now();
        assert_eq!(
            obs.run(&ObsAction::ToggleStream).await.unwrap(),
            Outcome::Ok
        );
        assert!(stopping.elapsed() < STREAM_START_WAIT / 3);
        // And starts it again.
        assert_eq!(
            obs.run(&ObsAction::ToggleStream).await.unwrap(),
            Outcome::Ok
        );
        assert_eq!(
            fake.seen(),
            ["StartStream", "StartStream", "ToggleStream", "ToggleStream"]
        );
    }

    #[tokio::test]
    async fn a_stream_still_connecting_says_so_and_holds_up_nothing() {
        let fake = fake_obs(Behaviour {
            stream_server: StreamServer::NeverAnswers,
            ..Default::default()
        })
        .await;
        let obs = Arc::new(client(&fake.url, None).with_stream_wait(Duration::from_secs(1)));
        let starting = tokio::spawn({
            let obs = obs.clone();
            async move { obs.run(&ObsAction::StartStream).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Other keys and polls go on meanwhile.
        let meanwhile = Instant::now();
        assert_eq!(
            obs.run(&ObsAction::Scene {
                scene: "Gameplay".into()
            })
            .await
            .unwrap(),
            Outcome::Ok
        );
        assert!(obs.call("GetStreamStatus", json!({})).await.is_ok());
        assert!(meanwhile.elapsed() < Duration::from_millis(500));
        assert_eq!(
            starting.await.unwrap().unwrap(),
            Outcome::Alert(
                "OBS: the stream hasn't started yet; if it can't connect, OBS shows a message \
                 saying why"
                    .into()
            )
        );
    }

    #[tokio::test]
    async fn polls_wait_after_a_failed_connection_but_presses_try() {
        let fake = fake_obs(Behaviour {
            password: Some("right"),
            ..Default::default()
        })
        .await;
        let obs = client(&fake.url, Some("wrong"));
        for _ in 0..3 {
            let e = obs.call("GetRecordStatus", json!({})).await.unwrap_err();
            assert!(e.to_string().contains("password"), "{e}");
        }
        assert_eq!(fake.connections.load(Ordering::SeqCst), 1, "polls back off");
        assert!(obs.run(&ObsAction::ToggleRecord).await.is_err());
        assert_eq!(
            fake.connections.load(Ordering::SeqCst),
            2,
            "a press tries again"
        );
    }

    #[tokio::test]
    async fn polls_dont_wait_for_someone_elses_connection() {
        let fake = fake_obs(Behaviour {
            silent: true,
            ..Default::default()
        })
        .await;
        let obs = Arc::new(client(&fake.url, None).with_timeout(Duration::from_secs(2)));
        let pressing = tokio::spawn({
            let obs = obs.clone();
            async move { obs.run(&ObsAction::ToggleRecord).await }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        let started = Instant::now();
        let e = obs.call("GetRecordStatus", json!({})).await.unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(500), "{e}");
        assert!(e.to_string().contains("still connecting"), "{e}");
        assert!(pressing
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("didn't answer"));
    }

    #[tokio::test]
    async fn reconnects_when_obs_restarted_between_presses() {
        let mut fake = fake_obs(Behaviour {
            close_after_answer: true,
            ..Default::default()
        })
        .await;
        let obs = client(&fake.url, None);
        for _ in 0..3 {
            assert_eq!(
                obs.run(&ObsAction::ToggleRecord).await.unwrap(),
                Outcome::Ok
            );
            // Give the close time to arrive, as it has by the next real press.
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(fake.seen(), ["ToggleRecord"; 3], "each sent exactly once");
        assert_eq!(fake.connections.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_toggle_obs_may_have_received_is_not_sent_again() {
        let mut fake = fake_obs(Behaviour {
            drop_first_request: true,
            ..Default::default()
        })
        .await;
        let obs = client(&fake.url, None);
        let e = obs
            .run(&ObsAction::ToggleRecord)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("may or may not have happened"), "{e}");
        assert_eq!(fake.seen(), ["ToggleRecord"]);

        // A status request is safe to repeat, so it is.
        let mut fake = fake_obs(Behaviour {
            drop_first_request: true,
            ..Default::default()
        })
        .await;
        let obs = client(&fake.url, None);
        let status = obs.call("GetRecordStatus", json!({})).await.unwrap();
        assert_eq!(status["outputActive"], true);
        assert_eq!(fake.seen(), ["GetRecordStatus"; 2]);
    }

    #[tokio::test]
    async fn obs_quitting_mid_toggle_says_it_may_or_may_not_have_happened() {
        // What obs-websocket sends when OBS quits; its reason ends in a full stop.
        let mut fake = fake_obs(Behaviour {
            drop_first_request: true,
            drop_with: Some((1001, "Server stopping.")),
            ..Default::default()
        })
        .await;
        let obs = client(&fake.url, None);
        assert_eq!(
            obs.run(&ObsAction::ToggleStream)
                .await
                .unwrap_err()
                .to_string(),
            "OBS closed the connection before answering, so it may or may not have happened"
        );
        assert_eq!(fake.seen(), ["ToggleStream"]);
    }

    #[tokio::test]
    async fn a_toggle_obs_never_answers_says_it_may_or_may_not_have_happened() {
        let mut fake = fake_obs(Behaviour {
            unanswered: true,
            ..Default::default()
        })
        .await;
        let obs = client(&fake.url, None).with_timeout(Duration::from_millis(300));
        assert_eq!(
            obs.run(&ObsAction::ToggleRecord)
                .await
                .unwrap_err()
                .to_string(),
            "OBS didn't answer in time, so it may or may not have happened"
        );
        assert_eq!(fake.seen(), ["ToggleRecord"], "sent once");
    }

    #[tokio::test]
    async fn a_press_tries_itself_after_waiting_for_an_earlier_attempt() {
        // A poll starts connecting; OBS never greets, so it fails after the timeout.
        // A press that came in meanwhile waited for it, but must still try itself: by
        // now OBS may be listening.
        let fake = fake_obs(Behaviour {
            silent: true,
            ..Default::default()
        })
        .await;
        let obs = Arc::new(client(&fake.url, None).with_timeout(Duration::from_millis(400)));
        let polling = tokio::spawn({
            let obs = obs.clone();
            async move { obs.call("GetRecordStatus", json!({})).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(obs.run(&ObsAction::ToggleRecord).await.is_err());
        assert!(polling.await.unwrap().is_err());
        assert_eq!(
            fake.connections.load(Ordering::SeqCst),
            2,
            "the press made its own attempt"
        );
    }
}
