//! JSON-RPC 2.0 pieces: error codes and response objects.

use serde_json::{json, Value};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// MCP 2026-07-28: HTTP headers don't match the body, or required ones are missing.
pub const HEADER_MISMATCH: i64 = -32020;
/// MCP 2026-07-28: the requested protocol version isn't supported.
pub const UNSUPPORTED_VERSION: i64 = -32022;

/// A JSON-RPC error, with the HTTP status the Streamable HTTP transport answers it with.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
    pub http_status: u16,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
            http_status: 200,
        }
    }

    pub fn parse(message: impl Into<String>) -> Self {
        Self::new(PARSE_ERROR, message).status(400)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(INVALID_REQUEST, message).status(400)
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(METHOD_NOT_FOUND, format!("Method not found: {method}"))
    }

    pub fn header_mismatch(message: impl Into<String>) -> Self {
        Self::new(HEADER_MISMATCH, message).status(400)
    }

    pub fn status(mut self, status: u16) -> Self {
        self.http_status = status;
        self
    }

    pub fn data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    /// The error response for request `id` (`null` when it couldn't be read).
    pub fn response(&self, id: Value) -> Value {
        let mut error = json!({ "code": self.code, "message": self.message });
        if let Some(d) = &self.data {
            error["data"] = d.clone();
        }
        json!({ "jsonrpc": "2.0", "id": id, "error": error })
    }
}

pub fn result_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}
