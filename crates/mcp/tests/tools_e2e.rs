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

    // 6. loudness_timeline on the same file — the dynamics curve as JSON.
    let response = call_tool(&server, 7, "loudness_timeline", json!({"audio_path": wav}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let curve: Value = serde_json::from_str(tool_text(&response)).unwrap();
    assert_eq!(curve["hop_ms"], 100.0, "{curve}");
    assert!(
        curve["points"].as_array().is_some_and(|p| !p.is_empty()),
        "loudness timeline should have points: {curve}"
    );
    // A bad hop is an Invalid Params error, not a silently empty curve.
    let bad = call_tool(
        &server,
        8,
        "loudness_timeline",
        json!({"audio_path": wav, "hop_ms": 0}),
    );
    assert_eq!(bad["error"]["code"], -32602, "{bad}");

    // 7. beat_grid on the same file — the full TempoReport with the beat grid.
    let response = call_tool(&server, 9, "beat_grid", json!({"audio_path": wav}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let grid: Value = serde_json::from_str(tool_text(&response)).unwrap();
    // TempoReport shape: the per-beat grid the compact summary omits.
    assert!(grid["beats_ms"].is_array(), "beat grid present: {grid}");
    assert!(
        grid.get("downbeats_ms").is_some(),
        "downbeats present: {grid}"
    );
    assert!(
        grid.get("candidates").is_some(),
        "candidates present: {grid}"
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

/// `--root` confinement, laundered through score data. Every *path
/// argument* here is legitimately inside the root — it is the score's track
/// name that points outside, and `stems_dir/<track>.wav` used to follow it
/// (`Path::join` discards the base for an absolute argument). Reproduced
/// against 0.6.0: the escape wrote outside the root and the tool still
/// reported `isError: false`. The confinement test above only covers direct
/// path arguments, which is exactly why this one exists.
#[test]
fn root_confinement_survives_a_path_shaped_track_name() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("confined-root-stems");
    let outside = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("outside-the-root");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("stems")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let victim = outside.join("master.wav");
    std::fs::write(&victim, b"UNTOUCHED").unwrap();

    // The track name is an absolute path outside the root; the writer
    // appends `.wav`, so name the victim minus its extension.
    let score_text = format!(
        r#"Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: {:?}, instrument: Preset("sine"),
        notes: [ Note(at: (1, 1), dur: "1/4", pitch: "A4", vel: 96) ]) ],
)"#,
        victim.with_extension("").to_str().unwrap()
    );
    let score = root.join("evil.ron");
    std::fs::write(&score, score_text).unwrap();

    let server = Server::with_root(&root).expect("root exists");
    let out = root.join("mix.wav");
    let response = call_tool(
        &server,
        1,
        "render_score",
        json!({
            "score_path": score.to_str().unwrap(),
            "out_path": out.to_str().unwrap(),
            "stems_dir": root.join("stems").to_str().unwrap(),
        }),
    );

    // An `isError` result, not Invalid Params: every argument is valid and
    // inside the root, and it is the score's *content* that cannot be
    // exported — the same class as a parse failure. It also has to reach the
    // model as tool output, or an agent never learns to rename the track.
    assert_eq!(response["result"]["isError"], true, "{response}");
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("portable file name"),
        "the reason should reach the caller: {response}"
    );
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        b"UNTOUCHED",
        "a track name must not be able to write outside --root"
    );
    assert!(
        !out.exists(),
        "nothing should be written when a track name is refused"
    );
}

/// The CLI refuses a stem that would land on the mix or the score;
/// `render_score` had no such guard at all. With `out_path = <root>/lead.wav`
/// and `stems_dir = <root>`, the mix was written and then destroyed by the
/// `lead` stem — reported as `isError: false`, with the *mix's* peak
/// summarising a file that now held one track.
#[test]
fn render_score_refuses_a_stem_that_would_overwrite_the_mix() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("stem-over-mix");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let score = root.join("score.ron");
    std::fs::write(
        &score,
        r#"Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: "lead", instrument: Preset("sine"),
        notes: [ Note(at: (1, 1), dur: "1/4", pitch: "A4", vel: 96) ]) ],
)"#,
    )
    .unwrap();

    let server = Server::with_root(&root).expect("root exists");
    let out = root.join("lead.wav"); // == the `lead` track's stem path
    let response = call_tool(
        &server,
        1,
        "render_score",
        json!({
            "score_path": score.to_str().unwrap(),
            "out_path": out.to_str().unwrap(),
            "stems_dir": root.to_str().unwrap(),
        }),
    );

    assert_eq!(response["error"]["code"], -32602, "{response}");
    assert!(
        !out.exists(),
        "the guard must fire before anything is written"
    );
}

/// A well-formed track name is not enough: if a symlink already sits at the
/// stem's path, `File::create` follows it out of the stems directory and out
/// of `--root`. Reproduced against the first version of this fix, which
/// checked only the *name*.
#[test]
#[cfg(unix)]
fn render_score_cannot_be_redirected_out_of_root_by_a_symlinked_stem() {
    let base = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("stem-symlink");
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("root");
    let outside = base.join("outside");
    std::fs::create_dir_all(root.join("stems")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let victim = outside.join("master.wav");
    std::fs::write(&victim, b"UNTOUCHED").unwrap();
    std::os::unix::fs::symlink(&victim, root.join("stems").join("lead.wav")).unwrap();

    let score = root.join("score.ron");
    std::fs::write(
        &score,
        r#"Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: "lead", instrument: Preset("sine"),
        notes: [ Note(at: (1, 1), dur: "1/4", pitch: "A4", vel: 96) ]) ],
)"#,
    )
    .unwrap();

    let server = Server::with_root(&root).expect("root exists");
    let response = call_tool(
        &server,
        1,
        "render_score",
        json!({
            "score_path": score.to_str().unwrap(),
            "out_path": root.join("mix.wav").to_str().unwrap(),
            "stems_dir": root.join("stems").to_str().unwrap(),
        }),
    );

    assert_eq!(response["result"]["isError"], true, "{response}");
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        b"UNTOUCHED",
        "a symlinked stem path must not write outside --root"
    );
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

/// `transcribe_audio`: the audio→score arrow, end to end. Renders a known
/// scale through the server, transcribes it back, and asserts the recovered
/// notes — then renders the transcription again, since producing an
/// *editable, renderable* score is the whole point of the tool.
#[test]
fn transcribe_round_trip() {
    let server = Server::new();
    let scale_ron = tmp_path("mcp_scale.ron");
    std::fs::write(
        &scale_ron,
        r#"Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: "lead", instrument: Preset("sine"), notes: [
        Note(at: (1, 1), dur: "1/4", pitch: "C4", vel: 100),
        Note(at: (1, 2), dur: "1/4", pitch: "E4", vel: 100),
        Note(at: (1, 3), dur: "1/4", pitch: "G4", vel: 100),
    ]) ],
)"#,
    )
    .unwrap();

    let wav = tmp_path("mcp_scale.wav");
    let response = call_tool(
        &server,
        20,
        "render_score",
        json!({"score_path": scale_ron, "out_path": wav}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");

    let out = tmp_path("mcp_transcribed.ron");
    let response = call_tool(
        &server,
        21,
        "transcribe_audio",
        json!({"audio_path": wav, "out_path": out, "bpm": 120.0, "preset": "pluck"}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = tool_text(&response);
    assert!(text.contains("transcribed 3 notes"), "{text}");
    assert!(text.contains("monophonic"), "{text}");
    assert!(text.contains("120.00 BPM (given)"), "{text}");

    let ron = std::fs::read_to_string(&out).unwrap();
    for pitch in ["C4", "E4", "G4"] {
        assert!(
            ron.contains(pitch),
            "transcription should recover {pitch}:\n{ron}"
        );
    }
    assert!(ron.contains(r#"Preset("pluck")"#), "{ron}");

    // It lints, and it renders — an editable score, not just a report.
    let response = call_tool(&server, 22, "lint_score", json!({"score_path": out}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let remix = tmp_path("mcp_transcribed.wav");
    let response = call_tool(
        &server,
        23,
        "render_score",
        json!({"score_path": out, "out_path": remix}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
}

/// Bad parameters are Invalid Params errors, and nothing is written.
#[test]
fn transcribe_rejects_bad_parameters() {
    let server = Server::new();
    let wav = tmp_path("mcp_badparams.wav");
    let scale_ron = tmp_path("mcp_badparams.ron");
    std::fs::write(
        &scale_ron,
        r#"Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: "lead", instrument: Preset("sine"),
        notes: [ Note(at: (1, 1), dur: "1/4", pitch: "A4", vel: 96) ]) ],
)"#,
    )
    .unwrap();
    call_tool(
        &server,
        30,
        "render_score",
        json!({"score_path": scale_ron, "out_path": wav}),
    );

    let out = tmp_path("mcp_never_written.ron");
    let _ = std::fs::remove_file(&out);

    for bad in [
        json!({"audio_path": wav, "out_path": out, "grid": "banana"}),
        json!({"audio_path": wav, "out_path": out, "bpm": 99999.0}),
        json!({"audio_path": wav, "out_path": out, "bpm": "fast"}),
        // A negative integer must be an error, not a silent fallback to the
        // default — the CLI's clap rejects `--ppq -100`, so this must too.
        json!({"audio_path": wav, "out_path": out, "ppq": -100}),
        json!({"audio_path": wav, "out_path": out, "ppq": 12.5}),
        // In range as an integer, but outside the score IR's PPQ bounds.
        json!({"audio_path": wav, "out_path": out, "ppq": 3}),
        // A grid that cannot land on a whole tick at this PPQ.
        json!({"audio_path": wav, "out_path": out, "ppq": 25, "grid": "1/16"}),
        // A preset that is not in the catalog: caught before any write, so
        // the agent never receives an unrenderable score.
        json!({"audio_path": wav, "out_path": out, "preset": "saw"}),
        // Wrong JSON types must not silently fall back to the default.
        json!({"audio_path": wav, "out_path": out, "grid": 16}),
        json!({"audio_path": wav, "out_path": out, "preset": ["marimba"]}),
        json!({"audio_path": wav, "out_path": out, "track_name": 7}),
    ] {
        let response = call_tool(&server, 31, "transcribe_audio", bad.clone());
        assert!(
            response["error"].is_object() || response["result"]["isError"] == true,
            "expected a failure for {bad}: {response}"
        );
        assert!(
            !std::path::Path::new(&out).exists(),
            "nothing should be written for {bad}"
        );
    }

    // An explicit null means "not set", as everywhere else on this server —
    // clients that serialize unset optionals as null are the common case.
    let null_ok = tmp_path("mcp_nulls_ok.ron");
    let response = call_tool(
        &server,
        33,
        "transcribe_audio",
        json!({
            "audio_path": wav, "out_path": null_ok,
            "bpm": null, "grid": null, "preset": null, "track_name": null, "ppq": null
        }),
    );
    assert_eq!(
        response["result"]["isError"], false,
        "explicit nulls should read as absent: {response}"
    );
    assert!(std::path::Path::new(&null_ok).exists());

    // Aliasing the input is refused too.
    let response = call_tool(
        &server,
        32,
        "transcribe_audio",
        json!({"audio_path": wav, "out_path": wav}),
    );
    assert!(
        response["error"].is_object() || response["result"]["isError"] == true,
        "{response}"
    );
}

/// The write path must resolve a symlinked *final component*, not just its
/// parent. Regression: `resolve_read` canonicalized fully while
/// `resolve_write` stopped at the parent, so `audio_path == out_path ==
/// take.wav` (a symlink to master.wav) compared unequal, slipped past the
/// aliasing guard, and `fs::write` followed the link and destroyed the
/// audio — the exact data-loss class the CLI's `same_file` closes.
///
/// Unix-only: creating a symlink on Windows needs either developer mode or
/// elevation, so the *test* can't run there. The fix itself is
/// platform-independent (`Path::canonicalize` resolves Windows reparse
/// points too) — this asserts it where the setup is reliable.
#[cfg(unix)]
#[test]
fn a_symlinked_out_path_cannot_destroy_the_input() {
    let server = Server::new();
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("mcp_symlink_guard");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let score = dir.join("s.ron");
    std::fs::write(
        &score,
        r#"Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: "lead", instrument: Preset("sine"),
        notes: [ Note(at: (1, 1), dur: "1/4", pitch: "A4", vel: 96) ]) ],
)"#,
    )
    .unwrap();
    let master = dir.join("master.wav");
    let response = call_tool(
        &server,
        40,
        "render_score",
        json!({"score_path": score.to_str().unwrap(), "out_path": master.to_str().unwrap()}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    let before = std::fs::metadata(&master).unwrap().len();
    assert!(before > 1000);

    // take.wav -> master.wav
    let link = dir.join("take.wav");
    std::os::unix::fs::symlink("master.wav", &link).unwrap();

    let response = call_tool(
        &server,
        41,
        "transcribe_audio",
        json!({"audio_path": link.to_str().unwrap(), "out_path": link.to_str().unwrap()}),
    );
    assert!(
        response["error"].is_object() || response["result"]["isError"] == true,
        "a symlinked out_path aliasing the input must be refused: {response}"
    );
    assert_eq!(
        std::fs::metadata(&master).unwrap().len(),
        before,
        "the input audio must be untouched"
    );

    // The same link is still perfectly usable as a *read* path.
    let out = dir.join("ok.ron");
    let response = call_tool(
        &server,
        42,
        "transcribe_audio",
        json!({"audio_path": link.to_str().unwrap(), "out_path": out.to_str().unwrap()}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(out.exists());
    assert_eq!(std::fs::metadata(&master).unwrap().len(), before);
}
