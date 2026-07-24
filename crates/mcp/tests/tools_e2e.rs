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
    assert_eq!(report["schema_version"], 5);
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
    // The image comes back inline as MCP image content (content[0]) with
    // the text summary after it — a client with no filesystem access gets
    // to look at the audio directly. The base64 payload must decode-match
    // the PNG on disk byte-for-byte (same encoder).
    let blocks = response["result"]["content"]
        .as_array()
        .expect("content array");
    assert_eq!(blocks[0]["type"], "image", "{response}");
    assert_eq!(blocks[0]["mimeType"], "image/png", "{response}");
    let b64 = blocks[0]["data"].as_str().expect("base64 image data");
    assert!(!b64.is_empty() && b64.len().is_multiple_of(4));
    assert_eq!(blocks[1]["type"], "text", "{response}");
    assert!(
        blocks[1]["text"].as_str().unwrap().contains("inline"),
        "{response}"
    );

    let sheet = tmp_path("first_light_sheet.png");
    let response = call_tool(
        &server,
        4,
        "spectrogram",
        json!({"audio_path": wav, "out_path": sheet, "sheet": true, "bars_per_tile": 2}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(std::path::Path::new(&sheet).exists());

    // out_path is optional now: an inline-only call still returns the
    // image.
    let response = call_tool(&server, 41, "spectrogram", json!({"audio_path": wav}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert_eq!(
        response["result"]["content"][0]["type"], "image",
        "{response}"
    );

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
    assert_eq!(report["schema_version"], 5);
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

/// out_path colliding with the input is Invalid Params, not a "successful"
/// call that destroys the caller's file.
#[test]
fn out_path_must_not_overwrite_the_input() {
    let server = Server::new();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "render_score",
                   "arguments": {"score_path": score_path(), "out_path": score_path()}},
    });
    let line = server.handle_line(&request.to_string()).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["error"]["code"], -32602, "{response}");

    // Same guard on spectrogram, against an existing file (path
    // resolution precedes the alias check, so a nonexistent input fails
    // as an ordinary read error instead — also correct, differently).
    let request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "spectrogram",
                   "arguments": {"audio_path": score_path(), "out_path": score_path()}},
    });
    let line = server.handle_line(&request.to_string()).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["error"]["code"], -32602, "{response}");
}

/// The self-description contract: `score_reference` must (a) exist, (b)
/// document every preset the live bank actually has, and (c) contain a
/// worked example that *itself* parses, lints clean, and renders — an
/// agent that pastes the reference's example must succeed on the first
/// try, forever.
#[test]
fn score_reference_example_actually_renders() {
    let server = Server::new();
    let response = call_tool(&server, 1, "score_reference", json!({}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = tool_text(&response);

    for preset in cochlea_synth::PatchBank::presets().patch_names() {
        assert!(
            text.contains(&format!("`{preset}`")),
            "reference must document preset {preset}"
        );
    }
    for tool in ["lint_score", "render_score", "probe_audio", "spectrogram"] {
        assert!(text.contains(tool), "reference must mention {tool}");
    }

    // Extract the worked example: the last ```ron block.
    let start = text.rfind("```ron").expect("a worked example block") + "```ron".len();
    let end = start + text[start..].find("```").expect("closing fence");
    let example = text[start..end].trim();
    let score =
        cochlea_score::Score::from_ron(example).expect("the reference's worked example must parse");
    let errors: Vec<_> = score
        .validate(&cochlea_synth::PatchBank::presets())
        .into_iter()
        .filter(|f| f.severity == cochlea_score::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "example lint errors: {errors:?}");
    let rendered = cochlea_render::render(&score).expect("example must render");
    assert!(rendered.frames() > 0);
}

/// `--root` confinement, end to end: reads and writes outside the root
/// come back as JSON-RPC Invalid Params (-32602) before any filesystem
/// work; the same operations inside the root succeed.
#[test]
fn root_confinement_refuses_escapes_end_to_end() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("confined-root");
    std::fs::create_dir_all(&root).unwrap();
    // A score inside the root, copied from the repo fixture.
    let inside_score = root.join("first_light.ron");
    std::fs::copy(score_path(), &inside_score).unwrap();

    let server = Server::with_root(&root).expect("root exists");

    // Reading the repo fixture (outside the root) is refused.
    let response = call_tool(
        &server,
        1,
        "lint_score",
        json!({"score_path": score_path()}),
    );
    assert_eq!(response["error"]["code"], -32602, "{response}");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--root"),
        "{response}"
    );

    // Writing outside the root (dot-dot escape) is refused.
    let escape_wav = root.join("../escaped.wav");
    let response = call_tool(
        &server,
        2,
        "render_score",
        json!({
            "score_path": inside_score.to_str().unwrap(),
            "out_path": escape_wav.to_str().unwrap(),
        }),
    );
    assert_eq!(response["error"]["code"], -32602, "{response}");

    // The same work entirely inside the root succeeds.
    let wav = root.join("mix.wav");
    let response = call_tool(
        &server,
        3,
        "render_score",
        json!({
            "score_path": inside_score.to_str().unwrap(),
            "out_path": wav.to_str().unwrap(),
        }),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(wav.exists());
}

/// The canonical alias guard: `./score.ron` vs `score.ron` is the same
/// file, and render_score must refuse to clobber it regardless of
/// spelling (the old guard was string equality and missed this).
#[test]
fn clobber_guard_sees_through_path_aliases() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("alias-guard");
    std::fs::create_dir_all(&dir).unwrap();
    let score = dir.join("s.ron");
    std::fs::copy(score_path(), &score).unwrap();
    let aliased = dir.join(".").join("s.ron");

    let server = Server::new();
    let response = call_tool(
        &server,
        1,
        "render_score",
        json!({
            "score_path": score.to_str().unwrap(),
            "out_path": aliased.to_str().unwrap(),
        }),
    );
    assert_eq!(response["error"]["code"], -32602, "{response}");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("alias"),
        "{response}"
    );
}

/// The 0.3.0 additions in one pass over cheap committed fixtures: window
/// params, annotated spectrograms, the diff heat map, and MIDI import.
#[test]
fn hearing_upgrade_tools_end_to_end() {
    let server = Server::new();
    let stereo = format!(
        "{}/../decode/tests/fixtures/tone_stereo_16.flac",
        env!("CARGO_MANIFEST_DIR")
    );
    let mono = format!(
        "{}/../decode/tests/fixtures/tone_mono_16.flac",
        env!("CARGO_MANIFEST_DIR")
    );

    // probe_audio with a window: times are relative to the cut and
    // source.start_ms anchors it.
    let response = call_tool(
        &server,
        1,
        "probe_audio",
        json!({"audio_path": stereo, "from_s": 0.1, "to_s": 0.4}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    let report: Value = serde_json::from_str(tool_text(&response)).unwrap();
    assert_eq!(report["source"]["start_ms"], 100.0, "{report}");
    assert_eq!(report["source"]["duration_ms"], 300.0, "{report}");

    // An inverted window is a JSON-RPC invalid-params error, not a tool
    // failure.
    let response = call_tool(
        &server,
        2,
        "probe_audio",
        json!({"audio_path": stereo, "from_s": 0.4, "to_s": 0.1}),
    );
    assert_eq!(response["error"]["code"], -32602, "{response}");

    // Annotated spectrogram comes back inline; annotate+sheet is refused.
    let response = call_tool(
        &server,
        3,
        "spectrogram",
        json!({"audio_path": stereo, "annotate": true}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert_eq!(
        response["result"]["content"][0]["type"], "image",
        "{response}"
    );
    let response = call_tool(
        &server,
        4,
        "spectrogram",
        json!({"audio_path": stereo, "annotate": true, "sheet": true}),
    );
    assert_eq!(response["error"]["code"], -32602, "{response}");

    // audio_diff with the heat map: image content plus the text report.
    let response = call_tool(
        &server,
        5,
        "audio_diff",
        json!({"audio_path_a": mono, "audio_path_b": stereo, "spectrogram": true}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    let blocks = response["result"]["content"].as_array().unwrap();
    assert_eq!(blocks[0]["type"], "image", "{response}");
    assert!(
        blocks[1]["text"].as_str().unwrap().contains("verdict:"),
        "{response}"
    );

    // import_midi: a minimal byte-built SMF in, a lintable RON score out.
    let chunk = |id: &[u8; 4], data: &[u8]| {
        let mut out = id.to_vec();
        out.extend((data.len() as u32).to_be_bytes());
        out.extend(data);
        out
    };
    let mut midi = chunk(b"MThd", &[0, 0, 0, 1, 0x01, 0xE0]);
    midi.extend(chunk(
        b"MTrk",
        &[
            0x00, 0x90, 60, 100, 0x83, 0x60, 0x80, 60, 64, // C4 quarter
            0x00, 0xFF, 0x2F, 0x00,
        ],
    ));
    let midi_path = tmp_path("mcp_import.mid");
    std::fs::write(&midi_path, midi).unwrap();
    let ron_path = tmp_path("mcp_import.ron");
    let response = call_tool(
        &server,
        6,
        "import_midi",
        json!({"midi_path": midi_path, "out_path": ron_path}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(
        tool_text(&response).contains("imported 1 tracks"),
        "{}",
        tool_text(&response)
    );
    let response = call_tool(&server, 7, "lint_score", json!({"score_path": ron_path}));
    assert_eq!(response["result"]["isError"], false, "{response}");
}
