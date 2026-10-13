//! A minimal PerritoDeck client, used by the `perritodeck-fakedeck` CLI and the
//! host's end-to-end tests. Speaks the same TLS + pinned-fingerprint + pairing flow
//! as the Android app.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures_util::{SinkExt, StreamExt};
use perritodeck_protocol::{
    decode, Decoded, Envelope, Grid, Hello, KeyEvent, Layout, Message, Pair, Paired, Platform,
    Screen, Welcome,
};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::{
    client_async,
    tungstenite::{self, client::IntoClientRequest},
    WebSocketStream,
};

/// Fingerprint the TLS verifier saw, filled in during the handshake.
type SeenFingerprint = Arc<Mutex<Option<String>>>;

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// How to secure a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientSecurity {
    /// Plain `ws://` (host started with `--insecure`).
    Insecure,
    /// TLS; the host's certificate must have this fingerprint.
    Pinned(String),
    /// TLS; accept any certificate and report its fingerprint (first contact / pairing).
    TrustOnFirstUse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// `host:port`
    pub addr: String,
    pub security: ClientSecurity,
}

pub struct Client {
    ws: WebSocketStream<Box<dyn Io>>,
    next_id: u32,
    /// Fingerprint of the certificate the host presented (TLS only).
    pub host_fingerprint: Option<String>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn fingerprint_of(der: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(der))
}

/// A `hello` describing a 10" landscape tablet.
pub fn fake_hello(device_id: &str, device_secret: Option<String>, grid: Option<Grid>) -> Hello {
    Hello {
        device_id: device_id.to_owned(),
        device_name: "fakedeck".to_owned(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        platform: Platform::Other,
        os_version: std::env::consts::OS.to_owned(),
        screen: Screen {
            width_px: 1920,
            height_px: 1200,
            density_dpi: 224,
        },
        capabilities: vec!["keys".to_owned()],
        requested_grid: grid,
        token: None,
        device_secret,
        network_failed: false,
    }
}

/// Pins (or records) the host certificate instead of checking a CA chain.
#[derive(Debug)]
struct PinVerifier {
    expected: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = fingerprint_of(end_entity);
        *self.seen.lock().expect("poisoned") = Some(fp.clone());
        match &self.expected {
            Some(exp) if *exp != fp => Err(rustls::Error::General(format!(
                "host certificate fingerprint {fp} does not match the paired host {exp}"
            ))),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

impl Client {
    pub async fn connect(ep: &Endpoint) -> Result<Self> {
        Self::connect_with(ep, |_| {}).await
    }

    /// Connect, letting the caller adjust the HTTP upgrade request (tests use this to add headers).
    pub async fn connect_with(
        ep: &Endpoint,
        tweak: impl FnOnce(&mut tungstenite::handshake::client::Request),
    ) -> Result<Self> {
        let tcp = TcpStream::connect(&ep.addr)
            .await
            .with_context(|| format!("connecting to {}", ep.addr))?;
        let _ = tcp.set_nodelay(true);
        let (stream, scheme, seen): (Box<dyn Io>, &str, Option<SeenFingerprint>) =
            match &ep.security {
                ClientSecurity::Insecure => (Box::new(tcp), "ws", None),
                sec => {
                    let provider = Arc::new(rustls::crypto::ring::default_provider());
                    let seen = Arc::new(Mutex::new(None));
                    let verifier = PinVerifier {
                        expected: match sec {
                            ClientSecurity::Pinned(fp) => Some(fp.clone()),
                            _ => None,
                        },
                        seen: seen.clone(),
                        provider: provider.clone(),
                    };
                    let config = rustls::ClientConfig::builder_with_provider(provider)
                        .with_safe_default_protocol_versions()?
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(verifier))
                        .with_no_client_auth();
                    let name = ServerName::try_from("perritodeck-host").expect("valid DNS name");
                    let tls = TlsConnector::from(Arc::new(config))
                        .connect(name, tcp)
                        .await
                        .context("TLS handshake with the host failed")?;
                    (Box::new(tls), "wss", Some(seen))
                }
            };
        let mut request = format!("{scheme}://{}/", ep.addr).into_client_request()?;
        tweak(&mut request);
        let (ws, _) = client_async(request, stream).await?;
        let host_fingerprint = seen.and_then(|s| s.lock().expect("poisoned").clone());
        Ok(Self {
            ws,
            next_id: 1,
            host_fingerprint,
        })
    }

    /// Connect, send `hello`, and wait for `welcome` + `layout`.
    pub async fn handshake(ep: &Endpoint, hello: Hello) -> Result<(Self, Welcome, Layout)> {
        let mut c = Self::connect(ep).await?;
        c.send(Message::Hello(hello)).await?;
        let welcome = match c.recv_msg(Duration::from_secs(5)).await?.msg {
            Message::Welcome(w) => w,
            Message::Error(e) => bail!("host refused connection ({:?}): {}", e.code, e.message),
            other => bail!("expected welcome, got {}", other.type_name()),
        };
        let layout = match c.recv_msg(Duration::from_secs(5)).await?.msg {
            Message::Layout(l) => l,
            other => bail!("expected layout, got {}", other.type_name()),
        };
        Ok((c, welcome, layout))
    }

    /// Pair with a one-time code. Returns the issued secret and the host's fingerprint.
    pub async fn pair(
        ep: &Endpoint,
        code: &str,
        device_id: &str,
        device_name: &str,
    ) -> Result<Paired> {
        let mut c = Self::connect(ep).await?;
        c.send(Message::Pair(Pair {
            code: code.to_owned(),
            device_id: device_id.to_owned(),
            device_name: device_name.to_owned(),
        }))
        .await?;
        let paired = match c.recv_msg(Duration::from_secs(5)).await?.msg {
            Message::Paired(p) => p,
            Message::Error(e) => bail!("pairing failed ({:?}): {}", e.code, e.message),
            other => bail!("expected paired, got {}", other.type_name()),
        };
        if let Some(seen) = &c.host_fingerprint {
            if *seen != paired.host_id {
                bail!(
                    "host id {} does not match its certificate {seen}",
                    paired.host_id
                );
            }
        }
        c.close().await;
        Ok(paired)
    }

    /// Send a message with a fresh correlation id; returns the id.
    pub async fn send(&mut self, msg: Message) -> Result<u32> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.send_raw(&Envelope::with_id(msg, Some(id)).encode())
            .await?;
        Ok(id)
    }

    pub async fn send_raw(&mut self, text: &str) -> Result<()> {
        self.ws.send(tungstenite::Message::text(text)).await?;
        Ok(())
    }

    /// Next decoded frame, or `None` once the socket closes.
    pub async fn recv(&mut self) -> Result<Option<Decoded>> {
        while let Some(frame) = self.ws.next().await {
            match frame {
                Ok(tungstenite::Message::Text(t)) => return Ok(Some(decode(t.as_str())?)),
                Ok(tungstenite::Message::Close(_)) => return Ok(None),
                Ok(_) => continue,
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    return Ok(None)
                }
                Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None)
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(None)
    }

    /// Next known message within `limit`; errors on close, timeout or unknown types.
    pub async fn recv_msg(&mut self, limit: Duration) -> Result<Envelope> {
        match timeout(limit, self.recv())
            .await
            .map_err(|_| anyhow!("timed out waiting for a message"))??
        {
            Some(Decoded::Message(env)) => Ok(env),
            Some(Decoded::UnknownType { type_name, .. }) => {
                bail!("unexpected unknown type {type_name}")
            }
            None => bail!("connection closed"),
        }
    }

    pub async fn key_down(&mut self, key_index: u32) -> Result<u32> {
        self.send(Message::KeyDown(KeyEvent {
            key_index,
            client_timestamp_ms: now_ms(),
        }))
        .await
    }

    pub async fn key_up(&mut self, key_index: u32) -> Result<u32> {
        self.send(Message::KeyUp(KeyEvent {
            key_index,
            client_timestamp_ms: now_ms(),
        }))
        .await
    }

    pub async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}
