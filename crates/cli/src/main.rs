//! The `cochlea` binary: `render`, `probe`, `lint`, `spectro`, `diff`.
//!
//! Exit codes: 0 ok, 1 verify/lint/`diff --tier2` failures, 2 usage/IO/render
//! errors (clap and anyhow errors both land on 2 via the wrapper in `main`).

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use cochlea_score::{Score, Severity};
use cochlea_synth::PatchBank;
use cochlea_verify::VerifyExt;

#[derive(Parser)]
#[command(
    name = "cochlea",
    version,
    about = "Headless audio engine for agents: compose, render, listen through numbers"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render a RON score to a WAV mix (and optionally per-track stems).
    Render {
        /// The score (RON data form, version 1).
        score: PathBuf,
        /// Output WAV path.
        #[arg(long)]
        out: PathBuf,
        /// PCM encoding: `float` (32-bit, lossless, the render's ground
        /// truth), `24`, or `16` (integer, for a small ordinary file).
        #[arg(long, default_value = "float", value_parser = parse_bit_depth)]
        bits: cochlea_render::WavBitDepth,
        /// Also write one WAV per track into this directory.
        #[arg(long)]
        stems: Option<PathBuf>,
        /// Run the score's embedded `verify:` assertions; exit 1 on failure.
        #[arg(long)]
        verify: bool,
        /// Write the verify report JSON here instead of stdout.
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Extract the feature report (and optionally a spectrogram) from an
    /// audio file (WAV or FLAC) — works on arbitrary files, no score needed.
    Probe {
        /// Input audio: WAV (f32 or 16/24/32-bit PCM) or FLAC.
        input: PathBuf,
        /// Write the JSON report here instead of stdout.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Also render a mel spectrogram PNG here.
        #[arg(long)]
        spectro: Option<PathBuf>,
        /// Print a compact text digest — sized for LLM context windows —
        /// to stdout instead of the JSON report. Combine with `--json` to
        /// get both: digest to stdout, full JSON report to the file.
        #[arg(long)]
        digest: bool,
        /// Write a windowed feature timeline (`SegmentTimeline` JSON) here.
        /// Composable with any other flag.
        #[arg(long)]
        segments: Option<PathBuf>,
        /// Write the loudness-over-time curve (`LoudnessTimeline` JSON:
        /// momentary + short-term LUFS every 100 ms) here — the dynamics
        /// view the single integrated/LRA summary can't give. Covers the
        /// whole file, times from its start (ignores --from/--to).
        #[arg(long)]
        loudness: Option<PathBuf>,
        /// Write the full beat grid (`TempoReport` JSON: every beat time,
        /// downbeats, candidates, stability) here — the detail the compact
        /// `tempo` summary in the main report drops. Covers the whole file,
        /// times from its start (ignores --from/--to).
        #[arg(long)]
        beats: Option<PathBuf>,
        /// Window length for `--digest`/`--segments`, milliseconds.
        #[arg(long, default_value_t = 1000.0, value_parser = parse_window_ms)]
        window_ms: f64,
        /// Analyze only from this time (seconds into the file) — the zoom
        /// lens: report times are then relative to the cut, with the
        /// offset recorded as `source.start_ms`.
        #[arg(long, value_parser = parse_seconds)]
        from: Option<f64>,
        /// Analyze only up to this time (seconds into the file).
        #[arg(long, value_parser = parse_seconds)]
        to: Option<f64>,
    },
    /// Feature-space diff of two audio files (WAV or FLAC) — "did my change do what I meant":
    /// loudness/onset/pitch/key deltas plus an equivalence verdict, printed
    /// as a compact text digest sized for LLM context windows.
    Diff {
        /// First input audio file (WAV or FLAC).
        a: PathBuf,
        /// Second input audio file (WAV or FLAC).
        b: PathBuf,
        /// Write the comparison JSON here.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Exit 1 unless the verdict is byte-identical or Tier-2 equivalent
        /// (the workspace's cross-platform feature tolerances).
        #[arg(long)]
        tier2: bool,
        /// Window length for the underlying segment timelines, milliseconds.
        #[arg(long, default_value_t = 1000.0, value_parser = parse_window_ms)]
        window_ms: f64,
        /// Also render a signed A→B difference spectrogram PNG here (red =
        /// louder in B, blue = quieter, black = unchanged).
        #[arg(long)]
        spectro: Option<PathBuf>,
        /// Compare only from this time (seconds), applied to both files.
        #[arg(long, value_parser = parse_seconds)]
        from: Option<f64>,
        /// Compare only up to this time (seconds), applied to both files.
        #[arg(long, value_parser = parse_seconds)]
        to: Option<f64>,
    },
    /// Score a directory of candidate audio files against a directory of
    /// reference (golden) files, matched by filename — the golden-audio /
    /// generative-model regression harness. Prints a per-file verdict table
    /// and exits 1 if any pair regressed or a reference is missing.
    Eval {
        /// Directory of candidate audio files (model/render outputs).
        #[arg(long)]
        candidates: PathBuf,
        /// Directory of reference (golden) audio files, matched by filename.
        #[arg(long)]
        references: PathBuf,
        /// Require byte-identity instead of Tier-2 feature-equivalence.
        #[arg(long)]
        exact: bool,
        /// Write the JSON eval report here.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Window length for the per-pair comparison, milliseconds.
        #[arg(long, default_value_t = 1000.0, value_parser = parse_window_ms)]
        window_ms: f64,
    },
    /// Statically validate a score against the preset catalog.
    Lint {
        /// The score (RON data form, version 1).
        score: PathBuf,
    },
    /// Render a mel spectrogram (or tiled contact sheet) from an audio file.
    Spectro {
        /// Input audio (WAV or FLAC).
        input: PathBuf,
        /// Output PNG path.
        #[arg(long)]
        out: PathBuf,
        /// Tile the piece into a contact sheet instead of one long strip.
        #[arg(long)]
        sheet: bool,
        /// Sections per tile when `--sheet` is set (time slices; bar-aware
        /// tiling uses markers, which need score context via `render`).
        #[arg(long, default_value_t = 8)]
        bars_per_tile: usize,
        /// Draw analysis overlays on the image: detected beats (orange
        /// ticks, top), onsets (cyan ticks, bottom), and pitch segments
        /// (magenta lines on the frequency axis).
        #[arg(long, conflicts_with = "sheet")]
        annotate: bool,
        /// Render only from this time (seconds into the file).
        #[arg(long, value_parser = parse_seconds)]
        from: Option<f64>,
        /// Render only up to this time (seconds into the file).
        #[arg(long, value_parser = parse_seconds)]
        to: Option<f64>,
    },
    /// Convert a Standard MIDI File (format 0 or 1) into a RON score.
    /// Timing imports exactly (SMF ticks -> score ticks, tempo events ->
    /// tempo map); instruments map to rough preset families and channel-10
    /// percussion to kick/snare/hat tracks — every guess is printed so you
    /// can re-voice.
    Import {
        /// Input MIDI file (.mid/.midi).
        input: PathBuf,
        /// Output RON score path.
        #[arg(long)]
        out: PathBuf,
        /// Sample rate for the imported score (MIDI files carry none).
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
    },
    /// Export a RON score to a Standard MIDI File (format 1). Timing is
    /// exact; instruments become rough General MIDI labels — the inverse of
    /// `import`.
    Export {
        /// Input RON score (data form, version 1).
        score: PathBuf,
        /// Output MIDI file path (.mid).
        #[arg(long)]
        out: PathBuf,
    },
    /// Transcribe an audio file into a RON score — the inverse of `render`.
    /// Pitch-tracks a monophonic melody, reads it against a tempo (detected
    /// unless you pass --bpm), quantizes to a grid, and writes an editable
    /// score. Every guess is printed: it hears one line, not an
    /// arrangement, so treat the result as a draft to re-voice.
    Transcribe {
        /// Input audio file (WAV, FLAC, mp3, or ogg).
        input: PathBuf,
        /// Output RON score path.
        #[arg(long)]
        out: PathBuf,
        /// Tempo to notate against. Detected from the audio when omitted.
        #[arg(long, value_parser = parse_bpm)]
        bpm: Option<f64>,
        /// Quantization grid as a note duration (`1/16`, `1/8`, `1/4`,
        /// `1/8t`...), or `none` to keep raw analyzer timing.
        #[arg(long, default_value = "1/16")]
        grid: String,
        /// Instrument preset for the transcribed track.
        #[arg(long, default_value = "sine")]
        preset: String,
        /// Track name in the written score.
        #[arg(long, default_value = "lead")]
        track: String,
        /// Tick resolution of the written score.
        #[arg(long, default_value_t = 960)]
        ppq: u32,
    },
    /// Print the score-authoring reference (RON grammar, instrument
    /// catalog, verify assertions, worked example) — the same text the
    /// MCP `score_reference` tool serves.
    Reference,
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("cochlea: {err:#}");
            std::process::ExitCode::from(2)
        }
    }
}

fn load_score(path: &Path) -> anyhow::Result<Score> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Score::from_ron(&text).with_context(|| format!("parsing {}", path.display()))
}

/// `--window-ms` validation delegates to the library's single rule
/// (`cochlea_features::validate_window_ms`): finite and at least 1 ms,
/// rejected at the flag boundary rather than degraded downstream.
fn parse_window_ms(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|err| format!("not a number: {err}"))?;
    cochlea_features::validate_window_ms(v)
}

/// `--bpm` validation: rejected at the flag boundary with the same bounds
/// the score IR enforces, so a bad tempo fails before any audio is read.
fn parse_bpm(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|err| format!("not a number: {err}"))?;
    cochlea_score::Bpm(v)
        .validate()
        .map(|()| v)
        .map_err(|e| e.to_string())
}

/// `--from`/`--to` validation: a finite, non-negative number of seconds.
fn parse_seconds(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|err| format!("not a number: {err}"))?;
    if !v.is_finite() || v < 0.0 {
        return Err(format!("must be a non-negative number of seconds: {v}"));
    }
    Ok(v)
}

/// Whether a path has a decodable audio extension (the `eval` file filter).
fn is_audio_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| matches!(e.as_str(), "wav" | "wave" | "flac" | "mp3" | "ogg" | "oga"))
}

/// The serde/display name of a [`cochlea_features::Verdict`].
fn verdict_name(v: cochlea_features::Verdict) -> &'static str {
    match v {
        cochlea_features::Verdict::ByteIdentical => "byte_identical",
        cochlea_features::Verdict::Tier2Equivalent => "tier2_equivalent",
        cochlea_features::Verdict::Different { .. } => "different",
    }
}

/// Feature-space compare of two audio files, returning just the verdict — the
/// per-pair unit of `eval` (mirrors `diff`'s comparison, minus the outputs).
fn compare_files(
    a: &Path,
    b: &Path,
    opts: &cochlea_features::SegmentOpts,
) -> anyhow::Result<cochlea_features::Verdict> {
    let audio_a = cochlea_decode::load(a).with_context(|| format!("reading {}", a.display()))?;
    let audio_b = cochlea_decode::load(b).with_context(|| format!("reading {}", b.display()))?;
    let probe_opts = cochlea_features::ProbeOpts::default();
    let report_a = cochlea_features::probe(&audio_a, &probe_opts);
    let report_b = cochlea_features::probe(&audio_b, &probe_opts);
    let timeline_a = cochlea_features::segment_timeline(&audio_a, opts);
    let timeline_b = cochlea_features::segment_timeline(&audio_b, opts);
    let identical = cochlea_features::samples_identical(&audio_a, &audio_b);
    let result = cochlea_features::compare_with_identity(
        cochlea_features::Analysis {
            report: &report_a,
            timeline: &timeline_a,
        },
        cochlea_features::Analysis {
            report: &report_b,
            timeline: &timeline_b,
        },
        identical,
    );
    Ok(result.verdict)
}

/// `--bits` validation: `float`/`32` → 32-bit float, `24`/`16` → integer PCM.
/// Delegates to [`cochlea_render::WavBitDepth`]'s `FromStr`, the one place the
/// encoding names are resolved (shared with the MCP and Python front doors).
fn parse_bit_depth(s: &str) -> Result<cochlea_render::WavBitDepth, String> {
    s.parse()
}

/// Resolve a path far enough to compare it with another, whether or not it
/// exists yet.
///
/// `canonicalize` only works on something already on disk, which is the
/// wrong half of the problem: the dangerous comparisons here are between
/// *outputs*, and an output does not exist when the guard runs. So fall back
/// to the shape [`cochlea_mcp`'s `resolve_write`] uses — canonicalize the
/// parent directory, which does exist, and re-attach the file name. That
/// normalizes `..`, symlinked directories, and absolute-vs-relative
/// spellings for a file that has yet to be created.
///
/// A path we cannot resolve at all is returned unchanged, so the caller
/// still gets the raw comparison rather than a false "different".
fn resolve_for_compare(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    let Some(name) = path.file_name() else {
        return path.to_path_buf();
    };
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    match std::fs::canonicalize(parent) {
        Ok(canonical_parent) => canonical_parent.join(name),
        Err(_) => path.to_path_buf(),
    }
}

/// Whether `a` and `b` name the same file on disk — the one rule every "don't
/// clobber my input" guard in this binary shares. A raw path compare first
/// (the common case), then a resolved compare so a *different spelling* of
/// the same file — `./x.wav` vs `x.wav`, a symlink, an absolute-vs-relative
/// mix, a `..` segment, or a name differing only by case — is caught too.
///
/// The resolved half deliberately does not require the files to exist. It
/// used to: both sides went through `canonicalize`, which fails for a
/// not-yet-written output, so the guard silently degraded to the raw compare
/// for exactly the pair it most needed to catch. `render --out d/stems/lead.wav
/// --stems d/stems/../stems` then wrote the mix and let the `lead` stem
/// overwrite it, at exit 0.
///
/// The comparison itself is [`cochlea_render::same_target_file`], shared with
/// the MCP server so both front doors judge "same file" identically — and,
/// crucially, case-insensitively: macOS and Windows are case-insensitive by
/// default, so `--out d/Lead.wav --stems d` with a track named `lead` wrote
/// the mix and then destroyed it with one stem, at exit 0, while this guard
/// compared the two paths unequal.
///
/// This lives in one place on purpose: the guard used to be open-coded per
/// subcommand, and only `export` had the canonical half, so `probe`/`diff`/
/// `import` could be tricked into overwriting their own input through an
/// aliased path while still exiting 0. Now every write path routes here.
fn same_file(a: &Path, b: &Path) -> bool {
    a == b || cochlea_render::same_target_file(&resolve_for_compare(a), &resolve_for_compare(b))
}

/// Apply a `--from`/`--to` window to loaded audio: no-op (offset 0) when
/// neither flag is set; an inverted or empty result is a usage error, not
/// a silent empty analysis.
fn apply_window(
    audio: cochlea_features::Audio,
    from: Option<f64>,
    to: Option<f64>,
) -> anyhow::Result<(cochlea_features::Audio, f64)> {
    if from.is_none() && to.is_none() {
        return Ok((audio, 0.0));
    }
    let from = from.unwrap_or(0.0);
    if let Some(to) = to
        && to <= from
    {
        anyhow::bail!("--to ({to}s) must be greater than --from ({from}s)");
    }
    let (cut, start_ms) = audio.window(from, to);
    if cut.frames() == 0 {
        anyhow::bail!("--from {from}s is past the end of the file");
    }
    Ok((cut, start_ms))
}

fn run() -> anyhow::Result<std::process::ExitCode> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Render {
            score,
            out,
            bits,
            stems,
            verify,
            report,
        } => {
            // The WAV and (optional) report writes must not land on the RON
            // score they read from — same guard the other subcommands use.
            for (flag, path) in [("--out", Some(&out)), ("--report", report.as_ref())] {
                if let Some(p) = path
                    && same_file(p, &score)
                {
                    anyhow::bail!("{flag} would overwrite the input score {p:?}");
                }
            }
            // ...and --report must not clobber the WAV --out just wrote (the
            // report is written after the mix, so it would silently win).
            if let Some(rp) = report.as_ref()
                && same_file(&out, rp)
            {
                anyhow::bail!("--report and --out point at the same path {out:?}");
            }
            let score_path = score.clone();
            let score = load_score(&score)?;
            // A per-track stem (`<dir>/<track>.wav`) must not land on the mix
            // (--out), the report, or the score it was read from — the same
            // overwrite class, but one path per track, so it needs the loaded
            // track names to check.
            if let Some(dir) = stems.as_ref() {
                for track in score.tracks() {
                    // A track name is score data, not a path the caller
                    // typed: unchecked, `dir.join` would let a name like
                    // `/etc/x` or `../../x` write clean outside --stems.
                    // Checked here (before the mix write) so a hostile score
                    // produces no output at all, not a mix plus an error.
                    let stem = dir.join(cochlea_render::stem_file_name(&track.name)?);
                    if same_file(&out, &stem) {
                        anyhow::bail!(
                            "the stem for track {:?} would overwrite the mix --out {}",
                            track.name,
                            out.display()
                        );
                    }
                    if let Some(rp) = report.as_ref()
                        && same_file(rp, &stem)
                    {
                        anyhow::bail!(
                            "the stem for track {:?} would overwrite --report {}",
                            track.name,
                            stem.display()
                        );
                    }
                    // ...and not onto the score itself. `load_score` does not
                    // require a `.ron` extension, so a score kept as
                    // `d/lead.wav` plus `--stems d` and a track named `lead`
                    // used to read the score, render, then destroy it at
                    // exit 0.
                    if same_file(&score_path, &stem) {
                        anyhow::bail!(
                            "the stem for track {:?} would overwrite the input score {}",
                            track.name,
                            score_path.display()
                        );
                    }
                }
            }
            let rendered = cochlea_render::render(&score)?;
            rendered
                .write_wav_as(&out, bits)
                .with_context(|| format!("writing {}", out.display()))?;
            eprintln!(
                "rendered {} frames at {} Hz -> {}",
                rendered.frames(),
                rendered.sample_rate().0,
                out.display()
            );
            if let Some(dir) = stems {
                rendered
                    .write_stems_as(&dir, bits)
                    .with_context(|| format!("writing stems to {}", dir.display()))?;
            }
            if verify {
                let result = rendered
                    .verify(&score)
                    .with_specs(score.verify_specs())
                    .run();
                let text = serde_json::to_string_pretty(&result)?;
                match report {
                    Some(path) => std::fs::write(&path, text)
                        .with_context(|| format!("writing {}", path.display()))?,
                    None => println!("{text}"),
                }
                if !result.passed {
                    return Ok(std::process::ExitCode::from(1));
                }
            }
            Ok(std::process::ExitCode::SUCCESS)
        }

        Cmd::Probe {
            input,
            json,
            spectro,
            digest,
            segments,
            loudness,
            beats,
            window_ms,
            from,
            to,
        } => {
            // Distinct output flags writing to one path would silently
            // last-write-win; make the collision a usage error instead.
            let outputs = [
                ("--json", json.as_deref()),
                ("--segments", segments.as_deref()),
                ("--spectro", spectro.as_deref()),
                ("--loudness", loudness.as_deref()),
                ("--beats", beats.as_deref()),
            ];
            for (i, (flag_a, path_a)) in outputs.iter().enumerate() {
                for (flag_b, path_b) in &outputs[i + 1..] {
                    if let (Some(pa), Some(pb)) = (path_a, path_b)
                        && same_file(pa, pb)
                    {
                        anyhow::bail!("{flag_a} and {flag_b} point at the same path {pa:?}");
                    }
                }
            }
            // And none of them may point back at the input — the write
            // would silently destroy the audio being probed (exit 0).
            for (flag, path) in &outputs {
                if let Some(p) = path
                    && same_file(p, &input)
                {
                    anyhow::bail!("{flag} would overwrite the input file {p:?}");
                }
            }

            let audio = cochlea_decode::load(&input)
                .with_context(|| format!("reading {}", input.display()))?;
            // --loudness/--beats describe the whole file, with times measured
            // from the start of the file (like the MCP loudness_timeline /
            // beat_grid tools) — so they read the pre-window audio. Only clone
            // it when a --from/--to window is also active; otherwise the
            // windowed `audio` below is already the whole file.
            let need_full =
                (from.is_some() || to.is_some()) && (loudness.is_some() || beats.is_some());
            let whole_file = need_full.then(|| audio.clone());
            let (audio, start_ms) = apply_window(audio, from, to)?;
            let report = cochlea_features::probe(
                &audio,
                &cochlea_features::ProbeOpts::default().with_start_ms(start_ms),
            );

            // Only pay for the segment timeline when a flag actually needs
            // it (docs/plan.md: probe stays cheap otherwise).
            let timeline = (digest || segments.is_some()).then(|| {
                let opts = cochlea_features::SegmentOpts::default().with_window_ms(window_ms);
                cochlea_features::segment_timeline(&audio, &opts)
            });

            if digest {
                let timeline = timeline
                    .as_ref()
                    .expect("computed above when digest is set");
                println!("{}", cochlea_features::digest_text(&report, timeline));
            }

            let report_text = serde_json::to_string_pretty(&report)?;
            match &json {
                Some(path) => std::fs::write(path, &report_text)
                    .with_context(|| format!("writing {}", path.display()))?,
                None if !digest => println!("{report_text}"),
                None => {}
            }

            if let Some(path) = &segments {
                let timeline = timeline
                    .as_ref()
                    .expect("computed above when segments is set");
                let text = serde_json::to_string_pretty(timeline)?;
                std::fs::write(path, text)
                    .with_context(|| format!("writing {}", path.display()))?;
            }

            if let Some(path) = &loudness {
                let curve = cochlea_features::loudness_timeline(
                    whole_file.as_ref().unwrap_or(&audio),
                    &cochlea_features::LoudnessTimelineOpts::default(),
                );
                std::fs::write(path, serde_json::to_string_pretty(&curve)?)
                    .with_context(|| format!("writing {}", path.display()))?;
            }

            if let Some(path) = &beats {
                let tempo = cochlea_features::estimate_tempo(
                    whole_file.as_ref().unwrap_or(&audio),
                    &cochlea_features::TempoOpts::default(),
                );
                std::fs::write(path, serde_json::to_string_pretty(&tempo)?)
                    .with_context(|| format!("writing {}", path.display()))?;
            }

            if let Some(path) = spectro {
                write_spectro(&audio, &path, false, 0)?;
            }
            Ok(std::process::ExitCode::SUCCESS)
        }

        Cmd::Diff {
            a,
            b,
            json,
            tier2,
            window_ms,
            spectro,
            from,
            to,
        } => {
            // Same input-protection rule as probe: writing the comparison
            // JSON over either input would destroy a file being compared.
            for (flag, path) in [("--json", &json), ("--spectro", &spectro)] {
                if let Some(path) = path
                    && (same_file(path, &a) || same_file(path, &b))
                {
                    anyhow::bail!("{flag} would overwrite the input file {path:?}");
                }
            }
            if let (Some(pa), Some(pb)) = (&json, &spectro)
                && same_file(pa, pb)
            {
                anyhow::bail!("--json and --spectro point at the same path {pa:?}");
            }

            let audio_a =
                cochlea_decode::load(&a).with_context(|| format!("reading {}", a.display()))?;
            let audio_b =
                cochlea_decode::load(&b).with_context(|| format!("reading {}", b.display()))?;
            let (audio_a, start_ms) = apply_window(audio_a, from, to)?;
            let (audio_b, _) = apply_window(audio_b, from, to)?;

            let opts = cochlea_features::SegmentOpts::default().with_window_ms(window_ms);
            let probe_opts = cochlea_features::ProbeOpts::default().with_start_ms(start_ms);
            let report_a = cochlea_features::probe(&audio_a, &probe_opts);
            let report_b = cochlea_features::probe(&audio_b, &probe_opts);
            let timeline_a = cochlea_features::segment_timeline(&audio_a, &opts);
            let timeline_b = cochlea_features::segment_timeline(&audio_b, &opts);

            let identical = cochlea_features::samples_identical(&audio_a, &audio_b);
            let result = cochlea_features::compare_with_identity(
                cochlea_features::Analysis {
                    report: &report_a,
                    timeline: &timeline_a,
                },
                cochlea_features::Analysis {
                    report: &report_b,
                    timeline: &timeline_b,
                },
                identical,
            );

            println!("{}", cochlea_features::compare_text(&result));

            if let Some(path) = &json {
                let text = serde_json::to_string_pretty(&result)?;
                std::fs::write(path, text)
                    .with_context(|| format!("writing {}", path.display()))?;
            }

            if let Some(path) = &spectro {
                let mel = |audio: &cochlea_features::Audio| {
                    cochlea_spectro::mel_spectrogram(
                        &audio.samples,
                        audio.channels,
                        audio.sample_rate,
                        &cochlea_spectro::SpectroOpts::new(),
                    )
                };
                let img = cochlea_spectro::render_diff_png(&mel(&audio_a), &mel(&audio_b))
                    .context("rendering the difference spectrogram")?;
                cochlea_spectro::write_png(&img, path)
                    .with_context(|| format!("writing {}", path.display()))?;
                eprintln!("diff spectrogram -> {}", path.display());
            }

            if tier2 {
                let equivalent = matches!(
                    result.verdict,
                    cochlea_features::Verdict::ByteIdentical
                        | cochlea_features::Verdict::Tier2Equivalent
                );
                if !equivalent {
                    return Ok(std::process::ExitCode::from(1));
                }
            }
            Ok(std::process::ExitCode::SUCCESS)
        }

        Cmd::Eval {
            candidates,
            references,
            exact,
            json,
            window_ms,
        } => {
            // Enumerate candidate audio files, sorted for a deterministic
            // report order regardless of directory-read order.
            let mut cands: Vec<PathBuf> = std::fs::read_dir(&candidates)
                .with_context(|| format!("reading {}", candidates.display()))?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| is_audio_ext(p))
                .collect();
            cands.sort();

            // The report is written *after* every pair is compared, so a
            // `--json` landing on one of the files being compared destroys it
            // while the run still reports the comparison it made a moment
            // earlier — `eval --json cands/a.wav` printed "1/1 passed", exited
            // 0, and left a 208-byte JSON report where a golden WAV had been.
            // Checked here, before the first comparison, so a bad invocation
            // costs nothing and prints no half-run table.
            if let Some(path) = &json {
                for cand in &cands {
                    if same_file(path, cand) {
                        anyhow::bail!("--json would overwrite the candidate {}", cand.display());
                    }
                    let reference = cand
                        .file_name()
                        .map(|name| references.join(name))
                        .unwrap_or_else(|| references.clone());
                    if same_file(path, &reference) {
                        anyhow::bail!(
                            "--json would overwrite the reference {}",
                            reference.display()
                        );
                    }
                }
            }

            let opts = cochlea_features::SegmentOpts::default().with_window_ms(window_ms);
            let mut cases = Vec::new();
            for cand in &cands {
                let name = cand
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let reference = references.join(&name);
                let (verdict, passed) = if !reference.exists() {
                    ("missing_reference".to_string(), false)
                } else {
                    match compare_files(cand, &reference, &opts) {
                        Ok(v) => {
                            let pass = if exact {
                                matches!(v, cochlea_features::Verdict::ByteIdentical)
                            } else {
                                matches!(
                                    v,
                                    cochlea_features::Verdict::ByteIdentical
                                        | cochlea_features::Verdict::Tier2Equivalent
                                )
                            };
                            (verdict_name(v).to_string(), pass)
                        }
                        Err(e) => (format!("error: {e}"), false),
                    }
                };
                println!(
                    "{:>4}  {name:<28}  {verdict}",
                    if passed { "ok" } else { "FAIL" }
                );
                cases.push(serde_json::json!({
                    "name": name,
                    "verdict": verdict,
                    "passed": passed,
                }));
            }

            let passed = cases.iter().filter(|c| c["passed"] == true).count();
            let total = cases.len();
            eprintln!(
                "eval: {passed}/{total} passed ({} tolerance)",
                if exact { "exact" } else { "tier-2" }
            );
            if total == 0 {
                eprintln!(
                    "warning: no candidate audio files in {}",
                    candidates.display()
                );
            }

            if let Some(path) = &json {
                let report = serde_json::json!({
                    "candidates": candidates.display().to_string(),
                    "references": references.display().to_string(),
                    "tier": if exact { "exact" } else { "tier2" },
                    "passed": passed,
                    "total": total,
                    "cases": cases,
                });
                std::fs::write(path, serde_json::to_string_pretty(&report)?)
                    .with_context(|| format!("writing {}", path.display()))?;
            }

            if total > 0 && passed == total {
                Ok(std::process::ExitCode::SUCCESS)
            } else {
                Ok(std::process::ExitCode::from(1))
            }
        }

        Cmd::Lint { score } => {
            let score = load_score(&score)?;
            let findings = score.validate(&PatchBank::presets());
            println!("{}", serde_json::to_string_pretty(&findings)?);
            let errors = findings
                .iter()
                .filter(|f| f.severity == Severity::Error)
                .count();
            if errors > 0 {
                eprintln!("{errors} error(s), {} finding(s) total", findings.len());
                return Ok(std::process::ExitCode::from(1));
            }
            Ok(std::process::ExitCode::SUCCESS)
        }

        Cmd::Spectro {
            input,
            out,
            sheet,
            bars_per_tile,
            annotate,
            from,
            to,
        } => {
            // The audio is fully decoded before the PNG is written, so an
            // aliasing `--out` destroys the input and still exits 0. The
            // `.wav` extension usually saves this by accident (the PNG
            // encoder refuses to write one), but `cochlea_decode::load`
            // recognizes a file by its magic bytes when the extension is not
            // an audio one — so `spectro audio.png --out audio.png` decoded
            // 2.7 MB of WAV and wrote a spectrogram over it. The MCP
            // `spectrogram` tool has always had this guard; the subcommand it
            // mirrors did not.
            if same_file(&out, &input) {
                anyhow::bail!("--out would overwrite the input file {out:?}");
            }
            let audio = cochlea_decode::load(&input)
                .with_context(|| format!("reading {}", input.display()))?;
            let (audio, _) = apply_window(audio, from, to)?;
            if annotate {
                write_annotated_spectro(&audio, &out)?;
            } else {
                write_spectro(&audio, &out, sheet, bars_per_tile)?;
            }
            Ok(std::process::ExitCode::SUCCESS)
        }

        Cmd::Import {
            input,
            out,
            sample_rate,
        } => {
            if same_file(&out, &input) {
                anyhow::bail!("--out would overwrite the input file {out:?}");
            }
            let bytes =
                std::fs::read(&input).with_context(|| format!("reading {}", input.display()))?;
            let cochlea_score::MidiImport { score, warnings } =
                cochlea_score::import_midi(&bytes, cochlea_score::SampleRate(sample_rate))?;
            for w in &warnings {
                eprintln!("note: {w}");
            }
            std::fs::write(&out, score.to_ron()?)
                .with_context(|| format!("writing {}", out.display()))?;
            eprintln!(
                "imported {} tracks, {} tempo changes -> {}",
                score.tracks().len(),
                score.tempo_changes().count(),
                out.display()
            );
            // The import maps instruments by rough family — lint the result
            // so a preset guess that doesn't exist in the catalog (it
            // shouldn't happen, but the lint is cheap) surfaces here, not
            // at render time.
            let errors = score
                .validate(&PatchBank::presets())
                .into_iter()
                .filter(|f| f.severity == Severity::Error)
                .count();
            if errors > 0 {
                anyhow::bail!("imported score fails lint with {errors} error(s)");
            }
            Ok(std::process::ExitCode::SUCCESS)
        }

        Cmd::Export { score, out } => {
            // Refuse to clobber the source score, including a different
            // spelling of the same file (`--out ./score.ron` vs `score.ron`).
            if same_file(&out, &score) {
                anyhow::bail!("--out would overwrite the input score {out:?}");
            }
            let loaded = load_score(&score)?;
            let bytes = cochlea_score::export_midi(&loaded)?;
            std::fs::write(&out, bytes).with_context(|| format!("writing {}", out.display()))?;
            eprintln!(
                "exported {} tracks, {} tempo changes -> {}",
                loaded.tracks().len(),
                loaded.tempo_changes().count(),
                out.display()
            );
            Ok(std::process::ExitCode::SUCCESS)
        }

        Cmd::Transcribe {
            input,
            out,
            bpm,
            grid,
            preset,
            track,
            ppq,
        } => {
            // The read subcommands never write over their own input, in any
            // spelling of the path (see `same_file`).
            if same_file(&out, &input) {
                anyhow::bail!("--out would overwrite the input audio {out:?}");
            }

            // Everything knowable without the audio is checked *first*:
            // decoding and the YIN pass cost seconds to minutes on a long
            // file, and `--out` is written near the end, so a bad flag that
            // surfaced late would burn that work and could clobber an
            // existing file with a score we already knew was invalid.
            let grid = parse_grid(&grid)?;
            let ppq = cochlea_score::Ppq(ppq);
            // Probe the whole (rate, ppq, grid) combination by building the
            // same empty score `transcribe` will build.
            let probe = cochlea_score::Score::try_new(cochlea_score::SampleRate(48_000), ppq)
                .context("--ppq is out of range")?;
            if let Some(g) = grid {
                g.resolve(ppq).with_context(|| {
                    format!("--grid does not land on a whole tick at --ppq {}", ppq.0)
                })?;
            }
            let bank = PatchBank::presets();
            if !bank.patch_names().contains(&preset.as_str()) {
                anyhow::bail!(
                    "unknown preset {preset:?} — known presets: {}",
                    bank.patch_names().join(", ")
                );
            }
            drop(probe);

            let audio = cochlea_decode::load(&input)
                .with_context(|| format!("reading {}", input.display()))?;
            // The score IR's sample-rate bounds are narrower than what the
            // decoders accept, so check before spending the analysis passes.
            cochlea_score::Score::try_new(cochlea_score::SampleRate(audio.sample_rate), ppq)
                .with_context(|| {
                    format!(
                        "the input's {} Hz sample rate is outside what a score can carry; \
                     resample it first",
                        audio.sample_rate
                    )
                })?;

            // Tempo, and the grid's phase. The beat grid is estimated even
            // when `--bpm` pins the tempo, because the first beat is what
            // tells us *where* the grid lines fall — without it a recording
            // with a count-in quantizes against an offset grid.
            let tempo =
                cochlea_features::estimate_tempo(&audio, &cochlea_features::TempoOpts::default());
            let (bpm, tempo_source) = match bpm {
                Some(b) => (b, "given"),
                None => match tempo.bpm {
                    Some(b) => (b, "detected"),
                    None => (120.0, "undetected, defaulted"),
                },
            };
            // The first detected beat carries the grid's phase; with no
            // pulse at all the file's start is the only reference left.
            let grid_anchor_ms = tempo.beats_ms.first().copied().unwrap_or(0.0);

            let melody = cochlea_features::extract_melody(&audio);
            // One downmix for every note, not one per note.
            let windows: Vec<(f64, f64)> = melody.iter().map(|n| (n.start_ms, n.end_ms)).collect();
            let peaks = cochlea_features::peak_dbfs_for_windows(&audio, &windows);
            let observations: Vec<cochlea_score::NoteObservation> = melody
                .iter()
                .zip(peaks)
                .map(|(n, peak)| {
                    cochlea_score::NoteObservation::from_peak_dbfs(
                        n.start_ms, n.end_ms, n.midi, peak,
                    )
                })
                .collect();

            let opts = cochlea_score::TranscribeOpts::new()
                .with_sample_rate(cochlea_score::SampleRate(audio.sample_rate))
                .with_ppq(ppq)
                .with_bpm(cochlea_score::Bpm(bpm))
                .with_grid(grid)
                .with_grid_anchor_ms(grid_anchor_ms)
                .with_preset(&preset)
                .with_track_name(&track);
            let cochlea_score::Transcription { score, warnings } =
                cochlea_score::transcribe(&observations, &opts)?;

            eprintln!("note: tempo {bpm:.2} BPM ({tempo_source})");
            eprintln!("note: pitch tracking is monophonic — chords and drums read as one line");
            for w in &warnings {
                eprintln!("note: {w}");
            }

            // Lint *before* writing: an invalid score must never land on
            // top of whatever `--out` already pointed at.
            let errors = score
                .validate(&PatchBank::presets())
                .into_iter()
                .filter(|f| f.severity == Severity::Error)
                .count();
            if errors > 0 {
                anyhow::bail!(
                    "transcribed score fails lint with {errors} error(s); {} not written",
                    out.display()
                );
            }

            std::fs::write(&out, score.to_ron()?)
                .with_context(|| format!("writing {}", out.display()))?;
            eprintln!(
                "transcribed {} notes -> {}",
                score.tracks().first().map_or(0, |t| t.notes.len()),
                out.display()
            );
            Ok(std::process::ExitCode::SUCCESS)
        }

        Cmd::Reference => {
            print!(
                "{}",
                cochlea_score::authoring_reference(&cochlea_synth::PatchBank::presets())
            );
            Ok(std::process::ExitCode::SUCCESS)
        }
    }
}

/// `--grid`: a note duration (`1/16`, `1/8t`, `1/4.`) or `none`/`raw` to
/// keep the analyzer's own timing.
fn parse_grid(s: &str) -> anyhow::Result<Option<cochlea_score::Dur>> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") || t.eq_ignore_ascii_case("raw") {
        return Ok(None);
    }
    Ok(Some(cochlea_score::Dur::parse(t)?))
}

fn write_spectro(
    audio: &cochlea_features::Audio,
    path: &Path,
    sheet: bool,
    per_tile: usize,
) -> anyhow::Result<()> {
    let spec = cochlea_spectro::mel_spectrogram(
        &audio.samples,
        audio.channels,
        audio.sample_rate,
        &cochlea_spectro::SpectroOpts::new(),
    );
    let img = if sheet {
        cochlea_spectro::contact_sheet(&spec, &[], per_tile)
    } else {
        cochlea_spectro::render_png(&spec, &[])
    };
    cochlea_spectro::write_png(&img, path)
        .with_context(|| format!("writing {}", path.display()))?;
    eprintln!("spectrogram -> {}", path.display());
    Ok(())
}

/// `spectro --annotate`: run the analyzers over the (possibly windowed)
/// audio and draw what they heard — beat grid, onsets, pitch segments —
/// onto the spectrogram. The overlay is built here, as plain sample/Hz
/// data, because `cochlea-spectro` never sees feature-report types
/// (dependency-direction law).
fn write_annotated_spectro(audio: &cochlea_features::Audio, path: &Path) -> anyhow::Result<()> {
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
    let overlay = cochlea_spectro::Overlay {
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
    };

    let spec = cochlea_spectro::mel_spectrogram(
        &audio.samples,
        audio.channels,
        audio.sample_rate,
        &cochlea_spectro::SpectroOpts::new(),
    );
    let img = cochlea_spectro::render_annotated(&spec, &[], &overlay);
    cochlea_spectro::write_png(&img, path)
        .with_context(|| format!("writing {}", path.display()))?;
    eprintln!(
        "annotated spectrogram ({} beats, {} onsets, {} pitch segments) -> {}",
        overlay.beats.len(),
        overlay.onsets.len(),
        overlay.pitch.len(),
        path.display()
    );
    Ok(())
}
