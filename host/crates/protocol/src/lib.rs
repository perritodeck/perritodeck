//! PerritoDeck wire protocol, v1.
//!
//! The source of truth is `protocol/schema/v1.schema.json`; the golden files in
//! `protocol/fixtures/` are decoded and re-encoded by this crate's tests.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol major version spoken by this crate.
pub const PROTOCOL_VERSION: u32 = 1;

/// Default TCP port for the host's WebSocket server.
pub const DEFAULT_PORT: u16 = 46710;

/// Message types this crate understands. Anything else decodes as [`Decoded::UnknownType`].
pub const KNOWN_TYPES: &[&str] = &[
    "hello",
    "welcome",
    "layout",
    "keyDown",
    "keyUp",
    "keyAck",
    "heartbeat",
    "heartbeatAck",
    "setTitle",
    "setImage",
    "error",
];

/// One message as it travels in a WebSocket text frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    #[serde(flatten)]
    pub msg: Message,
}

impl Envelope {
    pub fn new(msg: Message) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: None,
            msg,
        }
    }

    pub fn with_id(msg: Message, id: Option<u32>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            msg,
        }
    }

    /// Serialise to the JSON text sent in a frame.
    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("protocol types always serialise")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum Message {
    Hello(Hello),
    Welcome(Welcome),
    Layout(Layout),
    KeyDown(KeyEvent),
    KeyUp(KeyEvent),
    KeyAck(KeyAck),
    Heartbeat(Heartbeat),
    HeartbeatAck(Heartbeat),
    SetTitle(SetTitle),
    SetImage(SetImage),
    Error(ErrorMessage),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub device_id: String,
    pub device_name: String,
    pub app_version: String,
    pub platform: Platform,
    pub os_version: String,
    pub screen: Screen,
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_grid: Option<Grid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Android,
    Web,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    pub width_px: u32,
    pub height_px: u32,
    pub density_dpi: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grid {
    pub cols: u32,
    pub rows: u32,
}

impl Grid {
    /// Largest grid either side accepts.
    pub const MAX_DIMENSION: u32 = 32;

    pub fn is_valid(&self) -> bool {
        (1..=Self::MAX_DIMENSION).contains(&self.cols)
            && (1..=Self::MAX_DIMENSION).contains(&self.rows)
    }

    pub fn key_count(&self) -> u32 {
        self.cols * self.rows
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Welcome {
    pub host_name: String,
    pub host_version: String,
    pub protocol_version: u32,
    pub heartbeat_interval_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub cols: u32,
    pub rows: u32,
    pub keys: Vec<KeySpec>,
}

impl Layout {
    pub fn grid(&self) -> Grid {
        Grid {
            cols: self.cols,
            rows: self.rows,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeySpec {
    pub index: u32,
    pub title: String,
    pub bg_color: String,
    pub text_color: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyEvent {
    pub key_index: u32,
    pub client_timestamp_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyAck {
    pub key_index: u32,
    pub host_processing_us: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Heartbeat {
    pub seq: u32,
    pub sent_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTitle {
    pub key_index: u32,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetImage {
    pub key_index: u32,
    /// PNG/WebP data URL, or `None` to clear. Always serialised (as `null` when cleared).
    pub image: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorMessage {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Unauthorized,
    UnsupportedVersion,
    BadMessage,
    ProtocolViolation,
    Internal,
}

impl Message {
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Message::Error(ErrorMessage {
            code,
            message: message.into(),
        })
    }

    /// The `type` string this message is sent with.
    pub fn type_name(&self) -> &'static str {
        match self {
            Message::Hello(_) => "hello",
            Message::Welcome(_) => "welcome",
            Message::Layout(_) => "layout",
            Message::KeyDown(_) => "keyDown",
            Message::KeyUp(_) => "keyUp",
            Message::KeyAck(_) => "keyAck",
            Message::Heartbeat(_) => "heartbeat",
            Message::HeartbeatAck(_) => "heartbeatAck",
            Message::SetTitle(_) => "setTitle",
            Message::SetImage(_) => "setImage",
            Message::Error(_) => "error",
        }
    }
}

/// Result of decoding a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Decoded {
    Message(Envelope),
    /// A well-formed envelope with a `type` this version doesn't know. Peers must ignore these.
    UnknownType {
        type_name: String,
        id: Option<u32>,
    },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DecodeError {
    #[error("frame is not a JSON object with numeric `v` and string `type`")]
    NotAnEnvelope,
    #[error("unsupported protocol version {0} (this peer speaks {PROTOCOL_VERSION})")]
    UnsupportedVersion(u64),
    #[error("invalid `{type_name}` message: {reason}")]
    InvalidPayload { type_name: String, reason: String },
}

/// Decode one text frame.
pub fn decode(text: &str) -> Result<Decoded, DecodeError> {
    let value: Value = serde_json::from_str(text).map_err(|_| DecodeError::NotAnEnvelope)?;
    let obj = value.as_object().ok_or(DecodeError::NotAnEnvelope)?;
    let v = obj
        .get("v")
        .and_then(Value::as_u64)
        .ok_or(DecodeError::NotAnEnvelope)?;
    if v != u64::from(PROTOCOL_VERSION) {
        return Err(DecodeError::UnsupportedVersion(v));
    }
    let type_name = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or(DecodeError::NotAnEnvelope)?
        .to_owned();
    if !KNOWN_TYPES.contains(&type_name.as_str()) {
        let id = obj
            .get("id")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok());
        return Ok(Decoded::UnknownType { type_name, id });
    }
    serde_json::from_value::<Envelope>(value)
        .map(Decoded::Message)
        .map_err(|e| DecodeError::InvalidPayload {
            type_name,
            reason: e.to_string(),
        })
}

/// True when `s` is a `#RRGGBB` colour.
pub fn is_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_types_are_reported_not_rejected() {
        let d = decode(r#"{"v":1,"type":"dialRotate","id":9,"payload":{"ticks":2}}"#).unwrap();
        assert_eq!(
            d,
            Decoded::UnknownType {
                type_name: "dialRotate".into(),
                id: Some(9)
            }
        );
    }

    #[test]
    fn other_major_versions_are_rejected() {
        assert_eq!(
            decode(r#"{"v":2,"type":"hello","payload":{}}"#),
            Err(DecodeError::UnsupportedVersion(2))
        );
    }

    #[test]
    fn garbage_is_not_an_envelope() {
        assert_eq!(decode("nope"), Err(DecodeError::NotAnEnvelope));
        assert_eq!(decode("[1,2]"), Err(DecodeError::NotAnEnvelope));
        assert_eq!(
            decode(r#"{"type":"hello"}"#),
            Err(DecodeError::NotAnEnvelope)
        );
    }

    #[test]
    fn bad_payload_names_the_type() {
        let err = decode(r#"{"v":1,"type":"keyDown","payload":{"keyIndex":"x"}}"#).unwrap_err();
        assert!(
            matches!(err, DecodeError::InvalidPayload { type_name, .. } if type_name == "keyDown")
        );
    }

    #[test]
    fn type_name_matches_serialised_tag() {
        let m = Message::HeartbeatAck(Heartbeat {
            seq: 1,
            sent_at_ms: 2,
        });
        let v: Value = serde_json::from_str(&Envelope::new(m.clone()).encode()).unwrap();
        assert_eq!(v["type"], m.type_name());
    }

    #[test]
    fn colors() {
        assert!(is_color("#1E1e2E"));
        assert!(!is_color("1E1E2E"));
        assert!(!is_color("#1E1E2"));
        assert!(!is_color("#GGGGGG"));
    }
}
