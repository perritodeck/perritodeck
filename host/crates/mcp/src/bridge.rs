//! stdio → HTTP bridge: an assistant launches `perritodeck-host mcp`, which passes each
//! line from stdin to the running host's HTTP endpoint and prints the answers.
//!
//! It handles the HTTP side of the protocol for the assistant: the access token, the
//! `Mcp-Session-Id` of handshake-based versions (and starting a new session if the host
//! restarted), and the `MCP-Protocol-Version` / `Mcp-Method` / `Mcp-Name` headers.
//! Messages are forwarded one at a time, in order.
//!
//! Assistants keep the bridge running for their whole session, so it doesn't hold on
//! to the port and token it started with: when the host can't be reached or refuses the
//! token, it looks them up again (the person may have changed the port or made a new
//! token) and sends the message again if they changed.

use std::{fmt, io, time::Duration};

use serde_json::{json, Value};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::timeout,
};
use tracing::warn;

use crate::{
    engine::META_VERSION,
    http::{encode_header_value, read_body, read_head},
    jsonrpc::{RpcError, INTERNAL_ERROR},
    stdio::{read_line, Line},
    DEFAULT_MAX_BODY,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Commands may run for up to ten minutes; leave room for that.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(11 * 60);
const MAX_RESPONSE: usize = 16 * 1024 * 1024;

/// Where the host's endpoint is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// `http://127.0.0.1:<port>/mcp`.
    pub url: String,
    pub token: String,
}

/// Bridge this process's stdin/stdout to the host's endpoint. Returns when stdin closes.
///
/// `find` says where the endpoint is. It's asked when the bridge starts, and again
/// whenever the host can't be reached or refuses the token.
pub async fn bridge_stdio_to_http<F>(find: F) -> io::Result<()>
where
    F: FnMut() -> Endpoint,
{
    bridge_with(find, tokio::io::stdin(), tokio::io::stdout()).await
}

/// [`bridge_with`] to an endpoint that stays put.
pub async fn bridge<R, W>(url: &str, token: &str, input: R, output: W) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let endpoint = Endpoint {
        url: url.to_owned(),
        token: token.to_owned(),
    };
    bridge_with(move || endpoint.clone(), input, output).await
}

/// [`bridge_stdio_to_http`] over any streams.
pub async fn bridge_with<F, R, W>(find: F, input: R, mut output: W) -> io::Result<()>
where
    F: FnMut() -> Endpoint,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut link = Link::new(find)?;
    let mut state = State::default();
    let mut input = BufReader::new(input);
    while let Some(line) = read_line(&mut input, DEFAULT_MAX_BODY).await? {
        let replies = match line {
            Line::Text(text) if text.trim().is_empty() => continue,
            Line::Text(text) => forward(&mut link, text.trim(), &mut state).await,
            Line::TooLong => vec![RpcError::invalid_request(format!(
                "message longer than {DEFAULT_MAX_BODY} bytes"
            ))
            .response(Value::Null)],
        };
        for r in replies {
            let mut line = r.to_string();
            line.push('\n');
            output.write_all(line.as_bytes()).await?;
        }
        output.flush().await?;
    }
    Ok(())
}

#[derive(Default)]
struct State {
    session: Option<String>,
    version: Option<String>,
    /// The client's initialize request, replayed if the host forgets the session.
    init: Option<String>,
    initialized: bool,
}

/// The endpoint the bridge sends to, and how to look it up again.
struct Link<F> {
    find: F,
    target: Target,
    token: String,
}

impl<F: FnMut() -> Endpoint> Link<F> {
    fn new(mut find: F) -> io::Result<Self> {
        let e = find();
        Ok(Self {
            target: Target::parse(&e.url)?,
            token: e.token,
            find,
        })
    }

    /// Look the endpoint up again. True if its address or token changed.
    fn refresh(&mut self) -> bool {
        let e = (self.find)();
        let target = match Target::parse(&e.url) {
            Ok(t) => t,
            Err(err) => {
                warn!("{err}");
                return false;
            }
        };
        if target == self.target && e.token == self.token {
            return false;
        }
        self.target = target;
        self.token = e.token;
        true
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Target {
    host: String,
    port: u16,
    path: String,
}

impl Target {
    fn parse(url: &str) -> io::Result<Self> {
        let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidInput, format!("{url}: {m}"));
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| bad("only http:// URLs on this computer are supported"))?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !h.ends_with(':') => {
                (h, p.parse::<u16>().map_err(|_| bad("bad port"))?)
            }
            _ => (authority, 80),
        };
        if !matches!(
            host.to_ascii_lowercase().as_str(),
            "127.0.0.1" | "localhost" | "[::1]"
        ) {
            return Err(bad("the bridge only connects to this computer (127.0.0.1)"));
        }
        Ok(Self {
            host: host.to_owned(),
            port,
            path: path.to_owned(),
        })
    }
}

struct Response {
    status: u16,
    session: Option<String>,
    /// JSON-RPC messages in the body (one, or several from an SSE stream).
    messages: Vec<Value>,
}

async fn forward<F>(link: &mut Link<F>, text: &str, st: &mut State) -> Vec<Value>
where
    F: FnMut() -> Endpoint,
{
    let msg: Option<Value> = serde_json::from_str(text).ok();
    let method = msg
        .as_ref()
        .and_then(|m| m.get("method"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let request_id = msg
        .as_ref()
        .filter(|m| m.get("method").is_some())
        .and_then(|m| m.get("id"))
        .cloned();
    let modern = msg
        .as_ref()
        .and_then(|m| m.get("params"))
        .and_then(|p| p.get("_meta"))
        .and_then(|m| m.get(META_VERSION))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let is_init = method.as_deref() == Some("initialize") && request_id.is_some();

    let mut resp = send(
        &link.target,
        &link.token,
        &headers(msg.as_ref(), modern.as_deref(), is_init, st),
        text,
    )
    .await;
    // Nothing listens where the host was, or it refused the token: the person may have
    // changed the port or made a new token. Look again, and if either changed, send it
    // again (the host did nothing with it, so that's safe).
    let undelivered = matches!(
        &resp,
        Err(SendError::Connect(_)) | Ok(Response { status: 401, .. })
    );
    if undelivered && link.refresh() {
        resp = send(
            &link.target,
            &link.token,
            &headers(msg.as_ref(), modern.as_deref(), is_init, st),
            text,
        )
        .await;
    }
    let (t, token) = (&link.target, link.token.as_str());
    // The host restarted and forgot the session: start a new one and try again.
    let lost = matches!(&resp, Ok(r) if r.status == 404)
        && st.session.is_some()
        && modern.is_none()
        && !is_init;
    if lost && reinitialize(t, token, st).await {
        resp = send(t, token, &headers(msg.as_ref(), None, false, st), text).await;
    }
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            warn!("can't reach PerritoDeck at {}:{}: {e}", t.host, t.port);
            return request_id
                .map(|id| {
                    RpcError::new(
                        INTERNAL_ERROR,
                        "PerritoDeck isn't running, or its connection for AI assistants is \
                         turned off. Start PerritoDeck and turn it on in its settings, under AI \
                         assistants.",
                    )
                    .response(id)
                })
                .into_iter()
                .collect();
        }
    };
    if is_init && resp.status == 200 {
        if let Some(v) = resp
            .messages
            .first()
            .and_then(|m| m.get("result"))
            .and_then(|r| r.get("protocolVersion"))
            .and_then(Value::as_str)
        {
            st.version = Some(v.to_owned());
            st.session = resp.session.clone();
            st.init = Some(text.to_owned());
            st.initialized = false;
        }
    }
    if method.as_deref() == Some("notifications/initialized") && resp.status < 300 {
        st.initialized = true;
    }

    let Some(id) = request_id else {
        if resp.status >= 400 {
            warn!("PerritoDeck refused a notification (HTTP {})", resp.status);
        }
        return Vec::new();
    };
    if resp.status == 401 {
        // The host's own explanation is about HTTP headers, which the person never sees.
        return vec![RpcError::new(
            INTERNAL_ERROR,
            "PerritoDeck didn't accept the bridge's access token. Set up the assistant again \
             with the command PerritoDeck shows in its settings, under AI assistants.",
        )
        .response(id)];
    }
    if resp.messages.is_empty() {
        if resp.status >= 300 {
            return vec![RpcError::new(
                INTERNAL_ERROR,
                format!("PerritoDeck answered HTTP {}", resp.status),
            )
            .response(id)];
        }
        return Vec::new();
    }
    resp.messages
        .into_iter()
        .map(|mut m| {
            // Errors the transport sends without an id belong to this request.
            if m.get("error").is_some() && m.get("id").map_or(true, Value::is_null) && !id.is_null()
            {
                m["id"] = id.clone();
            }
            m
        })
        .collect()
}

fn headers(
    msg: Option<&Value>,
    modern: Option<&str>,
    is_init: bool,
    st: &State,
) -> Vec<(String, String)> {
    let mut h = Vec::new();
    match modern {
        Some(version) => {
            h.push(("MCP-Protocol-Version".into(), version.to_owned()));
            let method = msg
                .and_then(|m| m.get("method"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            h.push(("Mcp-Method".into(), encode_header_value(method)));
            let field = match method {
                "tools/call" | "prompts/get" => Some("name"),
                "resources/read" => Some("uri"),
                _ => None,
            };
            if let Some(name) = field
                .and_then(|f| msg?.get("params")?.get(f))
                .and_then(Value::as_str)
            {
                h.push(("Mcp-Name".into(), encode_header_value(name)));
            }
        }
        None if !is_init => {
            if let Some(s) = &st.session {
                h.push(("Mcp-Session-Id".into(), s.clone()));
            }
            if let Some(v) = &st.version {
                h.push(("MCP-Protocol-Version".into(), v.clone()));
            }
        }
        None => {}
    }
    h
}

async fn reinitialize(t: &Target, token: &str, st: &mut State) -> bool {
    let Some(init) = st.init.clone() else {
        return false;
    };
    let Ok(r) = send(t, token, &[], &init).await else {
        return false;
    };
    if r.status != 200 || r.session.is_none() {
        return false;
    }
    st.session = r.session;
    if st.initialized {
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string();
        let _ = send(t, token, &headers(None, None, false, st), &note).await;
    }
    true
}

/// Why a message got no answer.
enum SendError {
    /// Nothing answered the connection, so the message wasn't sent.
    Connect(io::Error),
    /// It was sent, but no whole answer came back.
    Answer(io::Error),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Connect(e) | SendError::Answer(e) => e.fmt(f),
        }
    }
}

async fn send(
    t: &Target,
    token: &str,
    extra: &[(String, String)],
    body: &str,
) -> Result<Response, SendError> {
    let stream = timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((t.host.trim_matches(['[', ']']), t.port)),
    )
    .await
    .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into()))
    .map_err(SendError::Connect)?;
    exchange(stream, t, token, extra, body)
        .await
        .map_err(SendError::Answer)
}

async fn exchange(
    mut stream: TcpStream,
    t: &Target,
    token: &str,
    extra: &[(String, String)],
    body: &str,
) -> io::Result<Response> {
    let mut req = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n\
         Content-Length: {}\r\nConnection: close\r\n",
        t.path,
        t.host,
        t.port,
        body.len()
    );
    for (n, v) in extra {
        req.push_str(&format!("{n}: {v}\r\n"));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.flush().await?;

    timeout(RESPONSE_TIMEOUT, async {
        let mut buf = Vec::new();
        let head = read_head(&mut stream, &mut buf)
            .await?
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        let status = head
            .start
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad status line"))?;
        let body = match head.header("Content-Length").map(str::parse::<usize>) {
            Some(Ok(n)) if n <= MAX_RESPONSE => read_body(&mut stream, &mut buf, n).await?,
            Some(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "response too large",
                ))
            }
            None => {
                let mut rest = Vec::new();
                (&mut stream)
                    .take(MAX_RESPONSE as u64)
                    .read_to_end(&mut rest)
                    .await?;
                buf.extend_from_slice(&rest);
                buf
            }
        };
        let sse = head
            .header("Content-Type")
            .is_some_and(|c| c.to_ascii_lowercase().starts_with("text/event-stream"));
        let messages = if sse {
            sse_messages(&String::from_utf8_lossy(&body))
        } else {
            serde_json::from_slice::<Value>(&body)
                .ok()
                .into_iter()
                .collect()
        };
        Ok(Response {
            status,
            session: head.header("Mcp-Session-Id").map(str::to_owned),
            messages,
        })
    })
    .await
    .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

/// The JSON messages in an SSE body (`data:` lines of each event).
fn sse_messages(body: &str) -> Vec<Value> {
    body.replace("\r\n", "\n")
        .split("\n\n")
        .filter_map(|event| {
            let data: Vec<&str> = event
                .lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .map(|d| d.strip_prefix(' ').unwrap_or(d))
                .collect();
            serde_json::from_str(&data.join("\n")).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_urls() {
        let t = Target::parse("http://127.0.0.1:46711/mcp").unwrap();
        assert_eq!(
            (t.host.as_str(), t.port, t.path.as_str()),
            ("127.0.0.1", 46711, "/mcp")
        );
        let t = Target::parse("http://[::1]:5/mcp").unwrap();
        assert_eq!((t.host.as_str(), t.port), ("[::1]", 5));
        assert!(Target::parse("http://localhost:1/mcp").is_ok());
        assert!(Target::parse("https://127.0.0.1:1/mcp").is_err());
        assert!(Target::parse("http://example.com:1/mcp").is_err());
        assert!(Target::parse("http://127.0.0.1.evil.example/mcp").is_err());
    }

    #[test]
    fn sse_bodies() {
        let body = "id: 1\ndata: \n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        assert_eq!(
            sse_messages(body),
            vec![json!({"jsonrpc":"2.0","id":1,"result":{}})]
        );
    }
}
