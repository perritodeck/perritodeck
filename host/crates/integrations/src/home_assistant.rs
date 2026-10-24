//! Home Assistant via its REST API: `POST /api/services/<domain>/<service>`.

use std::time::Duration;

use perritodeck_actions::{ActionError, Outcome};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HaConfig {
    /// e.g. `http://homeassistant.local:8123`
    pub url: String,
    /// A long-lived access token (your profile → Security in Home Assistant).
    pub token: String,
    /// `false` keeps the settings but leaves Home Assistant off.
    #[serde(default = "crate::on")]
    pub enabled: bool,
}

/// Never shows the token.
impl std::fmt::Debug for HaConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HaConfig")
            .field("url", &self.url)
            .field("token", &"<hidden>")
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// Why Test connection failed, before it's worded for the form.
#[derive(Debug)]
pub(crate) enum Problem {
    Http(ureq::Error),
    Status(u16),
    /// A redirect, and where to (as shown in a message).
    Redirect {
        code: u16,
        to: Option<String>,
    },
    NotJson,
}

impl Problem {
    /// For the form; `base` is the address as typed (normalized), `at` its `host:port`.
    pub(crate) fn for_form(&self, base: &str, at: &str, host: &str, wait: Duration) -> String {
        let plain = base.starts_with("http://");
        let https_hint = || {
            format!(
                "Something at {at} answered, but not like Home Assistant. If Home Assistant \
                 uses https, start the address with https://."
            )
        };
        match self {
            Problem::Redirect { to: Some(to), .. } => {
                format!("Home Assistant at {at} sends you to {to}. Use that address instead.")
            }
            Problem::Redirect { code, to: None } => format!(
                "Home Assistant at {at} answered {code}, a redirect, without saying where to."
            ),
            Problem::Status(401) => "Home Assistant rejected the token. Make a new long-lived \
                                     access token (your profile in Home Assistant → Security) \
                                     and paste it here."
                .into(),
            Problem::Status(403) => "Home Assistant refused the token (403). It may have \
                                     blocked this computer after too many wrong tokens; Home \
                                     Assistant's log says so if it did."
                .into(),
            Problem::Status(404) => format!(
                "Something answered at {base}, but it isn't Home Assistant (404). Use the \
                 address you open Home Assistant with, like http://homeassistant.local:8123."
            ),
            // nginx's answer to plain HTTP on its https port.
            Problem::Status(400) if plain => https_hint(),
            Problem::Status(code) => format!("Home Assistant answered {code}."),
            Problem::NotJson => {
                format!("Something answered at {base}, but not like Home Assistant.")
            }
            Problem::Http(e) => {
                if let Some(tls) = tls_error(e) {
                    return tls_message(tls, at, host, plain);
                }
                use std::io::ErrorKind;
                let refused = || {
                    format!(
                        "Nothing answered at {at}. Is Home Assistant running there? Its usual \
                         port is 8123."
                    )
                };
                match e {
                    ureq::Error::Io(io) => match io.kind() {
                        ErrorKind::ConnectionRefused => refused(),
                        ErrorKind::TimedOut => crate::check::no_answer(at, wait),
                        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => {
                            crate::check::unreachable(at)
                        }
                        ErrorKind::UnexpectedEof
                        | ErrorKind::ConnectionReset
                        | ErrorKind::ConnectionAborted
                            if plain =>
                        {
                            https_hint()
                        }
                        _ => format!("Couldn't connect to Home Assistant at {at}: {e}"),
                    },
                    ureq::Error::ConnectionFailed => refused(),
                    ureq::Error::HostNotFound => format!(
                        "Can't find a computer called “{host}”. Check the name, or use its IP \
                         address."
                    ),
                    ureq::Error::Timeout(ureq::Timeout::Connect | ureq::Timeout::Resolve) => {
                        crate::check::no_answer(at, wait)
                    }
                    ureq::Error::Timeout(_) => format!(
                        "Home Assistant at {at} took the connection but didn't answer within 8 \
                         seconds. It may still be starting."
                    ),
                    ureq::Error::Protocol(_) if plain => https_hint(),
                    other => format!("Couldn't connect to Home Assistant at {at}: {other}"),
                }
            }
        }
    }
}

/// The TLS failure behind an error, if that's what it was. ureq reports one from the
/// handshake as an I/O error wrapping rustls's.
fn tls_error(e: &ureq::Error) -> Option<&rustls::Error> {
    match e {
        ureq::Error::Rustls(tls) => Some(tls),
        ureq::Error::Io(io) => io.get_ref()?.downcast_ref::<rustls::Error>(),
        _ => None,
    }
}

fn tls_message(e: &rustls::Error, at: &str, host: &str, plain: bool) -> String {
    use rustls::{CertificateError as C, Error as R};
    match e {
        R::InvalidCertificate(C::UnknownIssuer) => "Home Assistant's certificate isn't from an \
            authority PerritoDeck trusts (is it self-signed?). Use its http:// address on your \
            network, or give Home Assistant a certificate from an authority such as Let's \
            Encrypt."
            .into(),
        R::InvalidCertificate(C::NotValidForName | C::NotValidForNameContext { .. }) => format!(
            "Home Assistant's certificate is for a different name than {host}. Use the name \
             the certificate was made for."
        ),
        R::InvalidCertificate(C::Expired | C::ExpiredContext { .. }) => {
            "Home Assistant's certificate has expired. Renew it in Home Assistant.".into()
        }
        R::InvalidMessage(_) if !plain => format!(
            "{at} doesn't answer with https. If Home Assistant doesn't use https, start the \
             address with http://."
        ),
        other => format!("The secure connection to Home Assistant failed: {other}"),
    }
}

/// What Test connection learned from Home Assistant's `/api/config`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct About {
    pub version: Option<String>,
    pub location_name: Option<String>,
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
        // Redirects aren't followed: the token wouldn't go with them, so a right token
        // would read as rejected, and another server would answer for Home Assistant.
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(8)))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .into();
        Self { config, agent }
    }

    /// A client for Test connection: the keys' 8 s for the whole request, and `connect`
    /// for the connection alone, so that "nothing answered" can be told apart from a
    /// slow answer.
    pub(crate) fn for_test(config: HaConfig, connect: Duration) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(8)))
            .timeout_connect(Some(connect))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .into();
        Self { config, agent }
    }

    /// `GET /api/config`, with the token as keys send it.
    pub(crate) async fn probe(&self) -> Result<About, Problem> {
        let url = format!("{}/api/config", self.config.url.trim_end_matches('/'));
        let agent = self.agent.clone();
        let token = self.config.token.clone();
        tokio::task::spawn_blocking(move || {
            let mut resp = agent
                .get(&url)
                .header("Authorization", &format!("Bearer {token}"))
                .call()
                .map_err(Problem::Http)?;
            match resp.status().as_u16() {
                200 => {
                    let config: Value =
                        resp.body_mut().read_json().map_err(|_| Problem::NotJson)?;
                    if !config.is_object() {
                        return Err(Problem::NotJson);
                    }
                    // Whatever answered chooses these; they go into the form's message.
                    let text = |key: &str| {
                        config[key]
                            .as_str()
                            .map(crate::check::shown)
                            .filter(|s| !s.is_empty())
                    };
                    Ok(About {
                        version: text("version"),
                        location_name: text("location_name"),
                    })
                }
                code @ 300..=399 => Err(Problem::Redirect {
                    code,
                    to: resp
                        .headers()
                        .get("location")
                        .and_then(|l| l.to_str().ok())
                        .map(|l| crate::check::shown_up_to(l, 120))
                        .filter(|s| !s.is_empty()),
                }),
                code => Err(Problem::Status(code)),
            }
        })
        .await
        .map_err(|e| Problem::Http(ureq::Error::Io(std::io::Error::other(e.to_string()))))?
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
                401 => Err(err("the token in Settings → Integrations was rejected")),
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
                401 => Err(err("the token in Settings → Integrations was rejected")),
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
        fake_ha_answering(status, "[]")
    }

    /// One-shot HTTP server that records the request and answers `status` with `answer`.
    fn fake_ha_answering(
        status: u16,
        answer: &'static str,
    ) -> (String, mpsc::Receiver<(String, String, String)>) {
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
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{answer}",
                answer.len()
            );
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
            enabled: true,
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
            enabled: true,
        })
        .call("light.toggle", None, None)
        .await
        .unwrap_err();
        assert_eq!(
            e.to_string(),
            "Home Assistant: the token in Settings → Integrations was rejected"
        );
        let (url, _seen2) = fake_ha(400);
        let out = HaClient::new(HaConfig {
            url,
            token: "x".into(),
            enabled: true,
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
            enabled: true,
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
            enabled: true,
        });
        assert!(ha
            .call("light.toggle", None, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("can't reach"));
    }

    // ---- Test connection (the form) ----

    use crate::check::{self, tests::resolver, Checked};

    const WAIT: Duration = Duration::from_millis(300);

    fn cfg(url: &str, token: &str) -> HaConfig {
        HaConfig {
            url: url.to_owned(),
            token: token.to_owned(),
            enabled: true,
        }
    }

    fn any_name() -> check::Resolve {
        resolver(Ok(vec!["127.0.0.1:8123".parse().unwrap()]))
    }

    async fn test(url: &str, token: &str) -> Checked {
        check::home_assistant(&cfg(url, token), WAIT, &any_name()).await
    }

    #[tokio::test]
    async fn check_reads_the_version_with_the_bearer_token() {
        let (url, seen) =
            fake_ha_answering(200, r#"{"version":"2026.10.1","location_name":"Home"}"#);
        assert_eq!(
            test(&url, "abc").await,
            Checked::ok("Connected to Home Assistant 2026.10.1 (Home).")
        );
        let (line, auth, _) = seen.recv().unwrap();
        assert_eq!(line, "GET /api/config HTTP/1.1");
        assert_eq!(auth, "Bearer abc");
        let (url, _seen) = fake_ha_answering(200, r#"{"version":"2026.10.1"}"#);
        assert_eq!(
            test(&url, "abc").await,
            Checked::ok("Connected to Home Assistant 2026.10.1.")
        );
        let (url, _seen) = fake_ha_answering(200, "{}");
        assert_eq!(
            test(&url, "abc").await,
            Checked::ok("Connected to Home Assistant.")
        );
    }

    #[tokio::test]
    async fn check_401_403_404_and_not_json() {
        let (url, _seen) = fake_ha(401);
        assert_eq!(
            test(&url, "x").await,
            Checked::failed(
                "Home Assistant rejected the token. Make a new long-lived access token (your \
                 profile in Home Assistant → Security) and paste it here."
            )
        );
        let (url, _seen) = fake_ha(403);
        assert_eq!(
            test(&url, "x").await,
            Checked::failed(
                "Home Assistant refused the token (403). It may have blocked this computer \
                 after too many wrong tokens; Home Assistant's log says so if it did."
            )
        );
        let (url, _seen) = fake_ha(404);
        assert_eq!(
            test(&url, "x").await,
            Checked::failed(format!(
                "Something answered at {url}, but it isn't Home Assistant (404). Use the \
                 address you open Home Assistant with, like http://homeassistant.local:8123."
            ))
        );
        let (url, _seen) = fake_ha(500);
        assert_eq!(
            test(&url, "x").await,
            Checked::failed("Home Assistant answered 500.")
        );
        let (url, _seen) = fake_ha_answering(200, "<html>");
        assert_eq!(
            test(&url, "x").await,
            Checked::failed(format!(
                "Something answered at {url}, but not like Home Assistant."
            ))
        );
        let (url, _seen) = fake_ha(200);
        assert_eq!(
            test(&url, "x").await,
            Checked::failed(format!(
                "Something answered at {url}, but not like Home Assistant."
            )),
            "a list isn't Home Assistant's config"
        );
    }

    #[tokio::test]
    async fn check_refused() {
        // Windows takes about 2 s to give up on a port nobody listens on.
        let got = check::home_assistant(
            &cfg("http://127.0.0.1:1", "x"),
            check::TEST_WAIT,
            &any_name(),
        )
        .await;
        assert_eq!(
            got,
            Checked::failed(
                "Nothing answered at 127.0.0.1:1. Is Home Assistant running there? Its usual \
                 port is 8123."
            )
        );
    }

    const HTTPS_HINT: &str =
        "answered, but not like Home Assistant. If Home Assistant uses https, start the \
         address with https://.";

    /// A TLS server with a self-signed certificate, like a Home Assistant with its own.
    async fn tls_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = perritodeck_core::HostIdentity::generate().tls_acceptor();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let _ = acceptor.accept(stream).await;
                });
            }
        });
        addr.to_string()
    }

    #[tokio::test]
    async fn check_http_to_a_tls_port_two_ways() {
        // A TLS server that answers plain HTTP with an alert.
        let at = tls_server().await;
        let got = test(&format!("http://{at}"), "x").await;
        assert_eq!(
            got,
            Checked::failed(format!("Something at {at} {HTTPS_HINT}"))
        );
        // One that hangs up without a word.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let at = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            let _ = BufReader::new(&stream).read_line(&mut line);
        });
        let got = test(&format!("http://{at}"), "x").await;
        assert_eq!(
            got,
            Checked::failed(format!("Something at {at} {HTTPS_HINT}"))
        );
    }

    /// nginx in front of Home Assistant answers plain HTTP on its https port with 400.
    #[tokio::test]
    async fn check_400_on_http_says_https() {
        let (url, _seen) = fake_ha(400);
        let at = url.trim_start_matches("http://").to_owned();
        assert_eq!(
            test(&url, "x").await,
            Checked::failed(format!("Something at {at} {HTTPS_HINT}"))
        );
    }

    #[tokio::test]
    async fn check_https_to_plain_port() {
        // A plain web server answers a TLS hello with HTTP.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let at = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
            let _ = stream.read(&mut [0u8; 1024]);
        });
        let got = test(&format!("https://{at}"), "x").await;
        assert_eq!(
            got,
            Checked::failed(format!(
                "{at} doesn't answer with https. If Home Assistant doesn't use https, start \
                 the address with http://."
            ))
        );
    }

    #[tokio::test]
    async fn check_self_signed_certificate() {
        let at = tls_server().await;
        let got = test(&format!("https://{at}"), "x").await;
        assert_eq!(
            got,
            Checked::failed(
                "Home Assistant's certificate isn't from an authority PerritoDeck trusts (is \
                 it self-signed?). Use its http:// address on your network, or give Home \
                 Assistant a certificate from an authority such as Let's Encrypt."
            )
        );
    }

    /// Home Assistant took the connection but is slow to answer (it may be starting):
    /// that isn't "nothing answered".
    #[test]
    fn a_slow_answer_is_not_no_answer() {
        let slow = Problem::Http(ureq::Error::Timeout(ureq::Timeout::RecvResponse));
        assert_eq!(
            slow.for_form("http://ha:8123", "ha:8123", "ha", WAIT),
            "Home Assistant at ha:8123 took the connection but didn't answer within 8 \
             seconds. It may still be starting."
        );
        let none = Problem::Http(ureq::Error::Timeout(ureq::Timeout::Connect));
        assert_eq!(
            none.for_form("http://ha:8123", "ha:8123", "ha", WAIT),
            check::no_answer("ha:8123", WAIT)
        );
    }

    #[tokio::test]
    async fn check_unknown_host_with_fake_resolvers() {
        let none = resolver(Err(std::io::ErrorKind::NotFound.into()));
        let got =
            check::home_assistant(&cfg("http://homeassistant.local:8123", "x"), WAIT, &none).await;
        assert_eq!(
            got,
            Checked::failed(
                "Can't find a computer called “homeassistant.local”. Check the name, or use \
                 its IP address."
            )
        );
    }

    /// A one-shot server that answers with `reply` and says whether anyone came.
    fn answering(reply: String) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
            }
            let _ = tx.send(first.trim().to_owned());
            stream.write_all(reply.as_bytes()).unwrap();
        });
        (url, rx)
    }

    /// A redirect isn't followed: the token wouldn't go with it, so a right token would
    /// read as rejected, or another server's 200 as "Connected". The form says where
    /// Home Assistant sends you instead, and keys don't follow either.
    #[tokio::test]
    async fn redirects_are_named_not_followed() {
        let (target, reached) = answering(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}"
                .into(),
        );
        let (url, _seen) = answering(format!(
            "HTTP/1.1 301 Moved Permanently\r\nLocation: {target}/api/config\r\nContent-Length: 0\r\n\r\n"
        ));
        let at = url.trim_start_matches("http://").to_owned();
        assert_eq!(
            test(&url, "good").await,
            Checked::failed(format!(
                "Home Assistant at {at} sends you to {target}/api/config. Use that address \
                 instead."
            ))
        );
        let (url, _seen) = answering(format!(
            "HTTP/1.1 302 Found\r\nLocation: {target}/api/services/light/toggle\r\nContent-Length: 0\r\n\r\n"
        ));
        let out = HaClient::new(cfg(&url, "good"))
            .call("light.toggle", None, None)
            .await
            .unwrap();
        assert_eq!(out, Outcome::Alert("Home Assistant answered 302".into()));
        assert!(
            reached.recv_timeout(Duration::from_millis(300)).is_err(),
            "the redirect was followed"
        );
    }

    /// What a server says about itself goes into the message trimmed, without control
    /// characters, and only so long.
    #[tokio::test]
    async fn what_home_assistant_says_about_itself_is_trimmed() {
        let long = "x".repeat(200);
        let body = format!(r#"{{"version":"2026.10\u001b[31m","location_name":"{long}"}}"#);
        let (url, _seen) = answering(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ));
        let got = test(&url, "abc").await;
        assert!(got.ok);
        assert!(!got.message.contains('\u{1b}'), "{}", got.message);
        assert!(got.message.chars().count() < 120, "{}", got.message);
        assert!(
            got.message
                .starts_with("Connected to Home Assistant 2026.10[31m (xxxxxxxx"),
            "{}",
            got.message
        );
        assert!(got.message.ends_with("…)."), "{}", got.message);
    }
}
