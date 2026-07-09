//! End-to-end tool tests: real render, real probe, real spectrogram, real
//! digest/diff, all driven through `Server::handle_line` in-process — the
//! same path a JSON line arriving on stdin would take, just without the
//! framing. Kept to one render (renders cost seconds); everything else
//! reuses its output.

use cochlea_mcp::server::Server;
use serde_json::{Value, json};

fn score_path() -> String {
    format!(
        "{}/../../examples/scores/first_light.ron",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn tmp_path(name: &str) -> String {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_str().unwrap().to_string()
}

fn call_tool(server: &Server, id: i64, name: &str, arguments: Value) -> Value {
    let request = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    });
    let line = server
        .handle_line(&request.to_string())
        .expect("tools/call must respond");
    serde_json::from_str(&line).expect("response must be valid JSON")
}

fn tool_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("expected text content: {response}"))
}

#[test]
fn render_probe_spectrogram_round_trip() {
    let server = Server::new();
    let wav = tmp_path("first_light.wav");
    let stems = tmp_path("stems");

    // 1. render_score, with verify — first_light.ron's embedded verify:
    // block (TruePeakBelow, SilentAfter) is the blessed golden, so it must
    // pass.
    let response = call_tool(
        &server,
        1,
        "render_score",
        json!({
            "score_path": score_path(),
            "out_path": wav,
            "stems_dir": stems,
            "verify": true,
        }),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = tool_text(&response);
    assert!(text.contains("48000 Hz"), "{text}");
    assert!(text.contains("dBFS"), "{text}");
    assert!(text.contains("\"passed\": true"), "{text}");
    assert!(std::path::Path::new(&wav).exists());
    assert!(std::path::Path::new(&stems).join("lead.wav").exists());

    // 2. probe_audio on the file render_score just wrote.
    let response = call_tool(&server, 2, "probe_audio", json!({"audio_path": wav}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let report: Value = serde_json::from_str(tool_text(&response)).unwrap();
    assert_eq!(report["schema_version"], 2);
    assert_eq!(report["source"]["sample_rate"], 48_000);
    assert_eq!(report["source"]["channels"], 2);

    // 3. spectrogram on the same file, plain and as a contact sheet.
    let png = tmp_path("first_light.png");
    let response = call_tool(
        &server,
        3,
        "spectrogram",
        json!({"audio_path": wav, "out_path": png}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(std::path::Path::new(&png).exists());
    assert!(tool_text(&response).contains(&png));

    let sheet = tmp_path("first_light_sheet.png");
    let response = call_tool(
        &server,
        4,
        "spectrogram",
        json!({"audio_path": wav, "out_path": sheet, "sheet": true, "bars_per_tile": 2}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(std::path::Path::new(&sheet).exists());

    // 4. probe_digest on the same file — the token-cheap alternative to
    // probe_audio's full JSON.
    let response = call_tool(&server, 5, "probe_digest", json!({"audio_path": wav}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let digest = tool_text(&response);
    assert!(digest.starts_with("cochlea digest:"), "{digest}");

    // 5. audio_diff against itself — same file both sides must land on
    // ByteIdentical, not just Tier2Equivalent.
    let response = call_tool(
        &server,
        6,
        "audio_diff",
        json!({"audio_path_a": wav, "audio_path_b": wav}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(
        tool_text(&response).contains("byte-identical"),
        "{}",
        tool_text(&response)
    );
}

/// FLAC goes through the same tools as WAV (`cochlea_decode::load`
/// dispatches on extension) — read the decode crate's committed fixture,
/// no render needed.
#[test]
fn probe_audio_and_digest_read_flac() {
    let server = Server::new();
    let flac = format!(
        "{}/../decode/tests/fixtures/tone_mono_16.flac",
        env!("CARGO_MANIFEST_DIR")
    );

    let response = call_tool(&server, 1, "probe_audio", json!({"audio_path": flac}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let report: Value = serde_json::from_str(tool_text(&response)).unwrap();
    assert_eq!(report["schema_version"], 2);
    assert_eq!(report["source"]["channels"], 1);

    let response = call_tool(&server, 2, "probe_digest", json!({"audio_path": flac}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(
        tool_text(&response).starts_with("cochlea digest:"),
        "{response}"
    );
}

#[test]
fn lint_score_on_the_shipped_example_is_ok() {
    let server = Server::new();
    let response = call_tool(
        &server,
        1,
        "lint_score",
        json!({"score_path": score_path()}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(tool_text(&response).contains("ok"), "{response}");
}

#[test]
fn render_score_reports_a_tool_level_failure_without_a_jsonrpc_error() {
    // A bad path is a *tool*-level failure (isError:true, still a success
    // envelope), not a JSON-RPC protocol error — per spec, JSON-RPC errors
    // are reserved for parse/method/params problems.
    let server = Server::new();
    let response = call_tool(
        &server,
        1,
        "render_score",
        json!({"score_path": "/no/such/score.ron", "out_path": tmp_path("unused.wav")}),
    );
    assert!(response["error"].is_null(), "{response}");
    assert_eq!(response["result"]["isError"], true, "{response}");
    assert!(
        tool_text(&response).contains("no/such/score.ron"),
        "{response}"
    );
}
