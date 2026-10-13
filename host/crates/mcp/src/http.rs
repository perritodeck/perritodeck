//! The Streamable HTTP transport, loopback only: `POST /mcp` with a JSON body, answered
//! with JSON (no SSE). A small HTTP/1.1 server of its own, so the crate needs no web
//! framework.

use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use base64::Engine as _;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{oneshot, Semaphore},
    task::JoinHandle,
    time::timeout,
};
use tracing::{debug, warn};

use crate::{
    engine::{all_versions, Server, Session, META_VERSION},
    jsonrpc::{RpcError, INVALID_REQUEST},
    same_secret, Backend, DEFAULT_MAX_BODY, DEFAULT_PORT, ENDPOINT_PATH,
};

const MAX_HEAD: usize = 16 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_CONNECTIONS: usize = 32;
const MAX_SESSIONS: usize = 64;
const SESSION_IDLE: Duration = Duration::from_secs(24 * 60 * 60);
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const BODY_TIMEOUT: Duration = Duration::from_secs(30);

/// How to run the HTTP endpoint.
#[derive(Debug, Clone)]
pub struct HttpOptions {
    /// Must be a loopback address. Port 0 picks a free one.
    pub addr: SocketAddr,
    /// Clients send `Authorization: Bearer <token>`. Make one with
    /// [`generate_token`](crate::generate_token).
    pub token: String,
    /// Largest request body accepted (413 above it).
    pub max_body: usize,
}

impl HttpOptions {
    /// `127.0.0.1:`[`DEFAULT_PORT`] with this token.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT)),
            token: token.into(),
            max_body: DEFAULT_MAX_BODY,
        }
    }

    pub fn port(mut self, port: u16) -> Self {
        self.addr.set_port(port);
        self
    }
}

/// A running HTTP endpoint. Dropping it stops listening (requests already being
/// answered finish).
pub struct HttpServer {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl HttpServer {
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// The endpoint URL to give assistants, e.g. `http://127.0.0.1:46711/mcp`.
    pub fn url(&self) -> String {
        format!("http://{}{ENDPOINT_PATH}", self.addr)
    }

    /// Stop listening and wait for the listener to close.
    pub async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

struct Shared {
    server: Server,
    backend: Arc<dyn Backend>,
    token: String,
    port: u16,
    max_body: usize,
    sessions: Mutex<HashMap<String, (Session, Instant)>>,
}

/// Start the HTTP endpoint. Fails if `addr` isn't loopback, the token is empty, or the
/// port is taken.
pub async fn serve_http(backend: Arc<dyn Backend>, opts: HttpOptions) -> io::Result<HttpServer> {
    if !opts.addr.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the MCP endpoint only listens on a loopback address",
        ));
    }
    if opts.token.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the MCP endpoint needs an access token",
        ));
    }
    let listener = TcpListener::bind(opts.addr).await?;
    let addr = listener.local_addr()?;
    let shared = Arc::new(Shared {
        server: Server::new(backend.clone()),
        backend,
        token: opts.token,
        port: addr.port(),
        max_body: opts.max_body,
        sessions: Mutex::new(HashMap::new()),
    });
    let (stop, mut stopped) = oneshot::channel();
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let Ok(slot) = slots.clone().try_acquire_owned() else {
                        warn!("too many MCP connections; dropping one");
                        continue;
                    };
                    let shared = shared.clone();
                    tokio::spawn(async move {
                        connection(stream, shared).await;
                        drop(slot);
                    });
                }
            }
        }
    });
    debug!(%addr, "MCP endpoint listening");
    Ok(HttpServer {
        addr,
        stop: Some(stop),
        task: Some(task),
    })
}

// ---- HTTP/1.1 messages ----

/// A request or response head.
#[derive(Debug, Clone)]
pub(crate) struct Head {
    /// The request line or status line.
    pub start: String,
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn count(&self, name: &str) -> usize {
        self.headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .count()
    }
}

/// Read a head; bytes after it stay in `buf`. `None` if the stream ended first.
pub(crate) async fn read_head<S: AsyncRead + Unpin>(
    s: &mut S,
    buf: &mut Vec<u8>,
) -> io::Result<Option<Head>> {
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let text = String::from_utf8_lossy(&buf[..end]).into_owned();
            buf.drain(..end + 4);
            return parse_head(&text).map(Some);
        }
        if buf.len() > MAX_HEAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "head too large"));
        }
        let mut chunk = [0u8; 4096];
        let n = s.read(&mut chunk).await?;
        if n == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn parse_head(text: &str) -> io::Result<Head> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_owned());
    let mut lines = text.split("\r\n");
    let start = lines.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    for line in lines {
        if line.starts_with([' ', '\t']) {
            return Err(bad("folded header"));
        }
        let (name, value) = line.split_once(':').ok_or_else(|| bad("bad header"))?;
        if name.is_empty() || name.contains([' ', '\t']) {
            return Err(bad("bad header name"));
        }
        headers.push((name.to_owned(), value.trim().to_owned()));
        if headers.len() > MAX_HEADERS {
            return Err(bad("too many headers"));
        }
    }
    Ok(Head { start, headers })
}

/// Read exactly `len` body bytes, starting with what's already in `buf`.
pub(crate) async fn read_body<S: AsyncRead + Unpin>(
    s: &mut S,
    buf: &mut Vec<u8>,
    len: usize,
) -> io::Result<Vec<u8>> {
    let have = buf.len().min(len);
    let mut body: Vec<u8> = buf.drain(..have).collect();
    if body.len() < len {
        let mut rest = vec![0u8; len - body.len()];
        s.read_exact(&mut rest).await?;
        body.extend_from_slice(&rest);
    }
    Ok(body)
}

struct Resp {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    close: bool,
}

impl Resp {
    fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            close: false,
        }
    }

    fn json(status: u16, v: &Value) -> Self {
        Self {
            status,
            headers: vec![("Content-Type", "application/json".into())],
            body: v.to_string().into_bytes(),
            close: false,
        }
    }

    /// An error with a JSON-RPC error body that has no id, as the spec allows.
    fn rpc_error(status: u16, code: i64, message: &str) -> Self {
        Self::json(
            status,
            &json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message } }),
        )
    }

    fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    fn closing(mut self) -> Self {
        self.close = true;
        self
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = format!("HTTP/1.1 {} {}\r\n", self.status, reason(self.status));
        for (n, v) in &self.headers {
            out.push_str(&format!("{n}: {v}\r\n"));
        }
        out.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        out.push_str("Cache-Control: no-store\r\n");
        if self.close {
            out.push_str("Connection: close\r\n");
        }
        out.push_str("\r\n");
        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        _ => "Error",
    }
}

// ---- Serving ----

async fn connection(mut stream: TcpStream, shared: Arc<Shared>) {
    let mut buf = Vec::new();
    let mut first = true;
    loop {
        let wait = if first { HEAD_TIMEOUT } else { IDLE_TIMEOUT };
        first = false;
        let head = match timeout(wait, read_head(&mut stream, &mut buf)).await {
            Ok(Ok(Some(h))) => h,
            Ok(Ok(None)) | Err(_) => return,
            Ok(Err(e)) => {
                let status = if e.kind() == io::ErrorKind::InvalidData {
                    431
                } else {
                    400
                };
                let _ = stream
                    .write_all(&Resp::empty(status).closing().bytes())
                    .await;
                linger(&mut stream).await;
                return;
            }
        };
        let resp = handle(&shared, &head, &mut stream, &mut buf).await;
        let close = resp.close || wants_close(&head);
        let resp = if close { resp.closing() } else { resp };
        if stream.write_all(&resp.bytes()).await.is_err() || close {
            linger(&mut stream).await;
            return;
        }
    }
}

/// Close after the client has had the response: stop writing, then read and drop what it
/// still sends for a moment. Closing with unread data (a refused body) would reset the
/// connection and could lose the response.
async fn linger(stream: &mut TcpStream) {
    let _ = stream.shutdown().await;
    let _ = timeout(Duration::from_millis(500), async {
        let mut sink = [0u8; 8192];
        let mut total = 0usize;
        while total < 4 * DEFAULT_MAX_BODY {
            match stream.read(&mut sink).await {
                Ok(0) | Err(_) => break,
                Ok(n) => total += n,
            }
        }
    })
    .await;
}

fn wants_close(head: &Head) -> bool {
    let conn = head
        .header("Connection")
        .unwrap_or_default()
        .to_ascii_lowercase();
    conn.split(',').any(|t| t.trim() == "close")
        || (head.start.ends_with("HTTP/1.0") && !conn.contains("keep-alive"))
}

async fn handle(shared: &Shared, head: &Head, stream: &mut TcpStream, buf: &mut Vec<u8>) -> Resp {
    let mut parts = head.start.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Resp::empty(400).closing();
    };
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0") {
        return Resp::empty(400).closing();
    }
    for single in [
        "Host",
        "Origin",
        "Authorization",
        "Content-Length",
        "Mcp-Session-Id",
    ] {
        if head.count(single) > 1 {
            return Resp::empty(400).closing();
        }
    }

    // DNS rebinding: a web page that got a hostname pointed at 127.0.0.1 sends its own
    // name as Host and its own Origin.
    if !host_ok(head.header("Host"), shared.port) {
        return Resp::rpc_error(403, INVALID_REQUEST, "Forbidden: unexpected Host header")
            .closing();
    }
    if let Some(origin) = head.header("Origin") {
        if !origin_ok(origin) {
            return Resp::rpc_error(
                403,
                INVALID_REQUEST,
                "Forbidden: requests from web pages aren't allowed",
            )
            .closing();
        }
    }
    if target.split('?').next() != Some(ENDPOINT_PATH) {
        return Resp::empty(404).closing();
    }
    let authorized = head
        .header("Authorization")
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .is_some_and(|t| same_secret(t.trim(), &shared.token));
    if !authorized {
        return Resp::rpc_error(401, INVALID_REQUEST, "Unauthorized: send the access token PerritoDeck shows in its settings as \"Authorization: Bearer <token>\"")
            .header("WWW-Authenticate", "Bearer realm=\"PerritoDeck\"")
            .closing();
    }
    if let Some(version) = head
        .header("PerritoDeck-Bridge")
        .and_then(|v| v.trim().parse::<u32>().ok())
    {
        let agent = head.header("User-Agent").unwrap_or_default();
        shared.backend.bridge_seen(agent, version);
    }
    match method {
        "POST" => {}
        "DELETE" => {
            let removed = head.header("Mcp-Session-Id").is_some_and(|id| {
                shared
                    .sessions
                    .lock()
                    .expect("sessions poisoned")
                    .remove(id)
                    .is_some()
            });
            return Resp::empty(if removed { 204 } else { 404 }).closing();
        }
        // No server-to-client stream: GET is 405, as the spec allows.
        _ => return Resp::empty(405).header("Allow", "POST, DELETE").closing(),
    }

    if head.header("Transfer-Encoding").is_some() {
        return Resp::empty(501).closing();
    }
    let Some(len) = head.header("Content-Length") else {
        return Resp::empty(411).closing();
    };
    let Ok(len) = len.parse::<usize>() else {
        return Resp::empty(400).closing();
    };
    if len > shared.max_body {
        return Resp::rpc_error(413, INVALID_REQUEST, "request body too large").closing();
    }
    let body = match timeout(BODY_TIMEOUT, read_body(stream, buf, len)).await {
        Ok(Ok(b)) => b,
        _ => return Resp::empty(400).closing(),
    };
    let json_type = head
        .header("Content-Type")
        .is_some_and(|t| t.to_ascii_lowercase().starts_with("application/json"));
    if !json_type {
        return Resp::rpc_error(
            415,
            INVALID_REQUEST,
            "Content-Type must be application/json",
        );
    }
    if let Some(accept) = head.header("Accept") {
        let a = accept.to_ascii_lowercase();
        if !(a.contains("application/json") || a.contains("*/*") || a.contains("application/*")) {
            return Resp::rpc_error(
                406,
                INVALID_REQUEST,
                "this endpoint answers with application/json",
            );
        }
    }
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let e = RpcError::parse(format!("Parse error: {e}"));
            return Resp::json(400, &e.response(Value::Null));
        }
    };
    post(shared, head, msg).await
}

async fn post(shared: &Shared, head: &Head, msg: Value) -> Resp {
    let meta_version = msg
        .get("params")
        .and_then(|p| p.get("_meta"))
        .and_then(|m| m.get(META_VERSION))
        .cloned();
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let is_initialize = method == "initialize" && msg.get("id").is_some();

    if let (Some(version), false) = (&meta_version, is_initialize) {
        // Stateless request: no session, headers must match the body.
        if let Some(version) = version.as_str() {
            if let Err(e) = check_modern_headers(head, &msg, version) {
                return Resp::json(
                    e.http_status,
                    &e.response(msg.get("id").cloned().unwrap_or(Value::Null)),
                );
            }
        }
        return reply(
            shared.server.handle_value(msg, &mut Session::new()).await,
            None,
        );
    }

    if let Some(v) = head.header("MCP-Protocol-Version") {
        if !all_versions().contains(&v) {
            return Resp::rpc_error(
                400,
                INVALID_REQUEST,
                &format!("Unsupported MCP-Protocol-Version: {v}"),
            );
        }
    }
    if is_initialize {
        let mut session = Session::new();
        let r = shared.server.handle_value(msg, &mut session).await;
        let ok = r
            .as_ref()
            .is_some_and(|r| r.message.get("result").is_some());
        if !ok {
            return reply(r, None);
        }
        let id = crate::generate_token();
        let mut sessions = shared.sessions.lock().expect("sessions poisoned");
        sessions.retain(|_, (_, used)| used.elapsed() < SESSION_IDLE);
        if sessions.len() >= MAX_SESSIONS {
            if let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            {
                sessions.remove(&oldest);
            }
        }
        sessions.insert(id.clone(), (session, Instant::now()));
        drop(sessions);
        return reply(r, Some(id));
    }

    let Some(id) = head.header("Mcp-Session-Id") else {
        return Resp::rpc_error(
            400,
            INVALID_REQUEST,
            "Missing Mcp-Session-Id header: send initialize first",
        );
    };
    let Some(mut session) = shared
        .sessions
        .lock()
        .expect("sessions poisoned")
        .get(id)
        .map(|(s, _)| s.clone())
    else {
        return Resp::rpc_error(
            404,
            INVALID_REQUEST,
            "Unknown or expired session: initialize again",
        );
    };
    let r = shared.server.handle_value(msg, &mut session).await;
    if let Some(entry) = shared
        .sessions
        .lock()
        .expect("sessions poisoned")
        .get_mut(id)
    {
        *entry = (session, Instant::now());
    }
    reply(r, None)
}

fn reply(r: Option<crate::engine::Reply>, session: Option<String>) -> Resp {
    let resp = match r {
        None => Resp::empty(202),
        Some(r) => Resp::json(r.http_status, &r.message),
    };
    match session {
        Some(id) => resp.header("Mcp-Session-Id", id),
        None => resp,
    }
}

fn check_modern_headers(head: &Head, msg: &Value, version: &str) -> Result<(), RpcError> {
    must_match(
        "MCP-Protocol-Version",
        head.header("MCP-Protocol-Version"),
        version,
    )?;
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    must_match("Mcp-Method", head.header("Mcp-Method"), method)?;
    let name_field = match method {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" => Some("uri"),
        _ => None,
    };
    if let (Some(field), true) = (name_field, msg.get("id").is_some()) {
        let body = msg
            .get("params")
            .and_then(|p| p.get(field))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let decoded = match head.header("Mcp-Name") {
            Some(h) => Some(decode_header_value(h).ok_or_else(|| {
                RpcError::header_mismatch("Header mismatch: Mcp-Name header isn't valid")
            })?),
            None => None,
        };
        must_match("Mcp-Name", decoded.as_deref(), body)?;
    }
    Ok(())
}

fn must_match(name: &str, header: Option<&str>, body: &str) -> Result<(), RpcError> {
    match header {
        None => Err(RpcError::header_mismatch(format!(
            "Header mismatch: {name} header is missing"
        ))),
        Some(h) if h != body => Err(RpcError::header_mismatch(format!(
            "Header mismatch: {name} header value '{h}' does not match body value '{body}'"
        ))),
        Some(_) => Ok(()),
    }
}

/// Undo the `=?base64?…?=` encoding used for header values that aren't plain ASCII.
pub(crate) fn decode_header_value(v: &str) -> Option<String> {
    match v
        .strip_prefix("=?base64?")
        .and_then(|r| r.strip_suffix("?="))
    {
        Some(b64) => base64::engine::general_purpose::STANDARD
            .decode(b64)
            .ok()
            .and_then(|b| String::from_utf8(b).ok()),
        None => Some(v.to_owned()),
    }
}

/// Encode a header value as the spec asks: plain when it's safe ASCII, else base64.
pub(crate) fn encode_header_value(v: &str) -> String {
    let plain = !v.is_empty()
        && v.bytes().all(|b| (0x20..0x7f).contains(&b))
        && !v.starts_with(' ')
        && !v.ends_with(' ')
        && !(v.starts_with("=?base64?") && v.ends_with("?="));
    if plain {
        v.to_owned()
    } else {
        format!(
            "=?base64?{}?=",
            base64::engine::general_purpose::STANDARD.encode(v)
        )
    }
}

fn host_ok(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else { return false };
    let host = host.to_ascii_lowercase();
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|h| host == format!("{h}:{port}"))
}

/// Pages served from this computer may call (they still need the token); any other
/// web page may not.
fn origin_ok(origin: &str) -> bool {
    let origin = origin.to_ascii_lowercase();
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let host = if let Some(v6) = rest.strip_prefix('[') {
        match v6.split_once(']') {
            Some((h, after)) if after.is_empty() || after.starts_with(':') => {
                return h == "::1" && port_ok(after);
            }
            _ => return false,
        }
    } else {
        rest
    };
    let (name, port) = match host.split_once(':') {
        Some((n, p)) => (n, format!(":{p}")),
        None => (host, String::new()),
    };
    matches!(name, "localhost" | "127.0.0.1") && port_ok(&port)
}

fn port_ok(after: &str) -> bool {
    after.is_empty()
        || after
            .strip_prefix(':')
            .is_some_and(|p| !p.is_empty() && p.parse::<u16>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_origins() {
        assert!(host_ok(Some("127.0.0.1:4000"), 4000));
        assert!(host_ok(Some("LOCALHOST:4000"), 4000));
        assert!(host_ok(Some("[::1]:4000"), 4000));
        assert!(!host_ok(Some("127.0.0.1:4001"), 4000));
        assert!(!host_ok(Some("evil.example:4000"), 4000));
        assert!(!host_ok(Some("127.0.0.1"), 4000));
        assert!(!host_ok(None, 4000));

        for ok in [
            "http://localhost",
            "http://localhost:6274",
            "https://127.0.0.1:1",
            "http://[::1]:80",
        ] {
            assert!(origin_ok(ok), "{ok}");
        }
        for bad in [
            "null",
            "http://evil.example",
            "http://localhost.evil.example",
            "http://127.0.0.1.nip.io",
            "file://",
            "tauri://localhost",
            "http://[::2]",
            "http://localhost:abc",
            "http://localhost/path",
        ] {
            assert!(!origin_ok(bad), "{bad}");
        }
    }

    #[test]
    fn header_value_encoding() {
        for v in [
            "get_profile",
            "Hello, 世界",
            " padded ",
            "line1\nline2",
            "=?base64?literal?=",
        ] {
            assert_eq!(
                decode_header_value(&encode_header_value(v)).as_deref(),
                Some(v)
            );
        }
        assert_eq!(encode_header_value("press_key"), "press_key");
        assert_eq!(
            encode_header_value("Hello, 世界"),
            "=?base64?SGVsbG8sIOS4lueVjA==?="
        );
    }
}
