//! OBS Studio via obs-websocket 5 (built into OBS 28+).
//!
//! One connection is opened on first use and reused; if it drops, the next action
//! reconnects. Protocol: <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use perritodeck_actions::{ActionError, ObsAction, Outcome};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{net::TcpStream, sync::Mutex, time::timeout};
use tokio_tungstenite::{
    connect_async, tungstenite::Message as WsMessage, MaybeTlsStream, WebSocketStream,
};
use tracing::debug;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const RPC_VERSION: u64 = 1;
const TIMEOUT: Duration = Duration::from_secs(5);

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
    conn: Mutex<Option<Ws>>,
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

async fn next_json(ws: &mut Ws) -> Result<Value, ActionError> {
    loop {
        let frame = timeout(TIMEOUT, ws.next())
            .await
            .map_err(|_| err("OBS didn't answer in time"))?
            .ok_or_else(|| err("OBS closed the connection"))?
            .map_err(|e| err(format!("connection problem: {e}")))?;
        match frame {
            WsMessage::Text(t) => {
                return serde_json::from_str(t.as_str())
                    .map_err(|e| err(format!("unexpected reply: {e}")))
            }
            WsMessage::Close(c) => {
                let why = c.map(|c| c.reason.to_string()).unwrap_or_default();
                return Err(err(if why.is_empty() {
                    "OBS closed the connection".to_owned()
                } else {
                    format!("OBS refused the connection: {why}")
                }));
            }
            _ => continue,
        }
    }
}

impl ObsClient {
    pub fn new(config: ObsConfig) -> Self {
        Self {
            config,
            conn: Mutex::new(None),
            next_id: Default::default(),
        }
    }

    async fn connect(&self) -> Result<Ws, ActionError> {
        let (mut ws, _) = timeout(TIMEOUT, connect_async(&self.config.url))
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
        let hello = next_json(&mut ws).await?;
        if hello["op"] != 0 {
            return Err(err("unexpected greeting from OBS"));
        }
        let mut identify =
            json!({ "op": 1, "d": { "rpcVersion": RPC_VERSION, "eventSubscriptions": 0 } });
        if let Some(auth) = hello["d"].get("authentication") {
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
        let identified = next_json(&mut ws).await.map_err(|e| match e {
            ActionError::Integration { reason, .. } if reason.contains("closed") => {
                err("OBS rejected the password in integrations.toml")
            }
            other => other,
        })?;
        if identified["op"] != 2 {
            return Err(err("OBS didn't accept the connection"));
        }
        debug!("connected to OBS at {}", self.config.url);
        Ok(ws)
    }

    async fn request(
        ws: &mut Ws,
        id: &str,
        request_type: &str,
        data: &Value,
    ) -> Result<Value, ActionError> {
        let req = json!({ "op": 6, "d": { "requestType": request_type, "requestId": id, "requestData": data } });
        ws.send(WsMessage::text(req.to_string()))
            .await
            .map_err(|e| err(e.to_string()))?;
        loop {
            let msg = next_json(ws).await?;
            if msg["op"] == 7 && msg["d"]["requestId"] == id {
                return Ok(msg["d"].clone());
            }
        }
    }

    /// Send any request; resolves to its `responseData` (or `{}`), or an error with
    /// OBS's comment when it refuses.
    pub async fn call(&self, request_type: &str, data: Value) -> Result<Value, ActionError> {
        let resp = self.send(request_type, &data).await?;
        let status = &resp["requestStatus"];
        if status["result"].as_bool().unwrap_or(false) {
            Ok(resp
                .get("responseData")
                .cloned()
                .unwrap_or_else(|| json!({})))
        } else {
            Err(err(format!(
                "OBS: {}",
                status["comment"].as_str().unwrap_or("request failed")
            )))
        }
    }

    pub async fn run(&self, action: &ObsAction) -> Result<Outcome, ActionError> {
        let (request_type, data) = request_for(action);
        let resp = self.send(request_type, &data).await?;
        let status = &resp["requestStatus"];
        if status["result"].as_bool().unwrap_or(false) {
            Ok(Outcome::Ok)
        } else {
            let comment = status["comment"].as_str().unwrap_or("request failed");
            Ok(Outcome::Alert(format!("OBS: {comment}")))
        }
    }

    async fn send(&self, request_type: &str, data: &Value) -> Result<Value, ActionError> {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .to_string();
        let mut guard = self.conn.lock().await;
        // Reuse the connection; if it has gone stale, reconnect once and retry.
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(self.connect().await?);
            }
            let ws = guard.as_mut().expect("just connected");
            match Self::request(ws, &id, request_type, data).await {
                Ok(resp) => return Ok(resp),
                Err(e) if attempt == 0 => {
                    debug!("OBS connection lost ({e}); reconnecting");
                    *guard = None;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("loop returns on the second attempt")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

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

    /// A tiny obs-websocket server: optional password, records requests, answers success
    /// unless the scene is called "Missing".
    async fn fake_obs(
        password: Option<&'static str>,
    ) -> (String, tokio::sync::mpsc::UnboundedReceiver<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
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
                                        let _ = ws.close(None).await;
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
                                let ok = d["requestData"]["sceneName"] != "Missing";
                                let _ = tx.send(d.clone());
                                let resp = json!({ "op": 7, "d": {
                                    "requestType": d["requestType"], "requestId": d["requestId"],
                                    "requestStatus": { "result": ok, "code": if ok { 100 } else { 600 },
                                                       "comment": if ok { Value::Null } else { json!("No source was found by the name of `Missing`.") } }
                                }});
                                ws.send(WsMessage::text(resp.to_string())).await.unwrap();
                            }
                            _ => {}
                        }
                    }
                });
            }
        });
        (url, rx)
    }

    #[tokio::test]
    async fn runs_requests_with_password() {
        let (url, mut seen) = fake_obs(Some("hunter2")).await;
        let obs = ObsClient::new(ObsConfig {
            url,
            password: Some("hunter2".into()),
        });
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
        let first = seen.recv().await.unwrap();
        assert_eq!(first["requestType"], "SetCurrentProgramScene");
        assert_eq!(first["requestData"]["sceneName"], "Gameplay");
        assert_eq!(seen.recv().await.unwrap()["requestType"], "ToggleRecord");
    }

    #[tokio::test]
    async fn obs_errors_become_alerts() {
        let (url, _) = fake_obs(None).await;
        let obs = ObsClient::new(ObsConfig {
            url,
            password: None,
        });
        let out = obs
            .run(&ObsAction::Scene {
                scene: "Missing".into(),
            })
            .await
            .unwrap();
        assert!(matches!(out, Outcome::Alert(m) if m.contains("No source")));
    }

    #[tokio::test]
    async fn wrong_or_missing_password_is_explained() {
        let (url, _) = fake_obs(Some("right")).await;
        let wrong = ObsClient::new(ObsConfig {
            url: url.clone(),
            password: Some("wrong".into()),
        });
        assert!(wrong
            .run(&ObsAction::ToggleStream)
            .await
            .unwrap_err()
            .to_string()
            .contains("password"));
        let none = ObsClient::new(ObsConfig {
            url,
            password: None,
        });
        assert!(none
            .run(&ObsAction::ToggleStream)
            .await
            .unwrap_err()
            .to_string()
            .contains("password"));
    }

    #[tokio::test]
    async fn obs_not_running_is_explained() {
        let obs = ObsClient::new(ObsConfig {
            url: "ws://127.0.0.1:1".into(),
            password: None,
        });
        assert!(obs
            .run(&ObsAction::ToggleRecord)
            .await
            .unwrap_err()
            .to_string()
            .contains("can't reach OBS"));
    }
}
