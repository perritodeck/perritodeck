//! The Streamable HTTP endpoint, over real sockets.

mod common;

use std::net::SocketAddr;

use common::*;
use perritodeck_mcp::{serve_http, HttpOptions, HttpServer};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

const TOKEN: &str = "test-token-0123456789";

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {:?}", self.body))
    }
}

fn parse(raw: &[u8]) -> Resp {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n").expect("a full response");
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .map(|l| {
            let (n, v) = l.split_once(':').unwrap();
            (n.to_owned(), v.trim().to_owned())
        })
        .collect();
    Resp {
        status,
        headers,
        body: body.to_owned(),
    }
}

async fn raw(addr: SocketAddr, request: &str) -> Resp {
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    parse(&out)
}

fn request(addr: SocketAddr, headers: &[(&str, &str)], body: &str) -> String {
    let mut r = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
        addr.port(),
        body.len()
    );
    for (n, v) in headers {
        r.push_str(&format!("{n}: {v}\r\n"));
    }
    r.push_str("\r\n");
    r.push_str(body);
    r
}

async fn post(addr: SocketAddr, headers: &[(&str, &str)], body: &Value) -> Resp {
    let auth = format!("Bearer {TOKEN}");
    let mut all = vec![("Authorization", auth.as_str())];
    all.extend_from_slice(headers);
    raw(addr, &request(addr, &all, &body.to_string())).await
}

async fn start() -> (std::sync::Arc<Fake>, HttpServer) {
    let fake = Fake::new();
    let server = serve_http(fake.clone(), HttpOptions::new(TOKEN).port(0))
        .await
        .unwrap();
    (fake, server)
}

fn init(version: &str) -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"t","version":"1"}}})
}

#[tokio::test]
async fn tokens_are_required() {
    let (_fake, server) = start().await;
    let addr = server.local_addr();
    assert_eq!(
        server.url(),
        format!("http://127.0.0.1:{}/mcp", addr.port())
    );
    let body = init("2025-11-25").to_string();

    let r = raw(addr, &request(addr, &[], &body)).await;
    assert_eq!(r.status, 401);
    assert!(r.header("WWW-Authenticate").unwrap().starts_with("Bearer"));
    let r = raw(
        addr,
        &request(addr, &[("Authorization", "Bearer wrong")], &body),
    )
    .await;
    assert_eq!(r.status, 401);
    let r = raw(addr, &request(addr, &[("Authorization", TOKEN)], &body)).await;
    assert_eq!(r.status, 401);
    let r = post(addr, &[], &init("2025-11-25")).await;
    assert_eq!(r.status, 200);
}

#[tokio::test]
async fn web_pages_are_refused() {
    let (_fake, server) = start().await;
    let addr = server.local_addr();
    let auth = format!("Bearer {TOKEN}");
    let body = init("2025-11-25").to_string();
    for origin in [
        "https://evil.example",
        "null",
        "http://localhost.evil.example",
    ] {
        let r = raw(
            addr,
            &request(addr, &[("Authorization", &auth), ("Origin", origin)], &body),
        )
        .await;
        assert_eq!(r.status, 403, "{origin}");
        assert!(r.json()["error"].is_object());
        assert!(r.json().get("id").is_none());
    }
    // A rebinding page reaches 127.0.0.1 under its own name.
    let req = request(addr, &[("Authorization", &auth)], &body).replace(
        &format!("Host: 127.0.0.1:{}", addr.port()),
        &format!("Host: evil.example:{}", addr.port()),
    );
    assert_eq!(raw(addr, &req).await.status, 403);
    // Even without the token, 403 comes first.
    let r = raw(
        addr,
        &request(addr, &[("Origin", "https://evil.example")], &body),
    )
    .await;
    assert_eq!(r.status, 403);
    // Local tools are fine.
    let r = raw(
        addr,
        &request(
            addr,
            &[
                ("Authorization", &auth),
                ("Origin", "http://localhost:6274"),
            ],
            &body,
        ),
    )
    .await;
    assert_eq!(r.status, 200);
    let req = request(addr, &[("Authorization", &auth)], &body)
        .replace("Host: 127.0.0.1:", "Host: localhost:");
    assert_eq!(raw(addr, &req).await.status, 200);
}

#[tokio::test]
async fn http_rules() {
    let fake = Fake::new();
    let server = serve_http(
        fake.clone(),
        HttpOptions {
            max_body: 1000,
            ..HttpOptions::new(TOKEN).port(0)
        },
    )
    .await
    .unwrap();
    let addr = server.local_addr();
    let auth = format!("Bearer {TOKEN}");
    let host = format!("Host: 127.0.0.1:{}", addr.port());

    // No server-to-client stream.
    let r = raw(addr, &format!("GET /mcp HTTP/1.1\r\n{host}\r\nAuthorization: {auth}\r\nAccept: text/event-stream\r\n\r\n")).await;
    assert_eq!(r.status, 405);
    assert!(r.header("Allow").unwrap().contains("POST"));
    // Other paths.
    let r = raw(
        addr,
        &request(addr, &[("Authorization", &auth)], "{}").replace("POST /mcp", "POST /other"),
    )
    .await;
    assert_eq!(r.status, 404);
    // Body limit: refused before reading it.
    let big = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"pad\":\"{}\"}}",
        "x".repeat(2000)
    );
    let r = raw(addr, &request(addr, &[("Authorization", &auth)], &big)).await;
    assert_eq!(r.status, 413);
    // Length required; chunked isn't supported.
    let r = raw(addr, &format!("POST /mcp HTTP/1.1\r\n{host}\r\nAuthorization: {auth}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n")).await;
    assert_eq!(r.status, 411);
    let r = raw(addr, &format!("POST /mcp HTTP/1.1\r\n{host}\r\nAuthorization: {auth}\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n2\r\n{{}}\r\n0\r\n\r\n")).await;
    assert_eq!(r.status, 501);
    // JSON only.
    let r = raw(
        addr,
        &request(addr, &[("Authorization", &auth)], "{}")
            .replace("Content-Type: application/json", "Content-Type: text/plain"),
    )
    .await;
    assert_eq!(r.status, 415);
    // Not JSON at all.
    let r = raw(addr, &request(addr, &[("Authorization", &auth)], "{nope")).await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["error"]["code"], -32700);
    // Duplicate auth headers are refused outright.
    let r = raw(
        addr,
        &request(
            addr,
            &[("Authorization", &auth), ("Authorization", &auth)],
            "{}",
        ),
    )
    .await;
    assert_eq!(r.status, 400);
    // A bad request line or garbage.
    let r = raw(addr, "HELLO\r\n\r\n").await;
    assert_eq!(r.status, 400);
}

#[tokio::test]
async fn sessions_for_handshake_versions() {
    let (fake, server) = start().await;
    let addr = server.local_addr();
    let r = post(addr, &[], &init("2025-11-25")).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("Content-Type"), Some("application/json"));
    assert_eq!(r.json()["result"]["protocolVersion"], "2025-11-25");
    let sid = r.header("Mcp-Session-Id").unwrap().to_owned();
    assert!(sid.len() >= 32 && sid.bytes().all(|b| (0x21..=0x7e).contains(&b)));
    let h = |sid: &str| -> Vec<(String, String)> {
        vec![
            ("Mcp-Session-Id".into(), sid.into()),
            ("MCP-Protocol-Version".into(), "2025-11-25".into()),
        ]
    };
    let hs = h(&sid);
    let hs: Vec<(&str, &str)> = hs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();

    let r = post(
        addr,
        &hs,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    assert_eq!(r.status, 202);
    assert!(r.body.is_empty());

    let r = post(
        addr,
        &hs,
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["result"]["tools"].as_array().unwrap().len(), 4);
    fake.allow(true, true, true);
    let r = post(
        addr,
        &hs,
        &json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
    )
    .await;
    assert_eq!(r.json()["result"]["tools"].as_array().unwrap().len(), 14);

    let r = post(addr, &hs, &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"press_key","arguments":{"row":0,"col":1}}})).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["result"]["isError"], false);
    assert_eq!(fake.pressed.lock().unwrap().len(), 1);

    // Batches aren't part of 2025-11-25.
    let r = post(
        addr,
        &hs,
        &json!([{"jsonrpc":"2.0","id":5,"method":"ping"}]),
    )
    .await;
    assert_eq!(r.json()["error"]["code"], -32600);

    // Without a session, or an unknown one.
    let r = post(
        addr,
        &[],
        &json!({"jsonrpc":"2.0","id":6,"method":"tools/list"}),
    )
    .await;
    assert_eq!(r.status, 400);
    let r = post(
        addr,
        &[("Mcp-Session-Id", "nope")],
        &json!({"jsonrpc":"2.0","id":7,"method":"tools/list"}),
    )
    .await;
    assert_eq!(r.status, 404);
    // A version nobody knows.
    let r = post(
        addr,
        &[
            ("Mcp-Session-Id", &sid),
            ("MCP-Protocol-Version", "1999-01-01"),
        ],
        &json!({"jsonrpc":"2.0","id":8,"method":"ping"}),
    )
    .await;
    assert_eq!(r.status, 400);

    // Ending the session.
    let del = |sid: &str| {
        format!(
        "DELETE /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\nMcp-Session-Id: {sid}\r\n\r\n",
        addr.port()
    )
    };
    assert_eq!(raw(addr, &del(&sid)).await.status, 204);
    assert_eq!(raw(addr, &del(&sid)).await.status, 404);
    let r = post(addr, &hs, &json!({"jsonrpc":"2.0","id":9,"method":"ping"})).await;
    assert_eq!(r.status, 404);

    // 2025-03-26 sessions take batches.
    let r = post(addr, &[], &init("2025-03-26")).await;
    let sid = r.header("Mcp-Session-Id").unwrap().to_owned();
    let r = post(addr, &[("Mcp-Session-Id", &sid)], &json!([{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"}])).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json(), json!([{"jsonrpc":"2.0","id":1,"result":{}}]));
    let r = post(
        addr,
        &[("Mcp-Session-Id", &sid)],
        &json!([{"jsonrpc":"2.0","method":"notifications/initialized"}]),
    )
    .await;
    assert_eq!(r.status, 202);
}

#[tokio::test]
async fn stateless_requests_and_their_headers() {
    let (_fake, server) = start().await;
    let addr = server.local_addr();
    let v = "2026-07-28";
    let list =
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":modern_meta()}});
    let r = post(
        addr,
        &[("MCP-Protocol-Version", v), ("Mcp-Method", "tools/list")],
        &list,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["result"]["resultType"], "complete");
    assert!(r.header("Mcp-Session-Id").is_none());
    // A stale session id is ignored.
    let r = post(
        addr,
        &[
            ("MCP-Protocol-Version", v),
            ("Mcp-Method", "tools/list"),
            ("Mcp-Session-Id", "old"),
        ],
        &list,
    )
    .await;
    assert_eq!(r.status, 200);

    // Headers must be there and match the body.
    for headers in [
        vec![("Mcp-Method", "tools/list")],
        vec![("MCP-Protocol-Version", v)],
        vec![
            ("MCP-Protocol-Version", "2025-11-25"),
            ("Mcp-Method", "tools/list"),
        ],
        vec![("MCP-Protocol-Version", v), ("Mcp-Method", "tools/call")],
    ] {
        let r = post(addr, &headers, &list).await;
        assert_eq!(r.status, 400, "{headers:?}");
        assert_eq!(r.json()["error"]["code"], -32020);
        assert_eq!(r.json()["id"], 1);
    }
    let call = |name: &str| json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":{},"_meta":modern_meta()}});
    let r = post(
        addr,
        &[("MCP-Protocol-Version", v), ("Mcp-Method", "tools/call")],
        &call("list_profiles"),
    )
    .await;
    assert_eq!(r.json()["error"]["code"], -32020);
    let r = post(
        addr,
        &[
            ("MCP-Protocol-Version", v),
            ("Mcp-Method", "tools/call"),
            ("Mcp-Name", "list_tablets"),
        ],
        &call("list_profiles"),
    )
    .await;
    assert_eq!(r.json()["error"]["code"], -32020);
    let r = post(
        addr,
        &[
            ("MCP-Protocol-Version", v),
            ("Mcp-Method", "tools/call"),
            ("Mcp-Name", "list_profiles"),
        ],
        &call("list_profiles"),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["result"]["isError"], false);
    // Base64-encoded names are decoded before comparing.
    let r = post(
        addr,
        &[
            ("MCP-Protocol-Version", v),
            ("Mcp-Method", "tools/call"),
            ("Mcp-Name", "=?base64?bGlzdF9wcm9maWxlcw==?="),
        ],
        &call("list_profiles"),
    )
    .await;
    assert_eq!(r.status, 200);

    // Unknown method: 404 with a JSON-RPC body, so clients can tell it from a missing endpoint.
    let m =
        json!({"jsonrpc":"2.0","id":3,"method":"prompts/list","params":{"_meta":modern_meta()}});
    let r = post(
        addr,
        &[("MCP-Protocol-Version", v), ("Mcp-Method", "prompts/list")],
        &m,
    )
    .await;
    assert_eq!(r.status, 404);
    assert_eq!(r.json()["error"]["code"], -32601);
    // Unsupported version.
    let mut meta = modern_meta();
    meta["io.modelcontextprotocol/protocolVersion"] = json!("2030-01-01");
    let m = json!({"jsonrpc":"2.0","id":4,"method":"tools/list","params":{"_meta":meta}});
    let r = post(
        addr,
        &[
            ("MCP-Protocol-Version", "2030-01-01"),
            ("Mcp-Method", "tools/list"),
        ],
        &m,
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["error"]["code"], -32022);
}

#[tokio::test]
async fn connections_are_kept_alive() {
    let (_fake, server) = start().await;
    let addr = server.local_addr();
    let mut s = TcpStream::connect(addr).await.unwrap();
    let body = init("2025-11-25").to_string();
    let one = request(
        addr,
        &[("Authorization", &format!("Bearer {TOKEN}"))],
        &body,
    )
    .replace("Connection: close\r\n", "");
    // Two requests in one write (pipelined), then a closing one.
    s.write_all(format!("{one}{one}").as_bytes()).await.unwrap();
    s.write_all(
        request(
            addr,
            &[("Authorization", &format!("Bearer {TOKEN}"))],
            &body,
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.matches("HTTP/1.1 200 OK").count(), 3, "{text}");
}

#[tokio::test]
async fn only_loopback_and_only_with_a_token() {
    let fake = Fake::new();
    let mut o = HttpOptions::new(TOKEN).port(0);
    o.addr = "0.0.0.0:0".parse().unwrap();
    assert!(serve_http(fake.clone(), o).await.is_err());
    assert!(serve_http(fake.clone(), HttpOptions::new(" ").port(0))
        .await
        .is_err());
    let server = serve_http(fake, HttpOptions::new(TOKEN).port(0))
        .await
        .unwrap();
    let addr = server.local_addr();
    server.shutdown().await;
    assert!(TcpStream::connect(addr).await.is_err());
}
