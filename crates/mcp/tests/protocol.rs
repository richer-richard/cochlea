//! Protocol conformance: the JSON-RPC envelope rules from the MCP stdio
//! transport, exercised entirely in-process against `Server::handle_line`
//! (no subprocess — `crates/mcp/src/lib.rs` exists precisely so this file
//! can do that).

use cochlea_mcp::server::Server;
use serde_json::{Value, json};

fn call(server: &Server, request: Value) -> Value {
    let line = server
        .handle_line(&request.to_string())
        .expect("expected a response line");
    serde_json::from_str(&line).expect("response must be valid JSON")
}

#[test]
fn initialize_handshake() {
    let server = Server::new();
    let response = call(
        &server,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.0.0"},
            },
        }),
    );
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(response["result"]["capabilities"]["tools"], json!({}));
    assert_eq!(response["result"]["serverInfo"]["name"], "cochlea-mcp");
    assert!(response["result"]["serverInfo"]["version"].is_string());
}

#[test]
fn initialize_echoes_a_recognized_older_protocol_version() {
    // Tools-only servers are version-tolerant across KNOWN revisions: a
    // client on a recognized older revision gets its own version back
    // rather than a hardcoded one it didn't ask for.
    let server = Server::new();
    let response = call(
        &server,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": "2024-11-05"},
        }),
    );
    assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
}

#[test]
fn initialize_answers_an_unrecognized_protocol_version_with_its_own() {
    // Per spec the server responds with a version it actually supports —
    // never a blind mirror of arbitrary client input.
    let server = Server::new();
    let response = call(
        &server,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": "not-a-real-version-2077-99-99"},
        }),
    );
    assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
}

#[test]
fn a_top_level_json_array_is_invalid_request_not_silence() {
    // JSON-RPC notifications are objects; a non-object payload (e.g. a
    // batch array, unsupported over MCP stdio) must get an id:null Invalid
    // Request response — silence would hang a client awaiting a reply.
    let server = Server::new();
    let line = server
        .handle_line(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#)
        .expect("a non-object payload must get a response");
    let response: Value = serde_json::from_str(&line).unwrap();
    assert!(response["id"].is_null(), "{response}");
    assert_eq!(response["error"]["code"], -32600, "{response}");

    let line = server
        .handle_line(r#""just a string""#)
        .expect("a bare string must get a response");
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["error"]["code"], -32600, "{response}");
}

#[test]
fn notification_gets_no_response() {
    let server = Server::new();
    let response = server
        .handle_line(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string());
    assert!(response.is_none());
}

#[test]
fn unknown_method_notification_still_gets_no_response() {
    // Notifications never get a response, even an error one — the
    // "no id" rule is unconditional, not specific to known methods.
    let server = Server::new();
    let response =
        server.handle_line(&json!({"jsonrpc": "2.0", "method": "no/such/method"}).to_string());
    assert!(response.is_none());
}

#[test]
fn ping() {
    let server = Server::new();
    let response = call(
        &server,
        json!({"jsonrpc": "2.0", "id": 7, "method": "ping"}),
    );
    assert_eq!(response["result"], json!({}));
}

#[test]
fn tools_list_shape() {
    let server = Server::new();
    let response = call(
        &server,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    );
    let tools = response["result"]["tools"]
        .as_array()
        .expect("tools must be an array");
    let names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("tool name must be a string"))
        .collect();
    assert_eq!(
        names,
        vec![
            "render_score",
            "probe_audio",
            "spectrogram",
            "lint_score",
            "probe_digest",
            "score_reference",
            "audio_diff",
            "import_midi",
            "export_midi",
            "transcribe_audio",
            "loudness_timeline",
            "beat_grid",
        ]
    );
    for tool in tools {
        assert!(tool["description"].is_string(), "{tool}");
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object", "{tool}");
        assert!(schema["properties"].is_object(), "{tool}");
        assert!(schema["required"].is_array(), "{tool}");
    }
}

#[test]
fn unknown_method_on_a_request_is_method_not_found() {
    let server = Server::new();
    let response = call(
        &server,
        json!({"jsonrpc": "2.0", "id": 3, "method": "no/such/method"}),
    );
    assert_eq!(response["error"]["code"], -32601);
    assert!(response["result"].is_null());
}

#[test]
fn malformed_json_is_parse_error_with_null_id() {
    let server = Server::new();
    let line = server
        .handle_line("{not valid json")
        .expect("parse errors still get a response");
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["error"]["code"], -32700);
    assert_eq!(response["id"], Value::Null);
}

#[test]
fn a_request_missing_method_is_invalid_request() {
    let server = Server::new();
    let response = call(&server, json!({"jsonrpc": "2.0", "id": 4, "params": {}}));
    assert_eq!(response["error"]["code"], -32600);
    assert_eq!(response["id"], 4);
}

#[test]
fn tools_call_missing_required_argument_is_invalid_params() {
    let server = Server::new();
    let response = call(
        &server,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "probe_audio", "arguments": {}},
        }),
    );
    assert_eq!(response["error"]["code"], -32602);
    assert!(response["result"].is_null());
}

#[test]
fn audio_diff_missing_audio_path_b_is_invalid_params() {
    let server = Server::new();
    let response = call(
        &server,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "audio_diff", "arguments": {"audio_path_a": "a.wav"}},
        }),
    );
    assert_eq!(response["error"]["code"], -32602);
    assert!(response["result"].is_null());
}

#[test]
fn tools_call_unknown_tool_name_is_invalid_params() {
    let server = Server::new();
    let response = call(
        &server,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {"name": "no_such_tool", "arguments": {}},
        }),
    );
    assert_eq!(response["error"]["code"], -32602);
}

#[test]
fn tools_call_missing_name_is_invalid_params() {
    let server = Server::new();
    let response = call(
        &server,
        json!({"jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": {}}),
    );
    assert_eq!(response["error"]["code"], -32602);
}

#[test]
fn identical_requests_produce_identical_responses() {
    // No wall clock, no session state — same input, same output, byte for
    // byte (the determinism contract this workspace holds everywhere).
    let server = Server::new();
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let first = server.handle_line(&request.to_string()).unwrap();
    let second = server.handle_line(&request.to_string()).unwrap();
    assert_eq!(first, second);
}

mod framing {
    //! The `serve` framing loop, driven with in-memory readers/writers —
    //! the same code `main` runs over stdin/stdout.

    use cochlea_mcp::server::{Server, serve};
    use serde_json::Value;

    #[test]
    fn serve_round_trips_requests_and_stays_silent_on_notifications() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
            "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        serve(&Server::new(), input.as_bytes(), &mut out).unwrap();
        let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
        assert_eq!(lines.len(), 2, "notification must produce no line");
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(first["id"], 1);
        assert_eq!(second["id"], 2);
    }

    #[test]
    fn serve_rejects_an_oversized_line_without_buffering_it() {
        // One line over the 8 MiB cap, followed by a normal request: the
        // huge line gets an id:null Invalid Request error and the server
        // keeps working. (~9 MB of input; the point is the cap fires and
        // the loop survives.)
        let mut input: Vec<u8> = Vec::with_capacity(9 * 1024 * 1024 + 64);
        input.extend(br#"{"pad":""#);
        input.resize(9 * 1024 * 1024, b'x');
        input.extend(b"\"}\n");
        input.extend(br#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#);
        input.extend(b"\n");

        let mut out: Vec<u8> = Vec::new();
        serve(&Server::new(), input.as_slice(), &mut out).unwrap();
        let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert!(first["id"].is_null(), "{first}");
        assert_eq!(first["error"]["code"], -32600, "{first}");
        assert!(
            first["error"]["message"]
                .as_str()
                .unwrap()
                .contains("exceeds"),
            "{first}"
        );
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["id"], 7, "the loop must survive an oversized line");
    }
}
