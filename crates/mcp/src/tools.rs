//! The tool set: schemas for `tools/list` and the implementations
//! `tools/call` dispatches to. Each tool mirrors one `cochlea` CLI
//! subcommand's semantics exactly (`crates/cli/src/main.rs`) over the same
//! library calls — this is a second front end onto the same offline
//! pipeline, not a reimplementation of it.
//!
//! Adding a tool is one [`schemas`] entry plus one function here plus one
//! match arm in `server::Server::tools_call`.

use std::path::Path;

use cochlea_score::Severity;
use cochlea_verify::VerifyExt;
use serde_json::{Value, json};

/// A tool's result before it's wrapped into the `tools/call` envelope.
pub enum ToolOutcome {
    /// Ran to completion; `isError: false`.
    Ok(String),
    /// Ran, but the *tool-level* operation failed (bad score, verify
    /// failure, render error) — still `tools/call`'s success response,
    /// per the spec, but `isError: true` with the reason as text.
    Failed(String),
    /// The arguments were unusable before any work could start (missing
    /// required field, unknown tool name) — surfaced as a JSON-RPC
    /// Invalid Params error, not a tool result.
    InvalidParams(String),
}

/// The `inputSchema` + `description` for every tool, in `tools/list` order.
/// Descriptions are written for an LLM caller: what the tool is for and
/// when to reach for it, not just its parameters.
pub fn schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "render_score",
            "description": "Render a cochlea RON score (the declarative tick/track/note/automation IR) to a deterministic WAV mix. Use this to turn a composed score into audible PCM before probing or inspecting it. Set verify=true to also run the score's embedded `verify:` assertions and get the pass/fail report back in the same call.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "score_path": {
                        "type": "string",
                        "description": "Path to a RON score file (data form version 1)."
                    },
                    "out_path": {
                        "type": "string",
                        "description": "Where to write the rendered mix (32-bit float stereo WAV)."
                    },
                    "stems_dir": {
                        "type": "string",
                        "description": "Optional directory to also write one WAV per track (created if missing)."
                    },
                    "verify": {
                        "type": "boolean",
                        "description": "Run the score's embedded verify: assertions after rendering and include the report; the tool call reports isError:true if verification fails. Default false.",
                        "default": false
                    }
                },
                "required": ["score_path", "out_path"]
            }
        }),
        json!({
            "name": "probe_audio",
            "description": "Extract the full feature report (integrated LUFS/true peak/LRA, onsets, YIN pitch track, chroma/key, tempo with a clear_rhythm flag, stereo image, structural sections, silence, clipping — schema v2) from any WAV or FLAC file, no score needed. Use this to 'listen' to audio through numbers: check loudness targets, confirm onset timing or tempo, or read back pitch/key/stereo width.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "audio_path": {
                        "type": "string",
                        "description": "Path to a WAV (8/16/24/32-bit PCM or 32-bit float) or FLAC file."
                    }
                },
                "required": ["audio_path"]
            }
        }),
        json!({
            "name": "spectrogram",
            "description": "Render a mel spectrogram PNG (or a tiled contact sheet covering the whole file) from a WAV or FLAC file, for visual inspection of harmonic content, sweeps, or silence. Use this when a numeric probe report isn't enough and you want to look at the audio.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "audio_path": {
                        "type": "string",
                        "description": "Path to the input WAV or FLAC."
                    },
                    "out_path": {
                        "type": "string",
                        "description": "Where to write the output PNG."
                    },
                    "sheet": {
                        "type": "boolean",
                        "description": "Tile the piece into a contact sheet instead of one long strip — useful for reviewing a whole piece in one vision call. Default false.",
                        "default": false
                    },
                    "bars_per_tile": {
                        "type": "integer",
                        "description": "Time sections per tile when sheet is true. Default 8.",
                        "default": 8
                    }
                },
                "required": ["audio_path", "out_path"]
            }
        }),
        json!({
            "name": "lint_score",
            "description": "Statically validate a RON score against the instrument/preset catalog — catches unknown instruments or inserts, empty tracks, and other semantic problems without rendering any audio. Use this before render_score to fail fast on authoring mistakes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "score_path": {
                        "type": "string",
                        "description": "Path to a RON score file."
                    }
                },
                "required": ["score_path"]
            }
        }),
        json!({
            "name": "probe_digest",
            "description": "The token-cheap way to listen to a WAV or FLAC file: a ~40-line deterministic text digest (duration, loudness, onsets, pitch, key, and a windowed timeline table) instead of a full JSON report or raw PCM. Reach for this first when you just need a sense of what's in a file, and only fall back to probe_audio when you need exact numbers to assert against.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "audio_path": {
                        "type": "string",
                        "description": "Path to a WAV or FLAC file."
                    },
                    "window_ms": {
                        "type": "number",
                        "description": "Segment window length, milliseconds, for the digest's timeline rows. Default 1000.",
                        "default": 1000
                    }
                },
                "required": ["audio_path"]
            }
        }),
        json!({
            "name": "audio_diff",
            "description": "Compare two audio files (WAV or FLAC) in feature space (loudness, onsets, pitch, key, per-segment RMS) rather than byte-for-byte, and report a verdict: byte-identical, tier-2 equivalent (within this workspace's cross-platform tolerances), or different (naming which dimensions diverge). Use this to check whether a re-render, edit, or platform change actually altered the audio in a way that matters — a `different` verdict is a normal, successful answer, not a tool failure.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "audio_path_a": {
                        "type": "string",
                        "description": "Path to the first audio file (WAV or FLAC)."
                    },
                    "audio_path_b": {
                        "type": "string",
                        "description": "Path to the second audio file (WAV or FLAC)."
                    },
                    "window_ms": {
                        "type": "number",
                        "description": "Segment window length, milliseconds, for the per-segment comparison. Default 1000.",
                        "default": 1000
                    },
                    "json": {
                        "type": "boolean",
                        "description": "Also append the full CompareReport as pretty JSON after the text summary. Default false.",
                        "default": false
                    }
                },
                "required": ["audio_path_a", "audio_path_b"]
            }
        }),
    ]
}

/// Pulls a required string argument, or an [`ToolOutcome::InvalidParams`]
/// naming it.
fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolOutcome> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolOutcome::InvalidParams(format!("missing required argument {key:?}")))
}

fn bool_or(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn usize_or(args: &Value, key: &str, default: usize) -> usize {
    // `as_u64` alone would silently ignore a JSON number lexed with a
    // decimal point — serde_json stores `8.0` as a float — so also accept
    // integral non-negative floats rather than dropping to the default.
    args.get(key)
        .and_then(|v| {
            v.as_u64().or_else(|| {
                v.as_f64()
                    .filter(|f| f.fract() == 0.0 && *f >= 0.0)
                    .map(|f| f as u64)
            })
        })
        .map_or(default, |v| v as usize)
}

fn f64_or(args: &Value, key: &str, default: f64) -> f64 {
    args.get(key).and_then(Value::as_f64).unwrap_or(default)
}

/// `window_ms` for the segment-timeline tools — validation delegates to
/// the library's single rule (`cochlea_features::validate_window_ms`);
/// JSON can't express NaN, but it can express 0.001.
fn window_ms_or_invalid(args: &Value) -> Result<f64, ToolOutcome> {
    let v = f64_or(args, "window_ms", 1000.0);
    cochlea_features::validate_window_ms(v)
        .map_err(|reason| ToolOutcome::InvalidParams(format!("window_ms {reason}")))
}

/// `render_score`: mirrors `cochlea render` (`crates/cli/src/main.rs`
/// `Cmd::Render`), minus `--report` (the report is always inlined into the
/// text result here, never written to a file — the caller is an agent, not
/// a shell pipeline).
pub fn render_score(args: &Value) -> ToolOutcome {
    let score_path = match require_str(args, "score_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let out_path = match require_str(args, "out_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let stems_dir = args.get("stems_dir").and_then(Value::as_str);
    let verify = bool_or(args, "verify", false);

    // The score is fully read before out_path is written, so an equal path
    // would "succeed" while destroying the caller's score file — reject it
    // before any work starts.
    if out_path == score_path {
        return ToolOutcome::InvalidParams(
            "out_path must not equal score_path (the WAV write would overwrite the score)"
                .to_string(),
        );
    }

    let text = match std::fs::read_to_string(score_path) {
        Ok(text) => text,
        Err(err) => return ToolOutcome::Failed(format!("reading {score_path}: {err}")),
    };
    let score = match cochlea_score::Score::from_ron(&text) {
        Ok(score) => score,
        Err(err) => return ToolOutcome::Failed(format!("parsing {score_path}: {err}")),
    };
    let rendered = match cochlea_render::render(&score) {
        Ok(rendered) => rendered,
        Err(err) => return ToolOutcome::Failed(format!("rendering {score_path}: {err}")),
    };
    if let Err(err) = rendered.write_wav(out_path) {
        return ToolOutcome::Failed(format!("writing {out_path}: {err}"));
    }
    if let Some(dir) = stems_dir
        && let Err(err) = rendered.write_stems(dir)
    {
        return ToolOutcome::Failed(format!("writing stems to {dir}: {err}"));
    }

    let frames = rendered.frames();
    let sample_rate = rendered.sample_rate().0;
    let duration_s = frames as f64 / f64::from(sample_rate);

    let mut summary = format!(
        "rendered {frames} frames ({duration_s:.3} s) at {sample_rate} Hz, 2ch -> {out_path}\n"
    );
    match peak_dbfs(rendered.mix()) {
        Some(peak) => summary.push_str(&format!("peak: {peak:.2} dBFS\n")),
        None => summary.push_str("peak: (silence)\n"),
    }
    if let Some(dir) = stems_dir {
        summary.push_str(&format!("stems written to {dir}\n"));
    }

    if verify {
        let report = rendered
            .verify(&score)
            .with_specs(score.verify_specs())
            .run();
        let report_json = serde_json::to_string_pretty(&report)
            .unwrap_or_else(|err| format!("(failed to serialize verify report: {err})"));
        summary.push_str("\nverify report:\n");
        summary.push_str(&report_json);
        if !report.passed {
            return ToolOutcome::Failed(summary);
        }
    }
    ToolOutcome::Ok(summary)
}

/// Peak amplitude of an interleaved mix in dBFS (`20*log10`, via `libm` —
/// this is response-summary display math, not a DSP path, but there's no
/// reason to reach for std transcendentals when libm is already a
/// dependency and gives the same cross-platform-stable answer). `None` for
/// digital silence, mirroring `cochlea_features`' convention of never
/// emitting a `-inf` dB value.
fn peak_dbfs(samples: &[f32]) -> Option<f64> {
    let peak = samples.iter().fold(0.0f32, |max, &s| max.max(s.abs()));
    if peak > 0.0 {
        Some(20.0 * libm::log10(f64::from(peak)))
    } else {
        None
    }
}

/// `probe_audio`: mirrors `cochlea probe` (`crates/cli/src/main.rs`
/// `Cmd::Probe`) without the `--spectro` side effect — call `spectrogram`
/// separately for that, so each tool does exactly one thing.
pub fn probe_audio(args: &Value) -> ToolOutcome {
    let audio_path = match require_str(args, "audio_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let audio = match cochlea_decode::load(Path::new(audio_path)) {
        Ok(audio) => audio,
        Err(err) => return ToolOutcome::Failed(format!("reading {audio_path}: {err}")),
    };
    let report = cochlea_features::probe(&audio, &cochlea_features::ProbeOpts::default());
    match serde_json::to_string_pretty(&report) {
        Ok(text) => ToolOutcome::Ok(text),
        Err(err) => ToolOutcome::Failed(format!("serializing report: {err}")),
    }
}

/// `spectrogram`: mirrors `cochlea spectro` (`crates/cli/src/main.rs`
/// `Cmd::Spectro` / `write_spectro`) — plain spectrogram or contact sheet,
/// no markers (those need score context via `render_score`, per the CLI's
/// own `--bars-per-tile` doc comment).
pub fn spectrogram(args: &Value) -> ToolOutcome {
    let audio_path = match require_str(args, "audio_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let out_path = match require_str(args, "out_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let sheet = bool_or(args, "sheet", false);
    let bars_per_tile = usize_or(args, "bars_per_tile", 8);

    // Same protection as render_score: the audio is fully decoded before
    // out_path is written, so an equal path destroys the input while the
    // call still "succeeds".
    if out_path == audio_path {
        return ToolOutcome::InvalidParams(
            "out_path must not equal audio_path (the PNG write would overwrite the audio)"
                .to_string(),
        );
    }

    let audio = match cochlea_decode::load(Path::new(audio_path)) {
        Ok(audio) => audio,
        Err(err) => return ToolOutcome::Failed(format!("reading {audio_path}: {err}")),
    };
    let spec = cochlea_spectro::mel_spectrogram(
        &audio.samples,
        audio.channels,
        audio.sample_rate,
        &cochlea_spectro::SpectroOpts::new(),
    );
    let img = if sheet {
        cochlea_spectro::contact_sheet(&spec, &[], bars_per_tile)
    } else {
        cochlea_spectro::render_png(&spec, &[])
    };
    if let Err(err) = cochlea_spectro::write_png(&img, out_path) {
        return ToolOutcome::Failed(format!("writing {out_path}: {err}"));
    }
    ToolOutcome::Ok(format!(
        "spectrogram -> {out_path} ({}x{})",
        img.width(),
        img.height()
    ))
}

/// `lint_score`: mirrors `cochlea lint` (`crates/cli/src/main.rs`
/// `Cmd::Lint`) — errors (not warnings) are what make the tool call
/// `isError: true`, matching the CLI's exit-1 threshold.
pub fn lint_score(args: &Value) -> ToolOutcome {
    let score_path = match require_str(args, "score_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let text = match std::fs::read_to_string(score_path) {
        Ok(text) => text,
        Err(err) => return ToolOutcome::Failed(format!("reading {score_path}: {err}")),
    };
    let score = match cochlea_score::Score::from_ron(&text) {
        Ok(score) => score,
        Err(err) => return ToolOutcome::Failed(format!("parsing {score_path}: {err}")),
    };
    let findings = score.validate(&cochlea_synth::PatchBank::presets());
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();

    if findings.is_empty() {
        return ToolOutcome::Ok("ok: no lint findings".to_string());
    }
    let text = serde_json::to_string_pretty(&findings)
        .unwrap_or_else(|err| format!("(failed to serialize findings: {err})"));
    if errors > 0 {
        ToolOutcome::Failed(text)
    } else {
        ToolOutcome::Ok(text)
    }
}

/// `probe_digest`: the token-cheap sibling of `probe_audio` — a full probe
/// plus segment timeline, rendered through `cochlea_features::digest_text`
/// instead of returned as JSON.
pub fn probe_digest(args: &Value) -> ToolOutcome {
    let audio_path = match require_str(args, "audio_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let window_ms = match window_ms_or_invalid(args) {
        Ok(v) => v,
        Err(outcome) => return outcome,
    };

    let audio = match cochlea_decode::load(Path::new(audio_path)) {
        Ok(audio) => audio,
        Err(err) => return ToolOutcome::Failed(format!("reading {audio_path}: {err}")),
    };
    let report = cochlea_features::probe(&audio, &cochlea_features::ProbeOpts::default());
    let timeline = cochlea_features::segment_timeline(
        &audio,
        &cochlea_features::SegmentOpts::default().with_window_ms(window_ms),
    );
    ToolOutcome::Ok(cochlea_features::digest_text(&report, &timeline))
}

/// `audio_diff`: feature-space comparison of two WAVs. A `Different`
/// verdict is a valid, successful answer — only a real read failure on
/// either side makes this a tool-level [`ToolOutcome::Failed`].
pub fn audio_diff(args: &Value) -> ToolOutcome {
    let path_a = match require_str(args, "audio_path_a") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let path_b = match require_str(args, "audio_path_b") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let window_ms = match window_ms_or_invalid(args) {
        Ok(v) => v,
        Err(outcome) => return outcome,
    };
    let want_json = bool_or(args, "json", false);

    let audio_a = match cochlea_decode::load(Path::new(path_a)) {
        Ok(audio) => audio,
        Err(err) => return ToolOutcome::Failed(format!("reading {path_a}: {err}")),
    };
    let audio_b = match cochlea_decode::load(Path::new(path_b)) {
        Ok(audio) => audio,
        Err(err) => return ToolOutcome::Failed(format!("reading {path_b}: {err}")),
    };

    let opts = cochlea_features::SegmentOpts::default().with_window_ms(window_ms);
    let report_a = cochlea_features::probe(&audio_a, &cochlea_features::ProbeOpts::default());
    let timeline_a = cochlea_features::segment_timeline(&audio_a, &opts);
    let report_b = cochlea_features::probe(&audio_b, &cochlea_features::ProbeOpts::default());
    let timeline_b = cochlea_features::segment_timeline(&audio_b, &opts);

    let byte_identical = cochlea_features::samples_identical(&audio_a, &audio_b);
    let compare = cochlea_features::compare_with_identity(
        cochlea_features::Analysis {
            report: &report_a,
            timeline: &timeline_a,
        },
        cochlea_features::Analysis {
            report: &report_b,
            timeline: &timeline_b,
        },
        byte_identical,
    );

    let mut text = cochlea_features::compare_text(&compare);
    if want_json {
        let compare_json = serde_json::to_string_pretty(&compare)
            .unwrap_or_else(|err| format!("(failed to serialize compare report: {err})"));
        text.push_str("\n\n");
        text.push_str(&compare_json);
    }
    ToolOutcome::Ok(text)
}
