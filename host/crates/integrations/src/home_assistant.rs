//! Home Assistant via its REST API: `POST /api/services/<domain>/<service>`.

use std::time::Duration;

use perritodeck_actions::{ActionError, Outcome};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HaConfig {
    /// e.g. `http://homeassistant.local:8123`
    pub url: String,
    /// A long-lived access token (your profile → Security in Home Assistant).
    pub token: String,
}

pub struct HaClient {
    config: HaConfig,
    agent: ureq::Agent,
}

fn err(reason: impl Into<String>) -> ActionError {
    ActionError::Integration {
        service: "Home Assistant".into(),
        reason: reason.into(),
    }
}

/// The JSON body for a service call: `data` plus `entity_id` if given.
pub fn body(entity: Option<&str>, data: Option<&Value>) -> Value {
    let mut body = match data {
        Some(Value::Object(m)) => Value::Object(m.clone()),
        _ => json!({}),
    };
    if let Some(e) = entity {
        body["entity_id"] = json!(e);
    }
    body
}

impl HaClient {
    pub fn new(config: HaConfig) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(8)))
            .http_status_as_error(false)
            .build()
            .into();
        Self { config, agent }
    }

    /// An entity's state object (`state`, `attributes`, …).
    pub async fn state(&self, entity: &str) -> Result<Value, ActionError> {
        if !entity
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            return Err(err(format!(
                "{entity:?} isn't an entity id like \"light.desk\""
            )));
        }
        let url = format!(
            "{}/api/states/{entity}",
            self.config.url.trim_end_matches('/')
        );
        let agent = self.agent.clone();
        let token = self.config.token.clone();
        let base = self.config.url.clone();
        let entity = entity.to_owned();
        tokio::task::spawn_blocking(move || {
            let mut resp = agent
                .get(&url)
                .header("Authorization", &format!("Bearer {token}"))
                .call()
                .map_err(|e| err(format!("can't reach {base}: {e}")))?;
            match resp.status().as_u16() {
                200 => resp
                    .body_mut()
                    .read_json::<Value>()
                    .map_err(|e| err(format!("unexpected answer from Home Assistant: {e}"))),
                401 => Err(err("the token in integrations.toml was rejected")),
                404 => Err(err(format!("Home Assistant doesn't know {entity}"))),
                code => Err(err(format!("Home Assistant answered {code}"))),
            }
        })
        .await
        .map_err(|e| err(e.to_string()))?
    }

    pub async fn call(
        &self,
        service: &str,
        entity: Option<&str>,
        data: Option<&Value>,
    ) -> Result<Outcome, ActionError> {
        let (domain, name) = service
            .split_once('.')
            .ok_or_else(|| err(format!("{service:?} isn't a service like \"light.toggle\"")))?;
        let url = format!(
            "{}/api/services/{domain}/{name}",
            self.config.url.trim_end_matches('/')
        );
        let body = body(entity, data);
        let agent = self.agent.clone();
        let token = self.config.token.clone();
        let base = self.config.url.clone();
        let service = format!("{domain}.{name}");
        // ureq is blocking; keep it off the async workers.
        tokio::task::spawn_blocking(move || {
            let resp = agent
                .post(&url)
                .header("Authorization", &format!("Bearer {token}"))
                .send_json(&body)
                .map_err(|e| err(format!("can't reach {base}: {e}")))?;
            match resp.status().as_u16() {
                200..=299 => Ok(Outcome::Ok),
                401 => Err(err("the token in integrations.toml was rejected")),
                400 | 404 => Ok(Outcome::Alert(format!(
                    "Home Assistant doesn't know {service} (or its entity)"
                ))),
                code => Ok(Outcome::Alert(format!("Home Assistant answered {code}"))),
            }
        })
        .await
        .map_err(|e| err(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::mpsc,
    };

    #[test]
    fn body_merges_entity_and_data() {
        assert_eq!(
            body(Some("light.desk"), None),
            json!({ "entity_id": "light.desk" })
        );
        assert_eq!(
            body(Some("light.desk"), Some(&json!({ "brightness_pct": 40 }))),
            json!({ "entity_id": "light.desk", "brightness_pct": 40 })
        );
        assert_eq!(body(None, None), json!({}));
    }

    /// One-shot HTTP server that records the request and answers with `status`.
    fn fake_ha(status: u16) -> (String, mpsc::Receiver<(String, String, String)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let (mut auth, mut len) = (String::new(), 0usize);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let l = line.trim_end();
                if l.is_empty() {
                    break;
                }
                let lower = l.to_ascii_lowercase();
                if lower.starts_with("authorization:") {
                    auth = l[14..].trim().to_owned();
                }
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            let _ = tx.send((
                request_line.trim().to_owned(),
                auth,
                String::from_utf8(body).unwrap(),
            ));
            let reply = format!("HTTP/1.1 {status} X\r\nContent-Length: 2\r\nContent-Type: application/json\r\n\r\n[]");
            stream.write_all(reply.as_bytes()).unwrap();
        });
        (url, rx)
    }

    #[tokio::test]
    async fn calls_the_service_with_the_token() {
        let (url, seen) = fake_ha(200);
        let ha = HaClient::new(HaConfig {
            url: format!("{url}/"),
            token: "abc".into(),
        });
        let out = ha
            .call(
                "light.toggle",
                Some("light.desk"),
                Some(&json!({"transition": 2})),
            )
            .await
            .unwrap();
        assert_eq!(out, Outcome::Ok);
        let (line, auth, body) = seen.recv().unwrap();
        assert_eq!(line, "POST /api/services/light/toggle HTTP/1.1");
        assert_eq!(auth, "Bearer abc");
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            json!({"entity_id": "light.desk", "transition": 2})
        );
    }

    #[tokio::test]
    async fn status_codes_are_explained() {
        let (url, _seen) = fake_ha(401);
        let e = HaClient::new(HaConfig {
            url,
            token: "x".into(),
        })
        .call("light.toggle", None, None)
        .await
        .unwrap_err();
        assert!(e.to_string().contains("token"));
        let (url, _seen2) = fake_ha(400);
        let out = HaClient::new(HaConfig {
            url,
            token: "x".into(),
        })
        .call("light.nope", None, None)
        .await
        .unwrap();
        assert!(matches!(out, Outcome::Alert(m) if m.contains("light.nope")));
        // Named once: not "Home Assistant: Home Assistant doesn't know …".
        let (url, _seen3) = fake_ha(404);
        let e = HaClient::new(HaConfig {
            url,
            token: "x".into(),
        })
        .state("light.gone")
        .await
        .unwrap_err();
        assert_eq!(e.to_string(), "Home Assistant doesn't know light.gone");
    }

    #[tokio::test]
    async fn unreachable_host_is_an_error() {
        let ha = HaClient::new(HaConfig {
            url: "http://127.0.0.1:1".into(),
            token: "x".into(),
        });
        assert!(ha
            .call("light.toggle", None, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("can't reach"));
    }
}
