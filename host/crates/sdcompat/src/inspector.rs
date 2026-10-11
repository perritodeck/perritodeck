//! Property inspectors: a Stream Deck plugin's own settings page for an action (HTML in
//! the plugin's folder). The desktop app shows it in a frame; this module serves its
//! files over HTTP on 127.0.0.1 and lets it talk to the plugin like the Stream Deck app
//! does (`registerPropertyInspector`, `setSettings`, `sendToPlugin`, …).
//!
//! URLs carry a secret made at start, so only the app (which asks for them) can open
//! them. The page connects to the plugin socket from that HTTP origin, which is the
//! only `Origin` the socket accepts.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use perritodeck_deck::AgentKey;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, info};

use super::{images, manifest, Inner};

/// The settings page that's open, if any (one at a time, like the Stream Deck app).
pub(crate) struct PiConn {
    pub key: AgentKey,
    pub id: u64,
    pub tx: mpsc::UnboundedSender<String>,
}

/// The HTTP origin settings pages are served from.
pub(crate) fn origin(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// The URL of the settings page for an action instance, or why there isn't one.
pub(crate) fn url_for(inner: &Inner, key: &AgentKey) -> Result<String, String> {
    let mut st = inner.state.lock().expect("state poisoned");
    let p = st
        .plugins
        .get(&key.plugin)
        .ok_or_else(|| format!("The Stream Deck plugin {} isn't installed.", key.plugin))?;
    let action = p
        .manifest
        .action(&key.action)
        .ok_or_else(|| format!("{} has no action {}.", p.manifest.name, key.action))?;
    let path = action
        .property_inspector_path
        .clone()
        .ok_or_else(|| format!("“{}” has no settings page.", action.name))?;
    if !manifest::inside(&p.folder, &path).is_some_and(|f| f.is_file()) {
        return Err(format!(
            "{}'s settings page ({path}) is missing.",
            p.manifest.name
        ));
    }
    st.pi_pending.insert(key.context.clone(), key.clone());
    Ok(format!(
        "{}/pi/{}/{}/{}?ctx={}",
        origin(inner.http_port),
        inner.pi_secret,
        enc(&key.plugin),
        path.split('/').map(enc).collect::<Vec<_>>().join("/"),
        enc(&key.context)
    ))
}

/// Percent-encode everything but unreserved characters.
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Serve settings pages until the host stops.
pub(crate) async fn serve(inner: std::sync::Weak<Inner>, listener: TcpListener) {
    while let Ok((stream, _)) = listener.accept().await {
        let Some(inner) = inner.upgrade() else { return };
        tokio::spawn(async move {
            let _ = request(&inner, stream).await;
        });
    }
}

async fn request(inner: &Inner, mut stream: TcpStream) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut buf = [0u8; 2048];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buf))
            .await
            .map_err(|_| std::io::ErrorKind::TimedOut)??;
        if n == 0 || head.len() > 16 * 1024 {
            return Ok(());
        }
        head.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&head);
    let mut parts = text.lines().next().unwrap_or_default().split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let (status, ctype, body) = if method != "GET" {
        (405, "text/plain", b"Only GET".to_vec())
    } else {
        match page(inner, target) {
            Some((ctype, body)) => (200, ctype, body),
            None => (404, "text/plain", b"Not found".to_vec()),
        }
    };
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Method Not Allowed",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.shutdown().await
}

/// `/pi/<secret>/<plugin uuid>/<file>?ctx=<context>` → the file (the page itself with the
/// connection script added).
fn page(inner: &Inner, target: &str) -> Option<(&'static str, Vec<u8>)> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let rest = path.strip_prefix("/pi/")?;
    let (secret, rest) = rest.split_once('/')?;
    if !constant_eq(secret.as_bytes(), inner.pi_secret.as_bytes()) {
        return None;
    }
    let (uuid, file) = rest.split_once('/')?;
    let uuid = images::percent_decode(uuid);
    let file = images::percent_decode(file);
    let st = inner.state.lock().expect("state poisoned");
    let p = st.plugins.get(&uuid)?;
    let full = manifest::inside(&p.folder, &file)?;
    if !full.is_file() {
        return None;
    }
    let bytes = std::fs::read(&full).ok()?;
    let ctype = content_type(&file);
    let ctx = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("ctx="))
        .map(images::percent_decode);
    let key = ctx.and_then(|c| st.pi_pending.get(&c).cloned());
    match key {
        Some(key) if ctype.starts_with("text/html") && key.plugin == uuid => {
            let info = inner.info(&st, p);
            let action_info = action_info(&st, &key);
            let script = format!(
                "<script>(function(){{var go=function(){{var f=window.connectElgatoStreamDeckSocket||window.connectSocket;\
                 if(typeof f==='function')f({port},{ctx},'registerPropertyInspector',{info},{action});}};\
                 if(document.readyState==='complete')go();else window.addEventListener('load',go);}})();</script>",
                port = inner.port,
                ctx = js(&Value::String(key.context.clone())),
                info = js(&Value::String(info.to_string())),
                action = js(&Value::String(action_info.to_string())),
            );
            let mut html = String::from_utf8_lossy(&bytes).into_owned();
            match html.rfind("</body>") {
                Some(i) => html.insert_str(i, &script),
                None => html.push_str(&script),
            }
            Some((ctype, html.into_bytes()))
        }
        _ => Some((ctype, bytes)),
    }
}

/// A JSON value safe to put inside a `<script>`.
fn js(v: &Value) -> String {
    v.to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn content_type(file: &str) -> &'static str {
    let ext = file.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

fn payload(st: &super::State, key: &AgentKey) -> Value {
    json!({
        "controller": "Keypad",
        "coordinates": { "column": key.col, "row": key.row },
        "isInMultiAction": false,
        "settings": super::settings_of(st, key),
        "state": 0,
    })
}

fn action_info(st: &super::State, key: &AgentKey) -> Value {
    json!({ "action": key.action, "context": key.context, "device": key.device, "payload": payload(st, key) })
}

/// A settings page's WebSocket, after `registerPropertyInspector`.
pub(crate) async fn connection(
    inner: Arc<Inner>,
    context: String,
    mut sink: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>,
        WsMessage,
    >,
    mut source: futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<TcpStream>>,
) {
    let id = inner
        .connections
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let key = {
        let mut st = inner.state.lock().expect("state poisoned");
        let Some(key) = st.pi_pending.get(&context).cloned() else {
            debug!("Stream Deck: settings page for an unknown key");
            return;
        };
        // A newer page replaces the old one.
        if let Some(old) = st.pi.take() {
            send_plugin(&st, &old.key, disappear(&old.key));
        }
        st.pi = Some(PiConn {
            key: key.clone(),
            id,
            tx: tx.clone(),
        });
        send_plugin(
            &st,
            &key,
            json!({ "event": "propertyInspectorDidAppear", "action": key.action, "context": key.context, "device": key.device }),
        );
        key
    };
    info!(plugin = %key.plugin, "settings page open for {}", key.context);
    let writer = tokio::spawn(async move {
        while let Some(text) = rx.recv().await {
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    while let Some(Ok(msg)) = source.next().await {
        if let WsMessage::Text(t) = msg {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                handle(&inner, &key, &tx, &v);
            }
        }
    }
    writer.abort();
    let mut st = inner.state.lock().expect("state poisoned");
    if st.pi.as_ref().is_some_and(|p| p.id == id) {
        st.pi = None;
        send_plugin(&st, &key, disappear(&key));
    }
}

fn disappear(key: &AgentKey) -> Value {
    json!({ "event": "propertyInspectorDidDisappear", "action": key.action, "context": key.context, "device": key.device })
}

fn send_plugin(st: &super::State, key: &AgentKey, msg: Value) {
    if let Some((_, tx)) = st.plugins.get(&key.plugin).and_then(|p| p.out.as_ref()) {
        let _ = tx.send(msg.to_string());
    }
}

/// A message from a settings page.
fn handle(inner: &Inner, key: &AgentKey, to_page: &mpsc::UnboundedSender<String>, v: &Value) {
    let event = v["event"].as_str().unwrap_or_default();
    let mut st = inner.state.lock().expect("state poisoned");
    let with_id = |mut m: Value| {
        if let Some(id) = v.get("id") {
            m["id"] = id.clone();
        }
        m
    };
    match event {
        "setSettings" => {
            st.store
                .contexts
                .insert(super::store_key(key), v["payload"].clone());
            inner.save_store(&st);
            // The plugin hears about it, as with the Stream Deck app.
            let m = json!({ "event": "didReceiveSettings", "action": key.action, "context": key.context, "device": key.device, "payload": payload(&st, key) });
            send_plugin(&st, key, m);
        }
        "getSettings" => {
            let m = with_id(
                json!({ "event": "didReceiveSettings", "action": key.action, "context": key.context, "device": key.device, "payload": payload(&st, key) }),
            );
            let _ = to_page.send(m.to_string());
        }
        "setGlobalSettings" => {
            st.store
                .global
                .insert(key.plugin.clone(), v["payload"].clone());
            inner.save_store(&st);
            send_plugin(
                &st,
                key,
                json!({ "event": "didReceiveGlobalSettings", "payload": { "settings": v["payload"] } }),
            );
        }
        "getGlobalSettings" => {
            let settings = st
                .store
                .global
                .get(&key.plugin)
                .cloned()
                .unwrap_or_else(|| json!({}));
            let m = with_id(
                json!({ "event": "didReceiveGlobalSettings", "payload": { "settings": settings } }),
            );
            let _ = to_page.send(m.to_string());
        }
        "sendToPlugin" => {
            let m = json!({ "event": "sendToPlugin", "action": key.action, "context": key.context, "payload": v["payload"] });
            send_plugin(&st, key, m);
        }
        "openUrl" => {
            let url = v["payload"]["url"].as_str().unwrap_or_default();
            if url.starts_with("https://") || url.starts_with("http://") {
                let _ = open::that_detached(url);
            }
        }
        "logMessage" => {
            info!(plugin = %key.plugin, "settings page: {}", v["payload"]["message"].as_str().unwrap_or_default())
        }
        other => debug!(plugin = %key.plugin, "settings page sent {other}"),
    }
}

/// The plugin sent something to its settings page.
pub(crate) fn to_page(st: &super::State, plugin: &str, context: &str, payload: &Value) {
    if let Some(pi) = st
        .pi
        .as_ref()
        .filter(|p| p.key.plugin == plugin && p.key.context == context)
    {
        let m = json!({ "event": "sendToPropertyInspector", "action": pi.key.action, "context": context, "payload": payload });
        let _ = pi.tx.send(m.to_string());
    }
}
