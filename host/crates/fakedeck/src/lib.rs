//! A minimal PerritoDeck client, used by the `perritodeck-fakedeck` CLI and the
//! host's end-to-end tests.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use perritodeck_protocol::{
    decode, Decoded, Envelope, Grid, Hello, KeyEvent, Layout, Message, Platform, Screen, Welcome,
};
use tokio::{net::TcpStream, time::timeout};
use tokio_tungstenite::{connect_async, tungstenite, MaybeTlsStream, WebSocketStream};

pub struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: u32,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A `hello` describing a 10" landscape tablet.
pub fn fake_hello(device_id: &str, token: Option<String>, grid: Option<Grid>) -> Hello {
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
        token,
    }
}

impl Client {
    pub async fn connect(url: &str) -> Result<Self> {
        let (ws, _) = connect_async(url)
            .await
            .with_context(|| format!("connecting to {url}"))?;
        Ok(Self { ws, next_id: 1 })
    }

    /// Connect, send `hello`, and wait for `welcome` + `layout`.
    pub async fn handshake(url: &str, hello: Hello) -> Result<(Self, Welcome, Layout)> {
        let mut c = Self::connect(url).await?;
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
