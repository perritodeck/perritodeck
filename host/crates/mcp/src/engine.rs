//! The protocol engine: JSON-RPC messages in, JSON-RPC messages out, whatever the
//! transport.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::{json, Map, Value};
use tracing::debug;

use crate::{
    jsonrpc::{result_response, RpcError, UNSUPPORTED_VERSION},
    tools, Backend, LEGACY_VERSIONS, MODERN_VERSIONS,
};

pub(crate) const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub(crate) const META_CLIENT_CAPS: &str = "io.modelcontextprotocol/clientCapabilities";
pub(crate) const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// How many tool calls an assistant may make in [`RATE_WINDOW`].
const RATE_LIMIT: usize = 60;
const RATE_WINDOW: Duration = Duration::from_secs(10);
/// How long a modern client may cache `tools/list` (it changes when settings do).
const TOOLS_TTL_MS: u64 = 30_000;

const INSTRUCTIONS: &str = "PerritoDeck turns tablets into a Stream Deck: grids of keys that \
run hotkeys, type text, control media, OBS and Home Assistant, open pages of more keys, and so \
on. Keys live in profiles; each profile has named pages; a key sits at [row, col], counted from \
0 at the top left. Look before you change anything: list_profiles, then get_profile. Which other \
tools exist depends on what the person has allowed in PerritoDeck's settings (pressing keys, \
editing keys, running actions).";

/// Protocol state that lasts across messages: one stdio connection or one HTTP session.
/// Only the handshake-based versions use it; stateless (`2026-07-28`) requests ignore it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    pub(crate) version: Option<&'static str>,
    pub(crate) initialized: bool,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// The version agreed in `initialize`, if that happened.
    pub fn protocol_version(&self) -> Option<&'static str> {
        self.version
    }

    /// Whether the client has sent `notifications/initialized`.
    pub fn initialized(&self) -> bool {
        self.initialized
    }

    fn batches_allowed(&self) -> bool {
        matches!(self.version, Some("2024-11-05" | "2025-03-26"))
    }

    fn structured_content(&self) -> bool {
        self.version.map_or(true, |v| v >= "2025-06-18")
    }
}

/// A reply, with the HTTP status the Streamable HTTP transport sends it with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reply {
    pub message: Value,
    pub http_status: u16,
}

impl Reply {
    fn ok(message: Value) -> Self {
        Self {
            message,
            http_status: 200,
        }
    }

    fn error(id: Value, e: &RpcError) -> Self {
        Self {
            message: e.response(id),
            http_status: e.http_status,
        }
    }
}

/// How a request is being served.
pub(crate) struct Ctx {
    /// Stateless 2026-07-28 request.
    pub modern: bool,
    /// The client understands `structuredContent` (2025-06-18 and later).
    pub structured: bool,
}

/// The MCP server, independent of transport. Cheap to clone.
#[derive(Clone)]
pub struct Server {
    pub(crate) backend: Arc<dyn Backend>,
    calls: Arc<Mutex<VecDeque<Instant>>>,
}

impl Server {
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Self {
            backend,
            calls: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Handle one line of newline-delimited JSON-RPC (a message, or a batch where the
    /// protocol version allows them). Returns the line to send back, if any.
    pub async fn handle_text(&self, text: &str, session: &mut Session) -> Option<String> {
        let reply = match serde_json::from_str::<Value>(text) {
            Ok(v) => self.handle_value(v, session).await?,
            Err(e) => Reply::error(Value::Null, &RpcError::parse(format!("Parse error: {e}"))),
        };
        Some(reply.message.to_string())
    }

    pub(crate) async fn handle_value(&self, v: Value, session: &mut Session) -> Option<Reply> {
        let Value::Array(items) = v else {
            return self.handle_one(v, session, false).await;
        };
        if !session.batches_allowed() {
            let version = session.version.unwrap_or("this protocol version");
            return Some(Reply::error(
                Value::Null,
                &RpcError::invalid_request(format!(
                    "JSON-RPC batches aren't part of MCP {version}; send one message at a time"
                )),
            ));
        }
        if items.is_empty() {
            return Some(Reply::error(
                Value::Null,
                &RpcError::invalid_request("empty batch"),
            ));
        }
        let mut out = Vec::new();
        for item in items {
            if let Some(r) = self.handle_one(item, session, true).await {
                out.push(r.message);
            }
        }
        (!out.is_empty()).then(|| Reply::ok(Value::Array(out)))
    }

    async fn handle_one(&self, msg: Value, session: &mut Session, in_batch: bool) -> Option<Reply> {
        let Value::Object(mut obj) = msg else {
            return Some(Reply::error(
                Value::Null,
                &RpcError::invalid_request("expected a JSON-RPC message object"),
            ));
        };
        let id = obj.get("id").cloned();
        let id_or_null = || id.clone().unwrap_or(Value::Null);
        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(Reply::error(
                id_or_null(),
                &RpcError::invalid_request("\"jsonrpc\" must be \"2.0\""),
            ));
        }
        let method = match obj.get("method") {
            Some(Value::String(m)) => m.clone(),
            Some(_) => {
                return Some(Reply::error(
                    id_or_null(),
                    &RpcError::invalid_request("\"method\" must be a string"),
                ))
            }
            // A response to a request of ours. This server sends none, so drop it.
            None if obj.contains_key("result") || obj.contains_key("error") => return None,
            None => {
                return Some(Reply::error(
                    id_or_null(),
                    &RpcError::invalid_request("a request needs a \"method\""),
                ))
            }
        };
        let params = obj.remove("params");
        let Some(id) = id else {
            self.notification(&method, session);
            return None;
        };
        if !(id.is_string() || id.is_number()) {
            return Some(Reply::error(
                Value::Null,
                &RpcError::invalid_request("\"id\" must be a string or a number"),
            ));
        }
        let params = match params {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(m)) => m,
            Some(_) => {
                return Some(Reply::error(
                    id,
                    &RpcError::invalid_params("\"params\" must be an object").status(400),
                ))
            }
        };
        debug!(method, "MCP request");

        let outcome = if method == "initialize" {
            if in_batch {
                Err(RpcError::invalid_request(
                    "initialize can't be sent in a batch",
                ))
            } else {
                self.initialize(&params, session)
            }
        } else {
            match params.get("_meta").and_then(|m| m.get(META_VERSION)) {
                Some(v) => {
                    let v = v.clone();
                    self.modern(&method, params, &v).await
                }
                None => self.legacy(&method, params, session).await,
            }
        };
        Some(match outcome {
            Ok(result) => Reply::ok(result_response(id, result)),
            Err(e) => Reply::error(id, &e),
        })
    }

    fn notification(&self, method: &str, session: &mut Session) {
        match method {
            "notifications/initialized" => session.initialized = true,
            // Calls are short and run in order; nothing to cancel.
            "notifications/cancelled" => {}
            other => debug!(method = other, "ignoring notification"),
        }
    }

    fn initialize(
        &self,
        params: &Map<String, Value>,
        session: &mut Session,
    ) -> Result<Value, RpcError> {
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params("initialize needs \"protocolVersion\""))?;
        let version = LEGACY_VERSIONS
            .iter()
            .copied()
            .find(|v| *v == requested)
            .unwrap_or(LEGACY_VERSIONS[0]);
        *session = Session {
            version: Some(version),
            initialized: false,
        };
        Ok(json!({
            "protocolVersion": version,
            "capabilities": capabilities(),
            "serverInfo": server_info(),
            "instructions": INSTRUCTIONS,
        }))
    }

    async fn modern(
        &self,
        method: &str,
        params: Map<String, Value>,
        version: &Value,
    ) -> Result<Value, RpcError> {
        let Some(version) = version.as_str() else {
            return Err(RpcError::invalid_params(format!(
                "\"{META_VERSION}\" in _meta must be a string"
            ))
            .status(400));
        };
        if !MODERN_VERSIONS.contains(&version) {
            return Err(unsupported_version(version));
        }
        let meta = params.get("_meta").expect("checked by the caller");
        if !meta.get(META_CLIENT_CAPS).is_some_and(Value::is_object) {
            return Err(RpcError::invalid_params(format!(
                "\"{META_CLIENT_CAPS}\" is required in _meta"
            ))
            .status(400));
        }
        let ctx = Ctx {
            modern: true,
            structured: true,
        };
        let mut result = match method {
            "server/discover" => json!({
                "supportedVersions": all_versions(),
                "capabilities": capabilities(),
                "instructions": INSTRUCTIONS,
            }),
            "tools/list" => self.tools_list(&params, &ctx)?,
            "tools/call" => self.tools_call(&params, &ctx).await?,
            other => return Err(RpcError::method_not_found(other).status(404)),
        };
        let obj = result.as_object_mut().expect("results are objects");
        obj.insert("resultType".into(), json!("complete"));
        obj.insert("_meta".into(), json!({ META_SERVER_INFO: server_info() }));
        Ok(result)
    }

    async fn legacy(
        &self,
        method: &str,
        params: Map<String, Value>,
        session: &Session,
    ) -> Result<Value, RpcError> {
        let ctx = Ctx {
            modern: false,
            structured: session.structured_content(),
        };
        match method {
            "ping" => Ok(json!({})),
            "tools/list" => self.tools_list(&params, &ctx),
            "tools/call" => self.tools_call(&params, &ctx).await,
            "server/discover" => Err(RpcError::invalid_params(format!(
                "server/discover needs \"{META_VERSION}\" in _meta"
            ))
            .status(400)),
            other => Err(RpcError::method_not_found(other)),
        }
    }

    fn tools_list(&self, params: &Map<String, Value>, ctx: &Ctx) -> Result<Value, RpcError> {
        if params.get("cursor").is_some_and(|c| !c.is_null()) {
            return Err(RpcError::invalid_params(
                "unknown cursor (the tool list has only one page)",
            ));
        }
        let mut result = json!({ "tools": tools::list(self.backend.permissions()) });
        if ctx.modern {
            result["ttlMs"] = json!(TOOLS_TTL_MS);
            // The list depends on this host's settings.
            result["cacheScope"] = json!("private");
        }
        Ok(result)
    }

    async fn tools_call(&self, params: &Map<String, Value>, ctx: &Ctx) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params("tools/call needs the tool's \"name\""))?;
        let args = match params.get("arguments") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(m)) => m.clone(),
            Some(_) => return Err(RpcError::invalid_params("\"arguments\" must be an object")),
        };
        let out = if self.over_rate_limit() {
            tools::ToolOutput::error(
                "Too many PerritoDeck tool calls in a short time; wait a few seconds and try again.",
            )
        } else {
            tools::call(&*self.backend, name, args).await?
        };
        Ok(out.into_result(ctx.structured))
    }

    fn over_rate_limit(&self) -> bool {
        let now = Instant::now();
        let mut calls = self.calls.lock().expect("rate limiter poisoned");
        while calls
            .front()
            .is_some_and(|t| now.duration_since(*t) > RATE_WINDOW)
        {
            calls.pop_front();
        }
        if calls.len() >= RATE_LIMIT {
            return true;
        }
        calls.push_back(now);
        false
    }
}

pub(crate) fn unsupported_version(requested: &str) -> RpcError {
    RpcError::new(UNSUPPORTED_VERSION, "Unsupported protocol version")
        .data(json!({ "supported": all_versions(), "requested": requested }))
        .status(400)
}

pub(crate) fn all_versions() -> Vec<&'static str> {
    MODERN_VERSIONS
        .iter()
        .chain(LEGACY_VERSIONS.iter())
        .copied()
        .collect()
}

fn capabilities() -> Value {
    // The tool list changes with the host's settings, but this server has no stream to
    // announce that on; clients see it on their next tools/list.
    json!({ "tools": { "listChanged": false } })
}

fn server_info() -> Value {
    json!({
        "name": "perritodeck",
        "title": "PerritoDeck",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Keys on your PerritoDeck tablets: see them, press them, make new ones",
        "websiteUrl": "https://perritodeck.dev",
    })
}
