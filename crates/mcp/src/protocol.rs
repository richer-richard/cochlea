//! JSON-RPC 2.0 envelope: parsing one inbound line into a [`Message`] and
//! rendering one outbound line, per the MCP stdio transport (newline-
//! delimited, one JSON object per line, no batching). stdout carries only
//! these envelopes — everything else (logs, panics-as-text) goes to stderr,
//! wired in `main`.

use serde_json::{Value, json};

/// The line wasn't valid JSON at all.
pub const PARSE_ERROR: i64 = -32700;
/// The line was valid JSON but not a well-formed request/notification.
pub const INVALID_REQUEST: i64 = -32600;
/// `method` doesn't name anything this server handles.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// A known method's `params`/`arguments` were missing or the wrong shape.
pub const INVALID_PARAMS: i64 = -32602;

/// One parsed inbound request or notification. `id` is `None` exactly when
/// the "id" member was absent from the JSON object — a notification, which
/// must never get a response (JSON-RPC 2.0 sec. 4.1). This is deliberately
/// not `#[derive(Deserialize)]`: `Option<Value>` conflates "member absent"
/// with "member present and `null`", and the two must be told apart here.
pub struct Message {
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

/// The result of parsing one line.
pub enum ParseOutcome {
    Message(Message),
    /// Valid JSON, but not an object with a string "method" — e.g. a bare
    /// array, or an object missing "method". `id` carries through if the
    /// object had one, so a genuine (if malformed) request still gets an
    /// Invalid Request error rather than silent drop; a notification-shaped
    /// non-request (no "id") is dropped like any other notification.
    Invalid {
        id: Option<Value>,
    },
    /// The line was not valid JSON at all.
    ParseError,
}

/// Parses one line of input.
pub fn parse_line(line: &str) -> ParseOutcome {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return ParseOutcome::ParseError;
    };
    let obj = value.as_object();
    let has_id = obj.is_some_and(|o| o.contains_key("id"));
    let id = obj.and_then(|o| o.get("id")).cloned();
    let method = obj.and_then(|o| o.get("method")).and_then(Value::as_str);

    match method {
        Some(method) => ParseOutcome::Message(Message {
            id,
            method: method.to_string(),
            params: obj
                .and_then(|o| o.get("params"))
                .cloned()
                .unwrap_or(Value::Null),
        }),
        None => ParseOutcome::Invalid {
            id: if has_id { id } else { None },
        },
    }
}

/// Renders a success response line.
pub fn response(id: Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

/// Renders a JSON-RPC error response line (protocol-level errors only —
/// tool-level failures are a success response with `isError: true`, never
/// this).
pub fn error_response(id: Value, code: i64, message: impl Into<String>) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message.into()},
    })
    .to_string()
}
