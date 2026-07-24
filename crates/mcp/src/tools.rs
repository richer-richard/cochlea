//! The tool set: schemas for `tools/list` and the implementations
//! `tools/call` dispatches to. Each tool mirrors one `cochlea` CLI
//! subcommand's semantics exactly (`crates/cli/src/main.rs`) over the same
//! library calls — this is a second front end onto the same offline
//! pipeline, not a reimplementation of it.
//!
//! Adding a tool is one [`schemas`] entry plus one function here plus one
//! match arm in `server::Server::tools_call`.

use std::path::{Path, PathBuf};

use cochlea_score::Severity;
use cochlea_verify::VerifyExt;
use serde_json::{Value, json};

/// A tool's result before it's wrapped into the `tools/call` envelope.
#[derive(Debug)]
pub enum ToolOutcome {
    /// Ran to completion; `isError: false`.
    Ok(String),
    /// Ran to completion with pre-built content blocks (e.g. an inline
    /// image plus a text summary); `isError: false`.
    OkContent(Vec<Value>),
    /// Ran, but the *tool-level* operation failed (bad score, verify
    /// failure, render error) — still `tools/call`'s success response,
    /// per the spec, but `isError: true` with the reason as text.
    Failed(String),
    /// The arguments were unusable before any work could start (missing
    /// required field, unknown tool name, a path outside `--root`) —
    /// surfaced as a JSON-RPC Invalid Params error, not a tool result.
    InvalidParams(String),
}

/// Per-server tool context: the optional `--root` confinement directory
/// (canonicalized at startup). When set, every caller-supplied path —
/// reads and writes alike — must resolve inside it; the server refuses
/// anything else before touching the filesystem. Best-effort protection
/// against a confused or prompt-injected *client*, not against a hostile
/// local process (canonicalize-then-open is not TOCTOU-proof).
#[derive(Default)]
pub struct ToolCtx {
    pub root: Option<PathBuf>,
}

impl ToolCtx {
    /// Resolve a path that will be read: it must exist (canonicalization
    /// requires that) and, under `--root`, sit inside the root.
    fn resolve_read(&self, raw: &str, what: &str) -> Result<PathBuf, ToolOutcome> {
        let canonical = Path::new(raw)
            .canonicalize()
            .map_err(|err| ToolOutcome::Failed(format!("reading {raw}: {err}")))?;
        self.check_root(&canonical, raw, what)?;
        Ok(canonical)
    }

    /// Resolve a path that will be written: its *parent* must exist and
    /// canonicalize (the file itself usually doesn't exist yet), and the
    /// resulting parent-canonical + file-name path must sit inside the
    /// root. Rejects paths with no file name (`..`, a bare directory).
    fn resolve_write(&self, raw: &str, what: &str) -> Result<PathBuf, ToolOutcome> {
        let path = Path::new(raw);
        let Some(name) = path.file_name() else {
            return Err(ToolOutcome::InvalidParams(format!(
                "{what} {raw:?} has no file name"
            )));
        };
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let canonical_parent = parent
            .canonicalize()
            .map_err(|err| ToolOutcome::Failed(format!("resolving {what} {raw}: {err}")))?;
        let canonical = canonical_parent.join(name);
        self.check_root(&canonical, raw, what)?;
        Ok(canonical)
    }

    fn check_root(&self, canonical: &Path, raw: &str, what: &str) -> Result<(), ToolOutcome> {
        if let Some(root) = &self.root
            && !canonical.starts_with(root)
        {
            return Err(ToolOutcome::InvalidParams(format!(
                "{what} {raw:?} resolves outside this server's --root ({})",
                root.display()
            )));
        }
        Ok(())
    }
}

/// Inline-image size cap, bytes of raw PNG (~933 KB after base64) — under
/// typical MCP client message limits. Larger spectrograms are file-only.
const INLINE_PNG_CAP: usize = 700_000;

/// Standard (RFC 4648) base64, hand-rolled: three dependency-free dozen
/// lines beat pulling a crate into a supply-chain-sensitive audio
/// workspace for one encode call. Tested against RFC vectors in this
/// module's tests.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
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
                        "description": "Where to write the rendered mix (stereo WAV)."
                    },
                    "bits": {
                        "type": "string",
                        "enum": ["float", "24", "16"],
                        "description": "PCM encoding for the WAV: 'float' (32-bit, lossless, the render's ground truth — default), '24', or '16' (integer, for a small ordinary file).",
                        "default": "float"
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
            "description": "Extract the full feature report (integrated LUFS/true peak/LRA, onsets, YIN pitch track plus quantized melody notes, MFCC timbre digest, chroma/key, a chord timeline and per-section key (harmony), tempo with octave-alternative candidates and stability, rhythm with grid alignment and a straight-vs-triplet grid call, stereo image, structural sections, silence, clipping — schema v5) from any WAV, FLAC, mp3, or ogg file, no score needed. Use this to 'listen' to audio through numbers: check loudness targets, confirm onset timing or tempo, read back the melody you composed, or see the chord progression. Pass from_s/to_s to zoom into a time window instead of probing the whole file (report times are then relative to the cut; source.start_ms anchors them).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "audio_path": {
                        "type": "string",
                        "description": "Path to a WAV (8/16/24/32-bit PCM or 32-bit float), FLAC, mp3, or ogg file."
                    },
                    "from_s": {
                        "type": "number",
                        "description": "Optional: analyze only from this time (seconds into the file)."
                    },
                    "to_s": {
                        "type": "number",
                        "description": "Optional: analyze only up to this time (seconds into the file)."
                    }
                },
                "required": ["audio_path"]
            }
        }),
        json!({
            "name": "spectrogram",
            "description": "Render a mel spectrogram (or a tiled contact sheet covering the whole file) from a WAV, FLAC, mp3, or ogg file, for visual inspection of harmonic content, sweeps, or silence. The image comes back inline as MCP image content (base64 PNG) whenever it fits the size cap, so you can look at it directly without filesystem access; pass out_path to also (or instead) write the PNG to disk. Set annotate=true to draw what the analyzers heard onto the image — detected beats (orange ticks, top), onsets (cyan ticks, bottom), pitch segments (magenta lines) — and from_s/to_s to zoom into a time window. Use this when a numeric probe report isn't enough and you want to look at the audio.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "audio_path": {
                        "type": "string",
                        "description": "Path to the input WAV, FLAC, mp3, or ogg."
                    },
                    "out_path": {
                        "type": "string",
                        "description": "Optional: also write the PNG here. Required in practice only when the image exceeds the inline size cap (the call says so if that happens)."
                    },
                    "sheet": {
                        "type": "boolean",
                        "description": "Tile the piece into a contact sheet instead of one long strip — useful for reviewing a whole piece in one vision call. Default false. Incompatible with annotate.",
                        "default": false
                    },
                    "bars_per_tile": {
                        "type": "integer",
                        "description": "Time sections per tile when sheet is true. Default 8.",
                        "default": 8
                    },
                    "annotate": {
                        "type": "boolean",
                        "description": "Draw analysis overlays (beat grid, onsets, pitch) on the image. Default false.",
                        "default": false
                    },
                    "from_s": {
                        "type": "number",
                        "description": "Optional: render only from this time (seconds into the file)."
                    },
                    "to_s": {
                        "type": "number",
                        "description": "Optional: render only up to this time (seconds into the file)."
                    }
                },
                "required": ["audio_path"]
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
            "name": "score_reference",
            "description": "The score-authoring reference: the complete RON score grammar (tracks, notes, durations, automation, easing), the live instrument-preset catalog with every automatable parameter and range, all embeddable verify: assertions, and a worked example. Call this FIRST when composing — everything render_score accepts is documented here; do not guess the format.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "audio_diff",
            "description": "Compare two audio files (WAV, FLAC, mp3, or ogg) in feature space (loudness, onsets, pitch, key, timbre distance, per-segment RMS) rather than byte-for-byte, and report a verdict: byte-identical, tier-2 equivalent (within this workspace's cross-platform tolerances), or different (naming which dimensions diverge). Set spectrogram=true to also get a signed A→B difference heat map inline (red = louder in B, blue = quieter, black = unchanged) — 'what changed' as visible structure. Use this to check whether a re-render, edit, or platform change actually altered the audio in a way that matters — a `different` verdict is a normal, successful answer, not a tool failure.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "audio_path_a": {
                        "type": "string",
                        "description": "Path to the first audio file (WAV, FLAC, mp3, or ogg)."
                    },
                    "audio_path_b": {
                        "type": "string",
                        "description": "Path to the second audio file (WAV, FLAC, mp3, or ogg)."
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
                    },
                    "spectrogram": {
                        "type": "boolean",
                        "description": "Also return the signed A→B difference spectrogram as inline image content (requires both files to share a sample rate). Default false.",
                        "default": false
                    }
                },
                "required": ["audio_path_a", "audio_path_b"]
            }
        }),
        json!({
            "name": "import_midi",
            "description": "Convert a Standard MIDI File (format 0 or 1) into a cochlea RON score. Timing imports exactly (SMF ticks become score ticks, tempo events become the tempo map); General MIDI programs map to rough preset families and channel-10 percussion to kick/snare/hat tracks — every mapping guess comes back in the response so you can re-voice the score afterwards. Use this to bring existing musical material into the compose→render→probe loop.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "midi_path": {
                        "type": "string",
                        "description": "Path to a .mid/.midi file (SMF format 0 or 1, metrical division)."
                    },
                    "out_path": {
                        "type": "string",
                        "description": "Where to write the imported RON score."
                    },
                    "sample_rate": {
                        "type": "integer",
                        "description": "Sample rate for the imported score (MIDI files carry none). Default 48000.",
                        "default": 48000
                    }
                },
                "required": ["midi_path", "out_path"]
            }
        }),
        json!({
            "name": "export_midi",
            "description": "Convert a cochlea RON score into a Standard MIDI File (format 1) — the inverse of import_midi. Timing exports exactly (score ticks become SMF ticks, the tempo map and time signature carry over); instruments become rough General MIDI program labels, since a synth preset isn't a GM instrument. Use this to hand a composed score to a DAW or notation tool, or to round-trip through external MIDI editing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "score_path": {
                        "type": "string",
                        "description": "Path to a RON score file (data form version 1)."
                    },
                    "out_path": {
                        "type": "string",
                        "description": "Where to write the Standard MIDI File (.mid)."
                    }
                },
                "required": ["score_path", "out_path"]
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

/// Apply the optional `from_s`/`to_s` zoom window to loaded audio —
/// mirrors the CLI's `--from/--to` semantics exactly: a no-op (offset 0)
/// when neither is given, inverted or past-the-end windows are parameter
/// errors, and the returned offset feeds `ProbeOpts::with_start_ms`.
fn apply_window(
    audio: cochlea_features::Audio,
    args: &Value,
) -> Result<(cochlea_features::Audio, f64), ToolOutcome> {
    let from = args.get("from_s").and_then(Value::as_f64);
    let to = args.get("to_s").and_then(Value::as_f64);
    if from.is_none() && to.is_none() {
        return Ok((audio, 0.0));
    }
    let from = from.unwrap_or(0.0);
    if !from.is_finite() || from < 0.0 {
        return Err(ToolOutcome::InvalidParams(format!(
            "from_s must be a non-negative number of seconds: {from}"
        )));
    }
    if let Some(to) = to
        && (!to.is_finite() || to <= from)
    {
        return Err(ToolOutcome::InvalidParams(format!(
            "to_s ({to}) must be a finite number of seconds greater than from_s ({from})"
        )));
    }
    let (cut, start_ms) = audio.window(from, to);
    if cut.frames() == 0 {
        return Err(ToolOutcome::InvalidParams(format!(
            "from_s {from} is past the end of the file"
        )));
    }
    Ok((cut, start_ms))
}

/// `render_score`: mirrors `cochlea render` (`crates/cli/src/main.rs`
/// `Cmd::Render`), minus `--report` (the report is always inlined into the
/// text result here, never written to a file — the caller is an agent, not
/// a shell pipeline).
pub fn render_score(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
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
    // Missing → the lossless float default; otherwise resolve through the one
    // shared `WavBitDepth` parser (same names the CLI and Python accept).
    let bits = match args.get("bits").and_then(Value::as_str) {
        None => cochlea_render::WavBitDepth::Float32,
        Some(s) => match s.parse::<cochlea_render::WavBitDepth>() {
            Ok(depth) => depth,
            Err(err) => return ToolOutcome::InvalidParams(err),
        },
    };

    let score_resolved = match ctx.resolve_read(score_path, "score_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let out_resolved = match ctx.resolve_write(out_path, "out_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    // The score is fully read before out_path is written, so an aliasing
    // path would "succeed" while destroying the caller's score file —
    // reject before any work starts. Canonical comparison, so `./x.ron`
    // vs `x.ron`, symlinks, and absolute/relative spellings all count.
    if out_resolved == score_resolved {
        return ToolOutcome::InvalidParams(
            "out_path must not alias score_path (the WAV write would overwrite the score)"
                .to_string(),
        );
    }
    let stems_resolved = match stems_dir {
        Some(dir) => match ctx.resolve_write(dir, "stems_dir") {
            Ok(p) => Some(p),
            Err(outcome) => return outcome,
        },
        None => None,
    };

    let text = match std::fs::read_to_string(&score_resolved) {
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
    if let Err(err) = rendered.write_wav_as(&out_resolved, bits) {
        return ToolOutcome::Failed(format!("writing {out_path}: {err}"));
    }
    if let Some(dir) = &stems_resolved
        && let Err(err) = rendered.write_stems_as(dir, bits)
    {
        return ToolOutcome::Failed(format!("writing stems to {}: {err}", dir.display()));
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
pub fn probe_audio(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    let audio_path = match require_str(args, "audio_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let resolved = match ctx.resolve_read(audio_path, "audio_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let audio = match cochlea_decode::load(&resolved) {
        Ok(audio) => audio,
        Err(err) => return ToolOutcome::Failed(format!("reading {audio_path}: {err}")),
    };
    let (audio, start_ms) = match apply_window(audio, args) {
        Ok(pair) => pair,
        Err(outcome) => return outcome,
    };
    let report = cochlea_features::probe(
        &audio,
        &cochlea_features::ProbeOpts::default().with_start_ms(start_ms),
    );
    match serde_json::to_string_pretty(&report) {
        Ok(text) => ToolOutcome::Ok(text),
        Err(err) => ToolOutcome::Failed(format!("serializing report: {err}")),
    }
}

/// `spectrogram`: mirrors `cochlea spectro` (`crates/cli/src/main.rs`
/// `Cmd::Spectro` / `write_spectro`) — plain spectrogram or contact sheet,
/// no markers (those need score context via `render_score`, per the CLI's
/// own `--bars-per-tile` doc comment) — plus the MCP-native part the CLI
/// can't do: the PNG comes back *inline* as an image content block
/// (base64) whenever it fits [`INLINE_PNG_CAP`], so a client with no
/// filesystem access still gets to look at the audio. `out_path` is
/// optional; when the image exceeds the cap and no `out_path` was given,
/// the call fails with instructions rather than silently returning
/// nothing viewable.
pub fn spectrogram(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    let audio_path = match require_str(args, "audio_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let out_path = args.get("out_path").and_then(Value::as_str);
    let sheet = bool_or(args, "sheet", false);
    let bars_per_tile = usize_or(args, "bars_per_tile", 8);
    let annotate = bool_or(args, "annotate", false);
    if annotate && sheet {
        return ToolOutcome::InvalidParams(
            "annotate and sheet are incompatible (overlays draw on the single-strip image)"
                .to_string(),
        );
    }

    let audio_resolved = match ctx.resolve_read(audio_path, "audio_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let out_resolved = match out_path {
        Some(raw) => {
            let resolved = match ctx.resolve_write(raw, "out_path") {
                Ok(p) => p,
                Err(outcome) => return outcome,
            };
            // Same protection as render_score, canonical form: the audio
            // is fully decoded before out_path is written, so an aliasing
            // path destroys the input while the call still "succeeds".
            if resolved == audio_resolved {
                return ToolOutcome::InvalidParams(
                    "out_path must not alias audio_path (the PNG write would overwrite the audio)"
                        .to_string(),
                );
            }
            Some(resolved)
        }
        None => None,
    };

    let audio = match cochlea_decode::load(&audio_resolved) {
        Ok(audio) => audio,
        Err(err) => return ToolOutcome::Failed(format!("reading {audio_path}: {err}")),
    };
    let (audio, _start_ms) = match apply_window(audio, args) {
        Ok(pair) => pair,
        Err(outcome) => return outcome,
    };
    let spec = cochlea_spectro::mel_spectrogram(
        &audio.samples,
        audio.channels,
        audio.sample_rate,
        &cochlea_spectro::SpectroOpts::new(),
    );
    let img = if sheet {
        cochlea_spectro::contact_sheet(&spec, &[], bars_per_tile)
    } else if annotate {
        cochlea_spectro::render_annotated(&spec, &[], &build_overlay(&audio))
    } else {
        cochlea_spectro::render_png(&spec, &[])
    };

    let mut summary = format!("spectrogram {}x{}", img.width(), img.height());
    if annotate {
        summary.push_str(" (annotated: beats orange/top, onsets cyan/bottom, pitch magenta)");
    }
    if let Some(out) = &out_resolved {
        if let Err(err) = cochlea_spectro::write_png(&img, out) {
            return ToolOutcome::Failed(format!("writing {}: {err}", out.display()));
        }
        summary.push_str(&format!(" -> {}", out.display()));
    }

    let png = match cochlea_spectro::encode_png(&img) {
        Ok(bytes) => bytes,
        Err(err) => return ToolOutcome::Failed(format!("encoding PNG: {err}")),
    };
    if png.len() <= INLINE_PNG_CAP {
        summary.push_str(" (inline)");
        return ToolOutcome::OkContent(vec![
            json!({
                "type": "image",
                "data": base64_encode(&png),
                "mimeType": "image/png",
            }),
            json!({"type": "text", "text": summary}),
        ]);
    }
    if out_resolved.is_none() {
        return ToolOutcome::Failed(format!(
            "image is {} bytes, over the {INLINE_PNG_CAP}-byte inline cap, and no out_path was \
             given — pass out_path (or sheet: true, which tiles long audio into a denser image)",
            png.len()
        ));
    }
    summary.push_str(" (too large to inline)");
    ToolOutcome::Ok(summary)
}

/// Overlay data for `spectrogram`'s `annotate` — the same probe/tempo
/// pass the CLI's `--annotate` runs, translated to plain samples/Hz here
/// because the spectro crate never sees feature-report types.
fn build_overlay(audio: &cochlea_features::Audio) -> cochlea_spectro::Overlay {
    let report = cochlea_features::probe(audio, &cochlea_features::ProbeOpts::default());
    let tempo = cochlea_features::estimate_tempo(audio, &cochlea_features::TempoOpts::default());
    let sr = f64::from(audio.sample_rate);
    let ms_to_sample = |ms: f64| -> u64 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "report times are non-negative and within the buffer"
        )]
        {
            (ms / 1000.0 * sr).round().max(0.0) as u64
        }
    };
    cochlea_spectro::Overlay {
        beats: tempo.beats_ms.iter().map(|&ms| ms_to_sample(ms)).collect(),
        onsets: report
            .onsets
            .times_ms
            .iter()
            .map(|&ms| ms_to_sample(ms))
            .collect(),
        pitch: report
            .pitch
            .segments
            .iter()
            .map(|s| (ms_to_sample(s.start_ms), ms_to_sample(s.end_ms), s.f0_hz))
            .collect(),
    }
}

/// `import_midi`: mirrors `cochlea import` (`crates/cli/src/main.rs`
/// `Cmd::Import`) — SMF in, RON score out, every instrument-mapping guess
/// reported in the response text.
pub fn import_midi(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    let midi_path = match require_str(args, "midi_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let out_path = match require_str(args, "out_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let sample_rate = usize_or(args, "sample_rate", 48_000);
    let Ok(sample_rate) = u32::try_from(sample_rate) else {
        return ToolOutcome::InvalidParams(format!("sample_rate {sample_rate} out of range"));
    };

    let midi_resolved = match ctx.resolve_read(midi_path, "midi_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let out_resolved = match ctx.resolve_write(out_path, "out_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    if out_resolved == midi_resolved {
        return ToolOutcome::InvalidParams(
            "out_path must not alias midi_path (the RON write would overwrite the MIDI file)"
                .to_string(),
        );
    }

    let bytes = match std::fs::read(&midi_resolved) {
        Ok(bytes) => bytes,
        Err(err) => return ToolOutcome::Failed(format!("reading {midi_path}: {err}")),
    };
    let import = match cochlea_score::import_midi(&bytes, cochlea_score::SampleRate(sample_rate)) {
        Ok(import) => import,
        Err(err) => return ToolOutcome::Failed(err.to_string()),
    };
    let ron = match import.score.to_ron() {
        Ok(ron) => ron,
        Err(err) => return ToolOutcome::Failed(format!("serializing the imported score: {err}")),
    };
    if let Err(err) = std::fs::write(&out_resolved, ron) {
        return ToolOutcome::Failed(format!("writing {out_path}: {err}"));
    }

    let mut summary = format!(
        "imported {} tracks, {} tempo changes -> {out_path}\n",
        import.score.tracks().len(),
        import.score.tempo_changes().count(),
    );
    for w in &import.warnings {
        summary.push_str(&format!("note: {w}\n"));
    }
    summary.push_str(
        "\nNext: lint_score to check it, render_score to hear it, and edit the RON to re-voice \
         the guessed presets.",
    );
    ToolOutcome::Ok(summary)
}

/// `export_midi`: mirrors `cochlea export` (`crates/cli/src/main.rs`
/// `Cmd::Export`) — RON score in, Standard MIDI File out. Timing exports
/// exactly; instruments become rough GM labels (the inverse of the importer).
pub fn export_midi(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    let score_path = match require_str(args, "score_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let out_path = match require_str(args, "out_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };

    let score_resolved = match ctx.resolve_read(score_path, "score_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let out_resolved = match ctx.resolve_write(out_path, "out_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    if out_resolved == score_resolved {
        return ToolOutcome::InvalidParams(
            "out_path must not alias score_path (the MIDI write would overwrite the score)"
                .to_string(),
        );
    }

    let text = match std::fs::read_to_string(&score_resolved) {
        Ok(text) => text,
        Err(err) => return ToolOutcome::Failed(format!("reading {score_path}: {err}")),
    };
    let score = match cochlea_score::Score::from_ron(&text) {
        Ok(score) => score,
        Err(err) => return ToolOutcome::Failed(format!("parsing {score_path}: {err}")),
    };
    let bytes = match cochlea_score::export_midi(&score) {
        Ok(bytes) => bytes,
        Err(err) => return ToolOutcome::Failed(format!("exporting {score_path}: {err}")),
    };
    if let Err(err) = std::fs::write(&out_resolved, bytes) {
        return ToolOutcome::Failed(format!("writing {out_path}: {err}"));
    }

    ToolOutcome::Ok(format!(
        "exported {} tracks, {} tempo changes -> {out_path}\nnote: timing is exact; instruments \
         are rough General MIDI labels — re-voice in your MIDI tool as needed.",
        score.tracks().len(),
        score.tempo_changes().count(),
    ))
}

/// `score_reference`: the self-describing half of the compose loop — the
/// full authoring reference generated against the live preset bank
/// (`cochlea_score::authoring_reference`), so an agent that has only this
/// server can still learn to write scores.
pub fn score_reference() -> ToolOutcome {
    ToolOutcome::Ok(cochlea_score::authoring_reference(
        &cochlea_synth::PatchBank::presets(),
    ))
}

/// `lint_score`: mirrors `cochlea lint` (`crates/cli/src/main.rs`
/// `Cmd::Lint`) — errors (not warnings) are what make the tool call
/// `isError: true`, matching the CLI's exit-1 threshold.
pub fn lint_score(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    let score_path = match require_str(args, "score_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let resolved = match ctx.resolve_read(score_path, "score_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let text = match std::fs::read_to_string(&resolved) {
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
pub fn probe_digest(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    let audio_path = match require_str(args, "audio_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let window_ms = match window_ms_or_invalid(args) {
        Ok(v) => v,
        Err(outcome) => return outcome,
    };

    let resolved = match ctx.resolve_read(audio_path, "audio_path") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let audio = match cochlea_decode::load(&resolved) {
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
pub fn audio_diff(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
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
    let want_spectrogram = bool_or(args, "spectrogram", false);

    let resolved_a = match ctx.resolve_read(path_a, "audio_path_a") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let resolved_b = match ctx.resolve_read(path_b, "audio_path_b") {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };
    let audio_a = match cochlea_decode::load(&resolved_a) {
        Ok(audio) => audio,
        Err(err) => return ToolOutcome::Failed(format!("reading {path_a}: {err}")),
    };
    let audio_b = match cochlea_decode::load(&resolved_b) {
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

    if want_spectrogram {
        let mel = |audio: &cochlea_features::Audio| {
            cochlea_spectro::mel_spectrogram(
                &audio.samples,
                audio.channels,
                audio.sample_rate,
                &cochlea_spectro::SpectroOpts::new(),
            )
        };
        let img = match cochlea_spectro::render_diff_png(&mel(&audio_a), &mel(&audio_b)) {
            Ok(img) => img,
            Err(err) => {
                return ToolOutcome::Failed(format!("rendering the difference spectrogram: {err}"));
            }
        };
        let png = match cochlea_spectro::encode_png(&img) {
            Ok(bytes) => bytes,
            Err(err) => return ToolOutcome::Failed(format!("encoding PNG: {err}")),
        };
        if png.len() <= INLINE_PNG_CAP {
            text.push_str(
                "\ndiff spectrogram inline: red = louder in B, blue = quieter in B, black = \
                 unchanged (saturates at 24 dB)",
            );
            return ToolOutcome::OkContent(vec![
                json!({
                    "type": "image",
                    "data": base64_encode(&png),
                    "mimeType": "image/png",
                }),
                json!({"type": "text", "text": text}),
            ]);
        }
        text.push_str("\n(diff spectrogram exceeded the inline size cap; omitted)");
    }
    ToolOutcome::Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648 §10 test vectors.
    #[test]
    fn base64_matches_rfc_vectors() {
        for (input, expected) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(input), expected);
        }
    }

    /// Confinement: reads and writes outside `--root` are refused before
    /// any filesystem work; aliased spellings of an inside path pass.
    #[test]
    fn tool_ctx_root_confinement() {
        let dir =
            std::env::temp_dir().join(format!("cochlea-mcp-root-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("inside")).unwrap();
        std::fs::write(dir.join("inside/a.wav"), b"x").unwrap();
        std::fs::write(dir.join("outside.wav"), b"x").unwrap();

        let ctx = ToolCtx {
            root: Some(dir.join("inside").canonicalize().unwrap()),
        };

        let inside = dir.join("inside/a.wav");
        assert!(ctx.resolve_read(inside.to_str().unwrap(), "p").is_ok());
        // A dot-dot spelling of the same inside file still resolves fine.
        let dotted = dir.join("inside/../inside/a.wav");
        assert!(ctx.resolve_read(dotted.to_str().unwrap(), "p").is_ok());

        let outside = dir.join("outside.wav");
        match ctx.resolve_read(outside.to_str().unwrap(), "p") {
            Err(ToolOutcome::InvalidParams(msg)) => {
                assert!(msg.contains("--root"), "{msg}");
            }
            _ => panic!("outside read must be refused as InvalidParams"),
        }
        // Escape-by-dot-dot on a write is refused too.
        let escape = dir.join("inside/../escape.png");
        match ctx.resolve_write(escape.to_str().unwrap(), "p") {
            Err(ToolOutcome::InvalidParams(msg)) => {
                assert!(msg.contains("--root"), "{msg}");
            }
            _ => panic!("outside write must be refused as InvalidParams"),
        }
        // Writes to a not-yet-existing file inside the root are fine.
        let new_inside = dir.join("inside/new.png");
        assert!(ctx.resolve_write(new_inside.to_str().unwrap(), "p").is_ok());
    }

    /// Unconfined (no --root): alias detection still canonicalizes, so
    /// `./x` and `x` count as the same file.
    #[test]
    fn alias_detection_is_canonical_not_string_equality() {
        let dir =
            std::env::temp_dir().join(format!("cochlea-mcp-alias-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.ron");
        std::fs::write(&f, b"x").unwrap();

        let ctx = ToolCtx::default();
        let read = ctx.resolve_read(f.to_str().unwrap(), "p").unwrap();
        let aliased = dir.join(".").join("s.ron");
        let write = ctx.resolve_write(aliased.to_str().unwrap(), "p").unwrap();
        assert_eq!(read, write, "aliased spellings must resolve equal");
    }
}
