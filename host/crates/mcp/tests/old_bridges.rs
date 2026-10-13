//! Bridges in the data folder can be older than the app (a copy an assistant is running
//! isn't replaced), so the endpoint must keep answering every bridge version's requests.
//!
//! `tests/fixtures/bridge-v<N>.txt` holds the requests bridge version N sends for the same
//! assistant session: an initialize handshake, a tool list, a tool call, and a stateless
//! (2026-07-28) call. One test checks this build's bridge still sends exactly what its
//! version's file says; the other replays every file against the endpoint.

mod common;

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use common::*;
use perritodeck_mcp::{bridge_with, serve_http, Endpoint, HttpOptions, AGENT, BRIDGE_VERSION};
use serde_json::{json, Value};
use tokio::{
    io::{duplex, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    time::timeout,
};

const TOKEN: &str = "fixture-token-0123456789";
const SESSION: &str = "fixture-session";

/// What the assistant sends, line by line.
const SCRIPT: &[&str] = &[
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"assistant","version":"1"}}}"#,
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_profiles","arguments":{}}}"#,
    r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_tablets","arguments":{},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{},"io.modelcontextprotocol/clientInfo":{"name":"assistant","version":"1"}}}}"#,
];

fn fixtures() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// One request as it went over the wire: head lines, then the body.
#[derive(Debug, Clone, PartialEq)]
struct Request {
    head: Vec<String>,
    body: String,
}

/// Requests in a fixture file: each is its head lines, a blank line and a one-line body,
/// separated by `---` lines. `#` lines at the top are comments.
fn parse_fixture(text: &str) -> Vec<Request> {
    let text = text.replace("\r\n", "\n");
    let mut out = Vec::new();
    for block in text.split("\n---\n") {
        let block: String = block
            .lines()
            .skip_while(|l| l.starts_with('#') || l.is_empty())
            .map(|l| format!("{l}\n"))
            .collect();
        let Some((head, body)) = block.split_once("\n\n") else {
            continue;
        };
        out.push(Request {
            head: head.lines().map(str::to_owned).collect(),
            body: body.trim_end().to_owned(),
        });
    }
    out
}

fn render_fixture(requests: &[Request]) -> String {
    requests
        .iter()
        .map(|r| format!("{}\n\n{}\n", r.head.join("\n"), r.body))
        .collect::<Vec<_>>()
        .join("---\n")
}

/// A host that writes down each request and gives a minimal answer.
async fn recording_host() -> (u16, Arc<Mutex<Vec<Request>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    tokio::spawn({
        let seen = seen.clone();
        async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let (head, body) = read_message(&mut s).await;
                let msg: Value = serde_json::from_str(&body).unwrap();
                seen.lock().unwrap().push(Request {
                    head: head.lines().map(str::to_owned).collect(),
                    body,
                });
                let answer = match (msg.get("id"), msg["method"].as_str()) {
                    (Some(id), Some("initialize")) => Some((
                        json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"t","version":"1"}}}),
                        format!("Mcp-Session-Id: {SESSION}\r\n"),
                    )),
                    (Some(id), _) => {
                        Some((json!({"jsonrpc":"2.0","id":id,"result":{}}), String::new()))
                    }
                    (None, _) => None,
                };
                let reply = match answer {
                    Some((v, extra)) => {
                        let v = v.to_string();
                        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{v}", v.len())
                    }
                    None => {
                        "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .into()
                    }
                };
                let _ = s.write_all(reply.as_bytes()).await;
            }
        }
    });
    (port, seen)
}

/// Read a request or response: its head (without the blank line) and body.
async fn read_message(s: &mut TcpStream) -> (String, String) {
    let mut buf = Vec::new();
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let mut chunk = [0u8; 4096];
        let n = s.read(&mut chunk).await.unwrap();
        assert!(n > 0, "the connection closed early");
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8(buf[..end].to_vec()).unwrap();
    let len = head
        .lines()
        .find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    let mut body = buf[end + 4..].to_vec();
    while body.len() < len {
        let mut chunk = [0u8; 4096];
        let n = s.read(&mut chunk).await.unwrap();
        assert!(n > 0, "the connection closed early");
        body.extend_from_slice(&chunk[..n]);
    }
    (head, String::from_utf8(body).unwrap())
}

#[tokio::test]
async fn this_bridge_sends_what_its_version_says() {
    let (port, seen) = recording_host().await;
    let url = format!("http://127.0.0.1:{port}/mcp");
    let (client, bridge_side) = duplex(1 << 16);
    let (bin, bout) = tokio::io::split(bridge_side);
    tokio::spawn(async move {
        let find = move || Endpoint {
            url: url.clone(),
            token: TOKEN.into(),
        };
        bridge_with(AGENT, find, bin, bout).await
    });
    let (read, mut write) = tokio::io::split(client);
    let mut lines = BufReader::new(read).lines();
    for line in SCRIPT {
        write
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        if line.contains(r#""id""#) {
            timeout(Duration::from_secs(10), lines.next_line())
                .await
                .expect("an answer in time")
                .unwrap()
                .expect("a line");
        }
    }
    // The notification has no answer of its own: wait until the host has had it all.
    timeout(Duration::from_secs(10), async {
        while seen.lock().unwrap().len() < SCRIPT.len() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("every request");

    let recorded: Vec<Request> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|r| Request {
            head: r
                .head
                .iter()
                .map(|l| {
                    l.replace(&format!("127.0.0.1:{port}"), "127.0.0.1:{port}")
                        .replace(TOKEN, "{token}")
                        .replace(SESSION, "{session}")
                        .replace(AGENT, "perritodeck-mcp/{version}")
                })
                .collect(),
            body: r.body.clone(),
        })
        .collect();
    let file = fixtures().join(format!("bridge-v{BRIDGE_VERSION}.txt"));
    let expected = parse_fixture(&std::fs::read_to_string(&file).unwrap_or_default());
    assert!(
        recorded == expected,
        "The bridge's requests changed, so bridges already out there differ from this one. \
         Raise BRIDGE_VERSION, save what it sends now as tests/fixtures/bridge-v<new>.txt, and \
         keep the older files: the endpoint must go on answering them.\n\n\
         This bridge sent:\n\n{}",
        render_fixture(&recorded)
    );
}

#[tokio::test]
async fn every_bridge_version_still_works() {
    let mut files: Vec<_> = std::fs::read_dir(fixtures())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("bridge-v") && n.ends_with(".txt"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty());
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let version: u32 = name
            .trim_start_matches("bridge-v")
            .trim_end_matches(".txt")
            .parse()
            .unwrap();
        let fake = Fake::new();
        let server = serve_http(fake.clone(), HttpOptions::new(TOKEN).port(0))
            .await
            .unwrap();
        let port = server.local_addr().port();
        let mut session = String::new();
        for req in parse_fixture(&std::fs::read_to_string(&file).unwrap()) {
            let head: String = req
                .head
                .iter()
                .map(|l| {
                    let l = l
                        .replace("{port}", &port.to_string())
                        .replace("{token}", TOKEN)
                        .replace("{session}", &session)
                        .replace("{version}", "0.0.0");
                    format!("{l}\r\n")
                })
                .collect();
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(format!("{head}\r\n{}", req.body).as_bytes())
                .await
                .unwrap();
            let (rhead, rbody) = timeout(Duration::from_secs(10), read_message(&mut s))
                .await
                .expect("an answer in time");
            let status = rhead.split(' ').nth(1).unwrap_or_default().to_owned();
            let msg: Value = serde_json::from_str(&req.body).unwrap();
            match msg.get("id") {
                None => assert_eq!(status, "202", "{name}: {}\n{rhead}", req.body),
                Some(id) => {
                    assert_eq!(status, "200", "{name}: {}\n{rhead}\n{rbody}", req.body);
                    let answer: Value = serde_json::from_str(&rbody).unwrap();
                    assert_eq!(&answer["id"], id, "{name}: {rbody}");
                    assert!(answer.get("result").is_some(), "{name}: {rbody}");
                }
            }
            if msg["method"] == "initialize" {
                session = rhead
                    .lines()
                    .find_map(|l| l.strip_prefix("Mcp-Session-Id: "))
                    .expect("a session")
                    .to_owned();
            }
        }
        let seen = fake.bridges.lock().unwrap().clone();
        assert!(
            seen.iter()
                .all(|(a, v)| a == "perritodeck-mcp/0.0.0" && *v == version),
            "{name}: {seen:?}"
        );
        assert_eq!(seen.len(), SCRIPT.len(), "{name}");
        server.shutdown().await;
    }
}
