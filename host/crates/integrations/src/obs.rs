//! OBS Studio via obs-websocket 5 (built into OBS 28+).
//!
//! One connection is opened on first use and reused; if it drops, the next request
//! reconnects. Live values and sliders poll with `Get…` requests; while OBS can't be
//! reached they wait between attempts (5 s, doubling up to 30 s) instead of trying on
//! every poll, and never wait for someone else's attempt. Key presses and slider moves
//! always try. A request OBS may already have received is sent again only if doing it
//! twice changes nothing (`repeatable`).
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
    connect_async, tungstenite::Message as WsMessage, MaybeTlsStream, WebSocketStream,
};
use tracing::debug;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const RPC_VERSION: u64 = 1;
const TIMEOUT: Duration = Duration::from_secs(5);
/// How long polls wait after a failed connection attempt; doubles each time.
const BACKOFF_FIRST: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

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
    next_id: std::sync::atomic::AtomicU64,
}

fn err(reason: impl Into<String>) -> ActionError {
    ActionError::Integration {
        service: "OBS".into(),
        reason: reason.into(),
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
        (NOT_READY, _) => "it isn't ready yet; try again in a moment".into(),
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
            next_id: Default::default(),
        }
    }

    async fn connect(&self) -> Result<Ws, ActionError> {
        let (mut ws, _) = timeout(self.timeout, connect_async(&self.config.url))
            .await
            .map_err(|_| {
                err(format!(
                    "can't reach OBS at {} (is OBS open?)",
                    self.config.url
                ))
            })?
            .map_err(|e| {
                err(format!(
                    "can't reach OBS at {}: {e} (is the WebSocket server enabled?)",
                    self.config.url
                ))
            })?;
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
            json!({ "op": 1, "d": { "rpcVersion": RPC_VERSION, "eventSubscriptions": 0 } });
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
        let attempt = Instant::now();
        match self.connect().await {
            Ok(ws) => {
                backoff().succeeded();
                let mut guard = self.conn.lock().await;
                *guard = Some(ws);
                Ok(guard)
            }
            Err(e) => {
                let reason = match &e {
                    ActionError::Integration { reason, .. } => reason.clone(),
                    other => other.to_string(),
                };
                backoff().failed(attempt, Instant::now(), reason);
                Err(e)
            }
        }
    }

    async fn request(
        ws: &mut Ws,
        wait: Duration,
        id: &str,
        request_type: &str,
        data: &Value,
    ) -> Result<Value, Failure> {
        if closed_already(ws) {
            return Err(Failure::NotSent("OBS closed the connection".into()));
        }
        let req = json!({ "op": 6, "d": { "requestType": request_type, "requestId": id, "requestData": data } });
        ws.send(WsMessage::text(req.to_string()))
            .await
            .map_err(|e| Failure::NotSent(e.to_string()))?;
        loop {
            let msg = next_json(ws, wait).await.map_err(Failure::MaybeSent)?;
            if msg["op"] == 7 && msg["d"]["requestId"] == id {
                return Ok(msg["d"].clone());
            }
        }
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
        let resp = self.send(request_type, &data, false).await?;
        let status = &resp["requestStatus"];
        if status["result"].as_bool().unwrap_or(false) {
            Ok(Outcome::Ok)
        } else {
            Ok(Outcome::Alert(format!(
                "OBS: {}",
                refusal(request_type, &data, status)
            )))
        }
    }

    async fn send(
        &self,
        request_type: &str,
        data: &Value,
        poll: bool,
    ) -> Result<Value, ActionError> {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .to_string();
        // Reuse the connection. If it turns out to have closed, reconnect once and send
        // again, unless OBS may already have the request and it isn't repeatable.
        for attempt in 0..2 {
            let mut guard = self.connection(poll).await?;
            let ws = guard.as_mut().expect("connected");
            match Self::request(ws, self.timeout, &id, request_type, data).await {
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
    }

    struct Fake {
        url: String,
        requests: tokio::sync::mpsc::UnboundedReceiver<Value>,
        connections: Arc<AtomicUsize>,
    }

    /// A tiny obs-websocket server that records requests and answers success, except
    /// for a scene called "Missing" (600, with OBS's comment), StartRecord (500, no
    /// comment) and SaveReplayBuffer (604, replay buffer turned off).
    async fn fake_obs(behaviour: Behaviour) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (tx, requests) = tokio::sync::mpsc::unbounded_channel();
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                let (tx, dropped) = (tx.clone(), dropped.clone());
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    if behaviour.silent {
                        let _ = ws.next().await;
                        return;
                    }
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
                                let (code, comment) = match d["requestType"].as_str() {
                                    _ if d["requestData"]["sceneName"] == "Missing" => (
                                        600,
                                        json!("No source was found by the name of `Missing`."),
                                    ),
                                    Some("StartRecord") => (500, Value::Null),
                                    Some("SaveReplayBuffer") => {
                                        (604, json!("Replay buffer is not available."))
                                    }
                                    _ => (100, Value::Null),
                                };
                                let resp = json!({ "op": 7, "d": {
                                    "requestType": d["requestType"], "requestId": d["requestId"],
                                    "requestStatus": { "result": code == 100, "code": code, "comment": comment },
                                    "responseData": { "outputActive": true },
                                }});
                                ws.send(WsMessage::text(resp.to_string())).await.unwrap();
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
        fn seen(&mut self) -> Vec<String> {
            let mut out = Vec::new();
            while let Ok(r) = self.requests.try_recv() {
                out.push(r["requestType"].as_str().unwrap_or("").to_owned());
            }
            out
        }
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
            "OBS: OBS rejected the password in integrations.toml"
        );
        let none = client(&fake.url, None);
        assert!(none
            .run(&ObsAction::ToggleStream)
            .await
            .unwrap_err()
            .to_string()
            .contains("password"));
    }

    #[tokio::test]
    async fn obs_not_running_is_explained() {
        let obs = client("ws://127.0.0.1:1", None);
        assert!(obs
            .run(&ObsAction::ToggleRecord)
            .await
            .unwrap_err()
            .to_string()
            .contains("can't reach OBS"));
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
            "OBS: OBS closed the connection before answering, so it may or may not have happened"
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
            "OBS: OBS didn't answer in time, so it may or may not have happened"
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
