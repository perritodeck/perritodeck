//! stdio framing, and the stdio → HTTP bridge end to end through a real local server.

mod common;

use std::time::Duration;

use common::*;
use perritodeck_mcp::{bridge, serve_http, serve_stream, HttpOptions};
use serde_json::{json, Value};
use tokio::{
    io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines},
    time::timeout,
};

const TOKEN: &str = "bridge-token-0123456789";

async fn next<R: tokio::io::AsyncRead + Unpin>(lines: &mut Lines<BufReader<R>>) -> Value {
    let line = timeout(Duration::from_secs(10), lines.next_line())
        .await
        .expect("an answer in time")
        .unwrap()
        .expect("a line");
    serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"))
}

#[tokio::test]
async fn stdio_framing() {
    let fake = Fake::new();
    let (client, server_side) = duplex(1 << 16);
    let (server_in, server_out) = tokio::io::split(server_side);
    let task = tokio::spawn(serve_stream(fake.clone(), server_in, server_out));
    let (client_read, mut client_write) = tokio::io::split(client);
    let mut lines = BufReader::new(client_read).lines();

    // Two messages in one write, CRLF, a blank line, and a message split across writes.
    let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{}}});
    client_write
        .write_all(
            format!(
                "{init}\r\n\n{}\n",
                json!({"jsonrpc":"2.0","method":"notifications/initialized"})
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let r = next(&mut lines).await;
    assert_eq!(r["id"], 1);
    let ping = json!({"jsonrpc":"2.0","id":2,"method":"ping"}).to_string();
    let (a, b) = ping.split_at(10);
    client_write.write_all(a.as_bytes()).await.unwrap();
    client_write.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    client_write
        .write_all(format!("{b}\n").as_bytes())
        .await
        .unwrap();
    // The notification got no line: the next one answers the ping.
    assert_eq!(
        next(&mut lines).await,
        json!({"jsonrpc":"2.0","id":2,"result":{}})
    );

    // A line that's far too long gets an error, and the server carries on.
    let long = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\",\"x\":\"{}\"}}\n",
        "x".repeat(1_100_000)
    );
    client_write.write_all(long.as_bytes()).await.unwrap();
    assert_eq!(next(&mut lines).await["error"]["code"], -32600);
    client_write.write_all(b"not json\n").await.unwrap();
    assert_eq!(next(&mut lines).await["error"]["code"], -32700);

    // Tool results are one line each, even with newlines in them.
    client_write
        .write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_profile","arguments":{}}})).as_bytes())
        .await
        .unwrap();
    let r = next(&mut lines).await;
    assert!(r["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains('\n'));

    // Closing stdin ends the server.
    client_write.shutdown().await.unwrap();
    drop(client_write);
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

struct Bridge {
    write: tokio::io::WriteHalf<DuplexStream>,
    lines: Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
}

impl Bridge {
    fn start(url: String, token: &'static str) -> Self {
        let (client, bridge_side) = duplex(1 << 16);
        let (bin, bout) = tokio::io::split(bridge_side);
        tokio::spawn(async move { bridge(&url, token, bin, bout).await });
        let (read, write) = tokio::io::split(client);
        Self {
            write,
            lines: BufReader::new(read).lines(),
        }
    }

    async fn send(&mut self, v: Value) {
        self.write
            .write_all(format!("{v}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn recv(&mut self) -> Value {
        let line = timeout(Duration::from_secs(10), self.lines.next_line())
            .await
            .expect("an answer in time")
            .unwrap()
            .expect("a line");
        serde_json::from_str(&line).unwrap()
    }
}

#[tokio::test]
async fn bridge_end_to_end() {
    let fake = Fake::new();
    fake.allow(true, false, false);
    let server = serve_http(fake.clone(), HttpOptions::new(TOKEN).port(0))
        .await
        .unwrap();
    let port = server.local_addr().port();
    let mut b = Bridge::start(server.url(), TOKEN);

    b.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}})).await;
    let r = b.recv().await;
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    b.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
    b.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .await;
    let r = b.recv().await;
    assert_eq!(r["id"], 2);
    assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 6);
    b.send(json!({"jsonrpc":"2.0","id":"three","method":"tools/call","params":{"name":"press_key","arguments":{"row":1,"col":1}}})).await;
    let r = b.recv().await;
    assert_eq!(r["id"], "three");
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(fake.pressed.lock().unwrap().len(), 1);

    // Stateless requests get their headers added by the bridge.
    b.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_profiles","arguments":{},"_meta":modern_meta()}})).await;
    let r = b.recv().await;
    assert_eq!(r["result"]["resultType"], "complete", "{r}");
    b.send(json!({"jsonrpc":"2.0","id":5,"method":"nope","params":{"_meta":modern_meta()}}))
        .await;
    assert_eq!(b.recv().await["error"]["code"], -32601);

    // The host restarts (same port, sessions forgotten): the bridge starts a new session.
    server.shutdown().await;
    let server = serve_http(fake.clone(), HttpOptions::new(TOKEN).port(port))
        .await
        .unwrap();
    b.send(json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"press_key","arguments":{"row":1,"col":2}}})).await;
    let r = b.recv().await;
    assert_eq!(r["id"], 6);
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(fake.pressed.lock().unwrap().len(), 2);

    // The host stops: requests get an error that says what to do; notifications nothing.
    server.shutdown().await;
    b.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
    b.send(json!({"jsonrpc":"2.0","id":7,"method":"tools/list"}))
        .await;
    let r = b.recv().await;
    assert_eq!(r["id"], 7);
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("isn't running"),
        "{r}"
    );
}

#[tokio::test]
async fn bridge_with_a_wrong_token() {
    let fake = Fake::new();
    let server = serve_http(fake, HttpOptions::new(TOKEN).port(0))
        .await
        .unwrap();
    let mut b = Bridge::start(server.url(), "wrong-token");
    b.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{}}})).await;
    let r = b.recv().await;
    assert_eq!(r["id"], 1);
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("access token"),
        "{r}"
    );
}

#[tokio::test]
async fn bridge_refuses_remote_urls() {
    let (_c, side) = duplex(64);
    let (i, o) = tokio::io::split(side);
    let e = bridge("http://example.com:46711/mcp", TOKEN, i, o)
        .await
        .unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
}
