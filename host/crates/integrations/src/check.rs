//! Test connection in Settings → Integrations: the real clients' handshake, run on what's
//! typed in the form, with answers worded for a person rather than a key.
//!
//! A test never touches the running clients: it builds its own, from the form's values.

use std::{
    io,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use perritodeck_actions::BoxFuture;
use serde::Serialize;

use crate::{home_assistant::HaConfig, obs::ObsConfig};

/// How long a test waits for each step: the name lookup, the connection, each answer.
pub const TEST_WAIT: Duration = Duration::from_secs(5);
/// The longest a whole test may take.
pub const TEST_LIMIT: Duration = Duration::from_secs(15);

/// What a test came to, as the form shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checked {
    pub ok: bool,
    pub message: String,
}

impl Checked {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
        }
    }
}

/// Looks a name up; tests swap in their own.
pub type Resolve =
    Arc<dyn Fn(String, u16) -> BoxFuture<'static, io::Result<Vec<SocketAddr>>> + Send + Sync>;

/// The system's own lookup.
pub fn system_resolver() -> Resolve {
    Arc::new(|host, port| {
        Box::pin(async move {
            Ok(tokio::net::lookup_host((host.as_str(), port))
                .await?
                .collect())
        })
    })
}

/// Whether `host` is a name that resolves, within `wait`. An IP address needs no lookup.
pub async fn find_host(
    host: &str,
    port: u16,
    wait: Duration,
    resolve: &Resolve,
) -> Result<(), String> {
    if is_ip(host) {
        return Ok(());
    }
    let host = bare(host);
    match tokio::time::timeout(wait, resolve(host.to_owned(), port)).await {
        Ok(Ok(found)) if !found.is_empty() => Ok(()),
        Ok(_) => Err(format!(
            "Can't find a computer called “{host}”. Check the name, or use its IP address."
        )),
        Err(_) => Err(format!(
            "Looking up “{host}” took more than {}. Check the name, or use its IP address.",
            secs(wait)
        )),
    }
}

/// `host:port`, with an IPv6 address in brackets.
pub fn at(host: &str, port: u16) -> String {
    let host = bare(host);
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// "5 seconds", "1 second", "0.3 seconds".
pub fn secs(d: Duration) -> String {
    let ms = d.as_millis();
    if ms == 1000 {
        "1 second".into()
    } else if ms % 1000 == 0 {
        format!("{} seconds", ms / 1000)
    } else {
        let text = format!("{:.2}", d.as_secs_f64());
        format!("{} seconds", text.trim_end_matches('0'))
    }
}

pub fn no_answer(at: &str, wait: Duration) -> String {
    format!(
        "Nothing answered at {at} within {}. Check the address, and that a firewall isn't \
         blocking it.",
        secs(wait)
    )
}

pub fn unreachable(at: &str) -> String {
    format!(
        "This computer can't reach {at}. Check the address, and that both computers are on \
         the same network."
    )
}

/// `host` without the brackets around an IPv6 address.
pub(crate) fn bare(host: &str) -> &str {
    host.trim_start_matches('[').trim_end_matches(']')
}

pub(crate) fn is_ip(host: &str) -> bool {
    bare(host).parse::<IpAddr>().is_ok()
}

/// `check`, stopped after [`TEST_LIMIT`].
pub async fn limited(check: impl std::future::Future<Output = Checked>) -> Checked {
    within(TEST_LIMIT, check).await
}

async fn within(limit: Duration, check: impl std::future::Future<Output = Checked>) -> Checked {
    tokio::time::timeout(limit, check)
        .await
        .unwrap_or_else(|_| {
            Checked::failed(format!(
                "The test took more than {}, so it was stopped.",
                secs(limit)
            ))
        })
}

/// Test OBS at the form's address: connect, greet with the password, ask its version.
/// `given_password` says whether one was typed or kept, to mention it when OBS doesn't
/// ask for one.
pub async fn obs(
    cfg: &ObsConfig,
    given_password: bool,
    wait: Duration,
    resolve: &Resolve,
) -> Checked {
    limited(async {
        let url = cfg.url.trim();
        match url.split_once("://") {
            Some((scheme, _)) if scheme.eq_ignore_ascii_case("wss") => {
                return Checked::failed(crate::file::OBS_WSS)
            }
            Some((scheme, _)) if scheme.eq_ignore_ascii_case("ws") => {}
            _ => return Checked::failed(crate::file::OBS_SCHEME),
        }
        let address = match crate::file::split_address(url) {
            Ok(a) => a,
            Err(m) => return Checked::failed(m),
        };
        let port = address.port.unwrap_or(4455);
        let at = at(&address.host, port);
        if let Err(m) = find_host(&address.host, port, wait, resolve).await {
            return Checked::failed(m);
        }
        match crate::obs::ObsClient::probe(cfg.clone(), wait).await {
            Ok(p) => {
                let mut message = match (p.obs_version, p.ws_version) {
                    (Some(o), Some(w)) => format!("Connected to OBS {o} (obs-websocket {w})."),
                    (Some(o), None) => format!("Connected to OBS {o}."),
                    (None, Some(w)) => format!("Connected to OBS (obs-websocket {w})."),
                    (None, None) => "Connected to OBS.".to_owned(),
                };
                if given_password && !p.auth_required {
                    message
                        .push_str(" OBS doesn't ask for a password, so the one here isn't used.");
                }
                Checked::ok(message)
            }
            Err(p) => Checked::failed(p.for_form(&at, wait)),
        }
    })
    .await
}

/// Test Home Assistant at the form's address: `GET /api/config` with the token, as keys
/// send it.
pub async fn home_assistant(cfg: &HaConfig, wait: Duration, resolve: &Resolve) -> Checked {
    limited(async {
        let base = cfg.url.trim().trim_end_matches('/');
        let address = match crate::file::split_address(base) {
            Ok(a) => a,
            Err(m) => return Checked::failed(m),
        };
        let port = address
            .port
            .unwrap_or(if address.scheme == "https" { 443 } else { 80 });
        let at = at(&address.host, port);
        if let Err(m) = find_host(&address.host, port, wait, resolve).await {
            return Checked::failed(m);
        }
        let client = crate::home_assistant::HaClient::for_test(cfg.clone(), wait);
        match client.probe().await {
            Ok(about) => Checked::ok(match (about.version, about.location_name) {
                (Some(v), Some(l)) => format!("Connected to Home Assistant {v} ({l})."),
                (Some(v), None) => format!("Connected to Home Assistant {v}."),
                _ => "Connected to Home Assistant.".to_owned(),
            }),
            Err(p) => Checked::failed(p.for_form(base, &at, address.bare_host(), wait)),
        }
    })
    .await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A lookup that answers `answer` for every name.
    pub(crate) fn resolver(answer: io::Result<Vec<SocketAddr>>) -> Resolve {
        let answer = Arc::new(answer.map_err(|e| e.kind()));
        Arc::new(move |_, _| {
            let answer = answer.clone();
            Box::pin(async move { answer.as_ref().clone().map_err(io::Error::from) })
        })
    }

    /// A lookup that never answers in time.
    pub(crate) fn slow_resolver() -> Resolve {
        Arc::new(|_, _| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(Vec::new())
            })
        })
    }

    const WAIT: Duration = Duration::from_millis(300);

    #[tokio::test]
    async fn names_that_dont_resolve_say_so() {
        let cant =
            "Can't find a computer called “studio-pc”. Check the name, or use its IP address.";
        let none = resolver(Err(io::ErrorKind::NotFound.into()));
        assert_eq!(
            find_host("studio-pc", 4455, WAIT, &none).await,
            Err(cant.into())
        );
        let empty = resolver(Ok(Vec::new()));
        assert_eq!(
            find_host("studio-pc", 4455, WAIT, &empty).await,
            Err(cant.into())
        );
        assert_eq!(
            find_host("studio-pc", 4455, WAIT, &slow_resolver()).await,
            Err(
                "Looking up “studio-pc” took more than 0.3 seconds. Check the name, or use \
                 its IP address."
                    .into()
            )
        );
        let found = resolver(Ok(vec!["192.168.1.5:4455".parse().unwrap()]));
        assert_eq!(find_host("studio-pc", 4455, WAIT, &found).await, Ok(()));
        // Addresses aren't looked up at all.
        for ip in ["127.0.0.1", "[::1]", "::1"] {
            assert_eq!(find_host(ip, 4455, WAIT, &none).await, Ok(()), "{ip}");
        }
    }

    /// The real lookup, for a name that can't exist. Off by default: it needs a network
    /// and the system's resolver.
    #[tokio::test]
    #[ignore]
    async fn real_dns_unknown_host() {
        let e = find_host("no-such-host.invalid", 80, TEST_WAIT, &system_resolver())
            .await
            .unwrap_err();
        assert!(e.starts_with("Can't find a computer called"), "{e}");
    }

    #[test]
    fn addresses_and_waits_read_naturally() {
        assert_eq!(at("studio-pc", 4455), "studio-pc:4455");
        assert_eq!(at("::1", 4455), "[::1]:4455");
        assert_eq!(at("[::1]", 4455), "[::1]:4455");
        assert_eq!(secs(Duration::from_secs(5)), "5 seconds");
        assert_eq!(secs(Duration::from_secs(1)), "1 second");
        assert_eq!(secs(Duration::from_millis(300)), "0.3 seconds");
        assert_eq!(secs(Duration::from_millis(1500)), "1.5 seconds");
        assert_eq!(
            no_answer("pc:1", WAIT),
            "Nothing answered at pc:1 within 0.3 seconds. Check the address, and that a \
             firewall isn't blocking it."
        );
    }

    #[tokio::test]
    async fn a_test_that_runs_too_long_is_stopped() {
        let stuck = async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Checked::ok("never")
        };
        assert_eq!(
            within(Duration::from_millis(200), stuck).await,
            Checked::failed("The test took more than 0.2 seconds, so it was stopped.")
        );
        assert_eq!(
            limited(async { Checked::ok("fine") }).await,
            Checked::ok("fine")
        );
        assert_eq!(secs(TEST_LIMIT), "15 seconds");
    }
}
