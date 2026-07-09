//! Pure JSON-RPC dispatch: [`Server::handle_line`] takes one line, returns
//! at most one line, no process spawning or I/O beyond the tool calls
//! themselves — so this is fully unit-testable with in-memory strings.
//! stdin/stdout framing lives in `main`.

use serde_json::{Value, json};

use crate::protocol::{
    self, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, Message, ParseOutcome,
};
use crate::tools::{self, ToolOutcome};

/// The `protocolVersion` this server was built against. Echoed back to
/// clients that ask for something else, since a tools-only server (no
/// resources/prompts/sampling) serves every recent protocol revision
/// identically.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Stateless: every tool call is an independent offline operation on
/// caller-given paths, so there's no session state to carry between lines.
#[derive(Default)]
pub struct Server;

impl Server {
    pub fn new() -> Self {
        Server
    }

    /// Handles one line of input, returning the line to write back (if
    /// any). `None` means: this was a notification (no "id" member) —
    /// notifications never get a response, success or error alike
    /// (JSON-RPC 2.0 sec. 4.1).
    pub fn handle_line(&self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        match protocol::parse_line(line) {
            ParseOutcome::ParseError => Some(protocol::error_response(
                Value::Null,
                protocol::PARSE_ERROR,
                "parse error: invalid JSON",
            )),
            ParseOutcome::Invalid { id: Some(id) } => Some(protocol::error_response(
                id,
                INVALID_REQUEST,
                "invalid request: expected an object with a string \"method\"",
            )),
            ParseOutcome::Invalid { id: None } => None,
            ParseOutcome::Message(msg) => self.dispatch(msg),
        }
    }

    fn dispatch(&self, msg: Message) -> Option<String> {
        let result = match msg.method.as_str() {
            "initialize" => Ok(self.initialize(&msg.params)),
            "notifications/initialized" => Ok(Value::Null),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.tools_list()),
            "tools/call" => self.tools_call(&msg.params),
            other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
        };

        // Notifications (no "id") never get a response, whether the
        // dispatch above succeeded or not.
        let id = msg.id?;
        Some(match result {
            Ok(value) => protocol::response(id, value),
            Err((code, message)) => protocol::error_response(id, code, message),
        })
    }

    fn initialize(&self, params: &Value) -> Value {
        // Tools-only servers are version-tolerant (no resources/prompts/
        // sampling capability whose shape actually changed across
        // revisions), so echo back whatever the client requested rather
        // than force a mismatch it can't act on.
        let version = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(PROTOCOL_VERSION);
        json!({
            "protocolVersion": version,
            "capabilities": {"tools": {}},
            "serverInfo": {
                "name": "cochlea-mcp",
                "version": env!("CARGO_PKG_VERSION"),
            },
        })
    }

    fn tools_list(&self) -> Value {
        json!({"tools": tools::schemas()})
    }

    fn tools_call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params.get("name").and_then(Value::as_str).ok_or_else(|| {
            (
                INVALID_PARAMS,
                "tools/call requires a string \"name\"".to_string(),
            )
        })?;
        let empty = json!({});
        let args = params.get("arguments").unwrap_or(&empty);

        let outcome = match name {
            "render_score" => tools::render_score(args),
            "probe_audio" => tools::probe_audio(args),
            "spectrogram" => tools::spectrogram(args),
            "lint_score" => tools::lint_score(args),
            "probe_digest" => tools::probe_digest(args),
            "audio_diff" => tools::audio_diff(args),
            other => return Err((INVALID_PARAMS, format!("unknown tool: {other}"))),
        };

        Ok(match outcome {
            ToolOutcome::Ok(text) => json!({
                "content": [{"type": "text", "text": text}],
                "isError": false,
            }),
            ToolOutcome::Failed(text) => json!({
                "content": [{"type": "text", "text": text}],
                "isError": true,
            }),
            ToolOutcome::InvalidParams(message) => return Err((INVALID_PARAMS, message)),
        })
    }
}
