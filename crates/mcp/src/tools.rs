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
            "name": "probe_wav",
            "description": "Extract the full feature report (integrated LUFS/true peak, onsets, YIN pitch track, chroma/key, silence, clipping) from any WAV file — no score needed. Use this to 'listen' to audio through numbers: check loudness targets, confirm onset timing, or read back pitch/key.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "wav_path": {
                        "type": "string",
                        "description": "Path to a WAV file (8/16/24/32-bit PCM or 32-bit float)."
                    }
                },
                "required": ["wav_path"]
            }
        }),
        json!({
            "name": "spectrogram",
            "description": "Render a mel spectrogram PNG (or a tiled contact sheet covering the whole file) from a WAV, for visual inspection of harmonic content, sweeps, or silence. Use this when a numeric probe report isn't enough and you want to look at the audio.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "wav_path": {
                        "type": "string",
                        "description": "Path to the input WAV."
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
                "required": ["wav_path", "out_path"]
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
    args.get(key)
        .and_then(Value::as_u64)
        .map_or(default, |v| v as usize)
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

/// `probe_wav`: mirrors `cochlea probe` (`crates/cli/src/main.rs`
/// `Cmd::Probe`) without the `--spectro` side effect — call `spectrogram`
/// separately for that, so each tool does exactly one thing.
pub fn probe_wav(args: &Value) -> ToolOutcome {
    let wav_path = match require_str(args, "wav_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let audio = match cochlea_features::Audio::from_wav(Path::new(wav_path)) {
        Ok(audio) => audio,
        Err(err) => return ToolOutcome::Failed(format!("reading {wav_path}: {err}")),
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
    let wav_path = match require_str(args, "wav_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let out_path = match require_str(args, "out_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let sheet = bool_or(args, "sheet", false);
    let bars_per_tile = usize_or(args, "bars_per_tile", 8);

    let audio = match cochlea_features::Audio::from_wav(Path::new(wav_path)) {
        Ok(audio) => audio,
        Err(err) => return ToolOutcome::Failed(format!("reading {wav_path}: {err}")),
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
