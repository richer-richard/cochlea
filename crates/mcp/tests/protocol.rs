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
fn initialize_echoes_an_unrecognized_client_protocol_version() {
    // Tools-only servers are version-tolerant: a client on a different
    // protocol revision still gets served, and gets its own version back
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
            "probe_wav",
            "spectrogram",
            "lint_score",
            "probe_digest",
            "audio_diff",
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
            "params": {"name": "probe_wav", "arguments": {}},
        }),
    );
    assert_eq!(response["error"]["code"], -32602);
    assert!(response["result"].is_null());
}

#[test]
fn audio_diff_missing_wav_path_b_is_invalid_params() {
    let server = Server::new();
    let response = call(
        &server,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "audio_diff", "arguments": {"wav_path_a": "a.wav"}},
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
