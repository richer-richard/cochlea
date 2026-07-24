//! Pure JSON-RPC dispatch: [`Server::handle_line`] takes one line, returns
//! at most one line, no process spawning or I/O beyond the tool calls
//! themselves — so this is fully unit-testable with in-memory strings.
//! The newline-delimited framing loop is [`serve`], generic over any
//! reader/writer pair (`main` passes stdin/stdout; tests pass buffers).

use std::io::{BufRead, Read, Write};

use serde_json::{Value, json};

use crate::protocol::{
    self, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, Message, ParseOutcome,
};
use crate::tools::{self, ToolOutcome};

/// The `protocolVersion` this server was built against, and the answer to
/// any client requesting a version it doesn't recognize.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Protocol revisions this tools-only server serves identically (no
/// resources/prompts/sampling capability whose shape changed between
/// them). A recognized requested version is echoed back; anything else
/// gets [`PROTOCOL_VERSION`].
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Cap on one inbound stdio line. Real requests are paths and small flags
/// (a few hundred bytes); without a cap, a single unterminated or
/// maliciously huge line grows a buffer without bound in a long-lived
/// server process.
const MAX_LINE_BYTES: u64 = 8 * 1024 * 1024;

/// Stateless between lines: every tool call is an independent offline
/// operation on caller-given paths. The only configuration is the
/// optional `--root` confinement directory, fixed at startup.
#[derive(Default)]
pub struct Server {
    ctx: tools::ToolCtx,
}

impl Server {
    pub fn new() -> Self {
        Server::default()
    }

    /// A server whose tools refuse any path outside `root` (canonicalized
    /// here; the directory must exist). See [`tools::ToolCtx`].
    pub fn with_root(root: &std::path::Path) -> std::io::Result<Self> {
        Ok(Server {
            ctx: tools::ToolCtx {
                root: Some(root.canonicalize()?),
            },
        })
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
            ParseOutcome::NotAnObject => Some(protocol::error_response(
                Value::Null,
                INVALID_REQUEST,
                "invalid request: payload must be a single JSON-RPC object \
                 (batches are not supported over the MCP stdio transport)",
            )),
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
        // Tools-only servers are version-tolerant across the KNOWN protocol
        // revisions (no resources/prompts/sampling capability whose shape
        // changed), so echo a recognized requested version back. Anything
        // unrecognized gets this server's own version instead — per spec,
        // the server answers with a version it actually supports, never a
        // blind mirror of arbitrary client input.
        let version = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .filter(|v| SUPPORTED_PROTOCOL_VERSIONS.contains(v))
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

        // A tool must never be able to take the server down. This is a
        // long-lived stdio process; an unwinding panic from any tool would
        // tear it down and lose every in-flight and future request in the
        // session, not just fail the one call. `catch_unwind` contains a
        // panic and turns it into an ordinary `isError: true` result.
        // `AssertUnwindSafe` is sound here: `self.ctx` is read-only and
        // `args` is borrowed immutably, so a contained panic leaves no
        // half-mutated state observable to later calls. The individual
        // panics are still worth fixing at their source — this is the
        // defense-in-depth backstop, not a substitute.
        let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.run_tool(name, args)
        })) {
            Ok(outcome) => outcome,
            Err(payload) => ToolOutcome::Failed(panic_message(name, payload.as_ref())),
        };

        Ok(match outcome {
            ToolOutcome::Ok(text) => json!({
                "content": [{"type": "text", "text": text}],
                "isError": false,
            }),
            ToolOutcome::OkContent(blocks) => json!({
                "content": blocks,
                "isError": false,
            }),
            ToolOutcome::Failed(text) => json!({
                "content": [{"type": "text", "text": text}],
                "isError": true,
            }),
            ToolOutcome::InvalidParams(message) => return Err((INVALID_PARAMS, message)),
        })
    }

    /// The tool-name dispatch, split out so [`tools_call`] can run it under
    /// `catch_unwind`. An unknown tool becomes `InvalidParams` (the wrapping
    /// match turns that back into the JSON-RPC error the caller expects).
    fn run_tool(&self, name: &str, args: &Value) -> ToolOutcome {
        match name {
            "render_score" => tools::render_score(&self.ctx, args),
            "probe_audio" => tools::probe_audio(&self.ctx, args),
            "spectrogram" => tools::spectrogram(&self.ctx, args),
            "lint_score" => tools::lint_score(&self.ctx, args),
            "probe_digest" => tools::probe_digest(&self.ctx, args),
            "audio_diff" => tools::audio_diff(&self.ctx, args),
            "import_midi" => tools::import_midi(&self.ctx, args),
            "export_midi" => tools::export_midi(&self.ctx, args),
            "score_reference" => tools::score_reference(),
            // Test-only arm: a guaranteed panic to prove the `catch_unwind`
            // backstop keeps the server alive, independent of whichever real
            // panics happen to be reachable at any given time.
            #[cfg(test)]
            "__panic_for_test" => panic!("intentional panic for the containment test"),
            other => ToolOutcome::InvalidParams(format!("unknown tool: {other}")),
        }
    }
}

/// Render a caught panic payload into a caller-facing message. The payload
/// of a `panic!` is a `&str` or `String`; anything else is reported
/// generically. The message states plainly that the server survived.
fn panic_message(tool: &str, payload: &(dyn std::any::Any + Send)) -> String {
    let detail = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_string());
    format!(
        "internal error: the {tool} tool panicked and was contained ({detail}). \
         This is a bug in cochlea; the server is still running and can take further requests."
    )
}

/// Runs the newline-delimited JSON-RPC loop over an arbitrary reader/writer
/// pair — `main` passes locked stdin/stdout; tests pass in-memory buffers.
/// A line longer than [`MAX_LINE_BYTES`] is answered with an `id: null`
/// Invalid Request error and skipped without ever being buffered whole, so
/// one hostile or corrupted line can't grow the server's memory without
/// bound. Returns on EOF; propagates real I/O errors to the caller.
pub fn serve<R: BufRead, W: Write>(
    server: &Server,
    mut reader: R,
    mut writer: W,
) -> std::io::Result<()> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        let n = reader
            .by_ref()
            .take(MAX_LINE_BYTES)
            .read_until(b'\n', &mut buf)?;
        if n == 0 {
            return Ok(()); // EOF: clean shutdown.
        }
        if !buf.ends_with(b"\n") && n as u64 == MAX_LINE_BYTES {
            // Oversized: drain the remainder of the line in fixed-size
            // chunks (never retaining it), then answer with an error.
            loop {
                let available = reader.fill_buf()?;
                if available.is_empty() {
                    break;
                }
                match available.iter().position(|&b| b == b'\n') {
                    Some(pos) => {
                        reader.consume(pos + 1);
                        break;
                    }
                    None => {
                        let len = available.len();
                        reader.consume(len);
                    }
                }
            }
            let response = protocol::error_response(
                Value::Null,
                INVALID_REQUEST,
                format!("invalid request: line exceeds the {MAX_LINE_BYTES}-byte limit"),
            );
            writeln!(writer, "{response}")?;
            writer.flush()?;
            continue;
        }
        // Invalid UTF-8 degrades to replacement characters, which fail JSON
        // parsing and come back as an ordinary parse-error response.
        let line = String::from_utf8_lossy(&buf);
        if let Some(response) = server.handle_line(&line) {
            writeln!(writer, "{response}")?;
            writer.flush()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn tool_call_line(id: u64, tool: &str) -> String {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}"}}}}"#
        )
    }

    /// Finding 2 regression: a panicking tool must be contained as an
    /// `isError` result, and the *next* request on the same long-lived
    /// server must still be answered — the panic must not unwind the serve
    /// loop and kill the process.
    #[test]
    fn a_tool_panic_is_contained_and_the_server_keeps_serving() {
        let server = Server::new();
        let input = format!(
            "{}\n{}\n",
            tool_call_line(1, "__panic_for_test"),
            tool_call_line(2, "score_reference"),
        );
        let mut out = Vec::new();
        serve(&server, Cursor::new(input.into_bytes()), &mut out).expect("serve must not error");

        let out = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "both requests must be answered:\n{out}");

        // id:1 — the panic came back as a contained tool error, not a dead
        // process.
        let r1: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(r1["id"], 1);
        assert_eq!(
            r1["result"]["isError"], true,
            "a panicking tool must report isError"
        );
        let text = r1["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("panicked"),
            "panic surfaced to caller: {text}"
        );

        // id:2 — the very next call still works: the server survived.
        let r2: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(r2["id"], 2);
        assert_eq!(
            r2["result"]["isError"], false,
            "the server must keep serving after a contained panic"
        );
    }
}
