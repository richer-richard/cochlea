# Changelog

All notable changes to the cochlea workspace. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the workspace
versions all crates together.

## [0.4.0] — 2026-07-24

The voice-and-ears upgrade, and a hardening pass from an adversarial review:
close the two reproduced crashes and the read-path DoS, then answer harmony,
loudness-over-time, integer-PCM and MIDI export, a golden-audio eval harness,
and Python bindings.

### Hardening (adversarial review)

- **No tool can take the MCP server down.** `cochlea-mcp` now wraps every
  tool dispatch in `catch_unwind`, so a panic anywhere in the pipeline becomes
  a contained `isError` result instead of unwinding the single-threaded serve
  loop and killing the long-lived process (and every in-flight and future
  request in the session). Regression-tested with a poison call followed by a
  harmless one.
- **Far-future positions can't panic the renderer.** Authored ticks (tempo
  changes, note ends, automation keys) are bounded at load by `Ticks::MAX`
  (2³²) — chosen to keep the exact tick→sample/ns rational arithmetic from
  overflowing `u64` while still exceeding any one-hour render — so a crafted
  tempo tick that used to reach unchecked `mul_div` and panic now fails cleanly
  with `PositionTooFar`. Covers the programmatic builder, the RON loader, and
  MIDI import.
- **The read path is bounded.** `cochlea_decode::load` caps total decoded
  samples (`DEFAULT_MAX_SAMPLES`, overridable via `load_with_limit`),
  refusing a decompression bomb *as it accumulates* and an oversized WAV from
  its header — closing the unbounded-allocation / O(window²)-compute DoS that
  render's one-hour cap didn't cover.
- **Degenerate audio is refused at the door.** Zero channels, zero sample
  rate, or a ragged interleave are rejected by both `Audio::from_wav` and the
  decode boundary, so the mel spectrogram and per-frame analyzers can never be
  reached with a shape that would divide by zero or trip an assertion.

### Added — hearing (Report schema v5)

- **`harmony`** (`HarmonyReport`): a chord timeline plus per-section key — the
  two questions ("what's the progression", "what key is the bridge in") the
  single global `key` couldn't answer. Chords are template-matched from the
  same chroma the key estimate uses (major/minor/7ths/dim/aug/sus4 in all
  twelve roots), with a presence gate and a simplicity bias so a bare triad
  isn't over-read as a seventh or a lone note as a chord. Per-section key
  reuses the Krumhansl-Schmuckler correlation over structure-segmented chroma.
  Standalone `analyze_harmony`; a `harmony:` line in the digest.
- **`loudness.short_term_max_lufs`** and a standalone `loudness_timeline`
  (momentary + short-term LUFS sampled over time) — the dynamics view the
  single integrated/LRA summary can't give.
- **Downbeat / bar-relative reporting**: `TempoReport` gains `beats_per_bar`
  and `downbeats_ms` (the bar-opening beats, by onset-energy phase under an
  assumed meter), plus `TempoReport::bar_beat_at(ms)` — "beat 3 of bar 2"
  instead of a raw millisecond offset.

### Added — generation

- **`fm_bell` preset**: a single-operator FM voice (harmonic ratio so the
  pitch reads back on the played note, a decaying modulation index for the
  bright metallic strike) with an automatable `brightness` param — the
  palette's first non-subtractive voice, widening the timbral range and
  showing the IR carry a timbre knob an agent can sweep, not just a cutoff.

### Added — I/O

- **16- and 24-bit integer WAV output** alongside the lossless 32-bit float
  (`Rendered::write_wav_as`/`WavBitDepth`; CLI `render --bits 16|24|float`;
  MCP `render_score` `bits`). Deterministic round-to-nearest quantization,
  clamped, no dither.
- **MIDI export** (`export_midi`): a Score → Standard MIDI File (format 1)
  writer, the inverse of the importer — timing (ticks, tempo map, time
  signature) exports exactly, instruments become rough GM labels. CLI
  `export`, MCP `export_midi`; round-trip tested against the importer.
- **Python bindings** (`bindings/python`, pyo3 + maturin): `probe`, `diff`,
  `render`, `spectrogram`, `probe_digest`, `samples_identical`, plus an
  `assert_audio(...)` fluent API and a pytest `assert_audio` fixture. The
  deterministic core stays pure Rust; this is a thin reach layer (a detached
  crate, kept out of the determinism-critical build).

### Added — golden-audio harness

- **`cochlea eval`**: score a directory of candidate audio files against a
  directory of references (matched by filename), deterministically — a
  per-file verdict table, an aggregate pass rate, exit 1 on any regression or
  missing reference, optional JSON. The generative-model / reference-render
  regression oracle.
- A **GitHub composite action** (`.github/actions/golden-audio`) wrapping it,
  and a [golden-audio testing guide](docs/golden-audio.md).

## [0.3.0] — 2026-07-22

The hearing upgrade: melody as notes, timbre identity, triplet grids, a
zoom lens over every read tool, annotated and diff spectrograms, a master
bus with a real limiter, lossy-format probe input, and MIDI import.

### Added — reading audio (Report schema v4, CompareReport v3)

- **`pitch.melody`** (`MelodyNote`): the YIN track quantized to
  equal-tempered note events — start/end, note name, median f0, cents
  deviation. The compose loop's read-back half: an agent that wrote a
  melody can diff what the render *sounds like* as notes against what it
  wrote. Monophonic by construction (documented); standalone entry point
  `extract_melody`. The digest gains a `melody:` line.
- **`timbre`** (`TimbreReport`): a compact MFCC digest (26 mel filters,
  13 coefficients, per-coefficient mean and spread over the buffer) —
  the "did the re-render keep the instrument's character" axis. The
  compare report gains `timbre.mfcc_distance` (spectral shape only, `c0`
  excluded) and the diff text a `timbre` row.
- **`rhythm.grid`**: grid alignment now tests two subdivision hypotheses
  — straight sixteenths and eighth-note triplets — and reports whichever
  more onsets land on. A shuffle reads as an aligned *triplet* rhythm
  (alignment 1.0 on the calibration fixture) instead of being force-fit
  to sixteenths and scored sloppy; straight eighths stay decisively
  `straight` (their off-beats sit at 1/2 beat, not a triplet point).
  `CompareReport` gains `rhythm.grid_changed` — a feel change, distinct
  from a tightness change.
- **The zoom lens**: `Audio::window` (frame-exact `[from, to)` cuts) +
  `ProbeOpts::start_ms` + `source.start_ms` in the report; CLI
  `probe`/`spectro`/`diff` take `--from/--to`, and the MCP
  `probe_audio`/`spectrogram` tools take `from_s`/`to_s`. Report times
  are relative to the cut; `start_ms` anchors them in the file.
- **mp3 and ogg/vorbis probe input** (`cochlea-decode`): a lossy
  symphonia path with its contract stated where it lives — analysis
  input, never render ground truth (reproducible per build; no exactness
  claim once a codec has discarded the original samples). Extension and
  magic-byte dispatch (ID3/MPEG sync, `OggS`); the same tone probed from
  WAV, mp3, and ogg reads the same pitch within 5 cents (tested). The
  FLAC bit-exact module is untouched and separate.

### Added — looking at audio

- **Annotated spectrograms**: `render_annotated` draws analysis overlays
  on the image — detected beats (orange ticks, top edge), onsets (cyan
  ticks, bottom), pitch segments (magenta lines at their mel band) — via
  a plain-data `Overlay` (the spectro crate still never sees score or
  feature types). CLI `spectro --annotate`; MCP `spectrogram` with
  `annotate: true` (still inline image content).
- **Signed diff spectrograms**: `render_diff_png` — per-band `B − A` in
  dB, red = louder, blue = quieter, black = unchanged, saturating at
  ±24 dB. A moved onset is a blue/red vertical pair; a brightened sweep
  is a red wedge. Refuses mismatched analysis axes (`AxisMismatch`). CLI
  `diff --spectro delta.png`; MCP `audio_diff` with `spectrogram: true`
  (inline). `MelSpec` now carries its frequency axis (`fmin`/`fmax`) and
  maps Hz→band via `hz_band`.

### Added — making audio

- **Master bus** (`Master`/`Limiter` in the score IR, RON `master:`
  section): an output gain (−40..+24 dB) and a brick-wall lookahead
  limiter (ceiling −40..0 dBFS, lookahead 0..50 ms, release 1..1000 ms),
  applied to the f64 stem sum before the single f32 rounding. Offline
  lookahead is a forward sliding-window maximum — no delay line, no
  latency — and the *sample*-peak ceiling holds exactly by construction
  (leave ~1 dB under a `TruePeakBelow` target; true peaks are
  inter-sample). The do-nothing default master skips the stage entirely:
  master-less scores render byte-identically to 0.2.0 (both golden
  hashes unchanged), and `mix == Σ stems` still holds for them. Stems
  stay pre-master. This is the tool the loop kept asking for: push with
  `gain_db`, hold the ceiling, assert `IntegratedLufs` + `TruePeakBelow`.
- **MIDI import** (`cochlea_score::import_midi`, `cochlea import`, MCP
  `import_midi`): hand-rolled SMF format 0/1 parser (chunks, VLQs,
  running status). Timing maps exactly — the file's division becomes the
  score's PPQ verbatim, tempo metas become tempo-map steps, notes land
  unquantized on the integer grid. GM programs map to rough preset
  families and channel-10 percussion splits into kick/snare/hat tracks;
  every guess is returned as a warning for re-voicing. SMPTE division
  and format 2 are refused with reasons; CCs/bends/aftertouch are
  skipped loudly, never silently.

### Changed

- `probe()` runs one shared YIN pass for the pitch summary and melody.
- MCP tool descriptions updated to schema v4 and the wider format
  support; the tool list is eight (`import_midi` added).

## [0.2.0] — 2026-07-21

> **Versioning note.** The crates published to crates.io
> as `0.1.0` on 2026-07-10 were built from a tree that already contained
> the entire "agent read stack" below — the workspace was published
> mid-cycle without a version bump, so the published `0.1.0` does not
> match this changelog's `0.1.0` entry. `0.2.0` is the first release
> where the version number, the tag, and this changelog agree. Sorry;
> won't happen again (releases now cut from a tagged, changelog-matched
> commit only).

### Changed — the tempo/rhythm split (Report schema v3)

Tempo (speed) and rhythm (pattern) are separate axes that change
independently — a drum solo holds a steady pulse while its pattern turns
confusing — and are now separate detectors and report sections:

- **`tempo.confidence` is now pulse clarity** — mean-removed,
  length-unbiased normalized autocorrelation — replacing the v2
  mass-fraction score, which structurally punished music for carrying
  several metrical levels at once (the `drum_groove` demo read 0.01
  despite a spot-on BPM; it now reads 0.79). Not comparable to v2 values.
- **`tempo.candidates`**: the winner plus the strongest distinct octave
  alternatives with saliences — metrical ambiguity surfaced as data for
  the caller to weigh, instead of a coin flip hidden behind the prior.
- **`tempo.stability`**: windowed tempo agreement (mod octave) — the
  axis that separates "the rhythm got confusing" from "the speed moved".
- **New top-level `rhythm` section** (`cochlea_features::RhythmReport`):
  `grid_alignment` (fraction of onsets on the beat-subdivision grid),
  `offbeat_ratio` (syncopation as a number), `onset_rate_per_s`, and the
  grid-based **`clear_rhythm`** that replaces the old confidence rule
  (moved here from `tempo`).
- Tempo with fewer than two detected onsets is now `null` — a
  sustained tone's window-sliding flux ripple is genuinely periodic and
  used to read confidence 0.99.
- The Ellis beat-DP envelope is normalized to unit std with the penalty
  rescaled accordingly; the unnormalized envelope made the period
  penalty negligible, so the "beat grid" snapped to every onset.
- Robustness, measured (`crates/features/tests/rhythm.rs`): ±10 ms
  humanized timing jitter keeps the exact BPM and `clear_rhythm`;
  ±20–30 ms octave-folds the BPM but keeps alignment 1.0 and
  `clear_rhythm`; a dropped plus an extra hit in 22 changes nothing;
  uniformly random onsets are rejected on two independent gates.
- `CompareReport` schema v2: `rhythm` delta section; `tempo` delta
  carries `stability_delta`.

### Added

- **Verify**: `GridAlignmentAtLeast(min)`, and the render-side sweep
  checks `BrightnessRises`/`BrightnessFalls(track, from, to, min_ratio)`
  — `Monotone` deliberately validates only the *authored* automation
  curve, so nothing previously verified a sweep audibly happened in the
  output; these listen to the stem's spectral centroid (new public
  `cochlea_features::spectral_centroid_curve`). Builder forms:
  `grid_alignment_at_least`, `brightness_rises`, `brightness_falls`.
- **Synth**: `kick` (pitch-drop sine) and `snare` (tonal body +
  counter-RNG noise burst) presets — eight total; `chord_pad` is now
  genuinely stereo (detuned saws pan ±0.35, constant-power).
  `drum_groove` uses the real kit, pans hats/snare via the engine's
  (previously never-exercised) `pan` automation, and asserts
  `HasClearRhythm(true)` with grid alignment ≥ 0.9. Golden re-blessed.
- **Self-describing authoring reference**
  (`cochlea_score::authoring_reference`): the RON grammar, the live
  preset catalog (generated from the registry, cannot go stale), every
  verify assertion, and a worked example that the test suite parses and
  renders. Served by the new `cochlea reference` subcommand, the new
  `score_reference` MCP tool, and the book's Score Format page (all
  three pinned to one generator by tests).
- **MCP**: `spectrogram` returns the image inline as MCP image content
  (base64 PNG, size-capped; `out_path` now optional) — clients without
  filesystem access get the one-vision-call review. `--root DIR`
  confinement: every path must canonically resolve inside the root or
  the call is refused before any filesystem work; the input-clobber
  guards now compare canonical paths, not strings.
- `cochlea_features::estimate_tempo_and_rhythm` (one shared analysis
  pass) and `cochlea_spectro::encode_png` (PNG to memory).

### Fixed

- Structure detection computes a *banded* self-similarity matrix — the
  novelty kernel never reads beyond ±16 frames, so the full O(n²) matrix
  (~415 MB for two hours of audio) was almost entirely waste — plus a
  deterministic frame-count cap; the old 1 ms `frame_ms` floor never
  actually bounded long files, despite its comment claiming so.
- `UnknownInstrument` errors list the bank's actual patch names instead
  of a hard-coded six-name string.

## [0.1.0-unversioned] — published to crates.io 2026-07-10 (see the 0.2.0 versioning note)

### Added — the agent read stack (v2, wave 1 + wave 2)

- **Segment timeline** (`cochlea-features`): windowed per-second feature
  timeline (RMS/peak dBFS, onset counts, silence flags, YIN f0 + nearest
  note, low/mid/high band energy) so an agent seeks inside audio by index,
  never by PCM.
- **Text digest** (`cochlea-features`, `cochlea probe --digest`): a
  deterministic plain-text digest of any audio file — a 3-minute WAV
  (~66 MB of PCM) reads as ~2.3 KB of text (measured, not estimated:
  36 bucketed timeline rows, well under the 40-row cap); the timeline
  table is row-capped, not duration-scaled, so this stays roughly flat
  for longer files too.
- **Audio diff** (`cochlea-features::compare`, `cochlea diff`):
  feature-space comparison with `byte-identical` / `tier2-equivalent` /
  `different (dimensions…)` verdicts, reusing the workspace's Tier-2
  tolerances (LUFS 0.1 LU, onsets 2 ms, pitch 5 cents); `--tier2` exits
  nonzero when not equivalent.
- **MCP server** (`cochlea-mcp`): stdio JSON-RPC 2.0 server (hand-rolled,
  no async runtime) exposing `render_score`, `probe_audio`, `spectrogram`,
  `lint_score`, `probe_digest`, and `audio_diff` as agent tools; see
  `docs/mcp.md`.
- **FLAC input** (`cochlea-decode`): lossless decode via pure-Rust
  symphonia, verified bit-exact against WAV twins; `probe`, `diff`,
  `spectro`, and every MCP tool accept `.flac` alongside `.wav`. Still
  ffmpeg-free.
- **Tempo/beat tracking** (`cochlea-features::tempo`): onset-envelope
  autocorrelation with a log-Gaussian tempo prior and an Ellis-2007-style
  DP beat grid; reports BPM, a beat grid, a 0–1 confidence, and a
  calibrated `clear_rhythm` boolean an agent can trust.
- **Stereo image + loudness range** (`cochlea-features`): width /
  correlation / balance, and EBU R128 LRA.
- **Structure detection** (`cochlea-features`): self-similarity novelty
  section boundaries (Foote checkerboard-kernel novelty over per-second
  chroma+band+RMS vectors).
- **Report schema v2**: probe JSON now carries tempo (bpm / confidence /
  `clear_rhythm` / beat count), stereo image, structure, and LRA; the
  digest surfaces the same. Five new RON-embeddable verify assertions
  (`TempoIs`, `HasClearRhythm`, `StereoWidthWithin`, `LraBelow`,
  `SectionCount`) and a fourth demo (`drum_groove`, 110 BPM) with its own
  golden PCM hash.
- Per-crate READMEs and crates.io metadata for the whole workspace.

## [0.1.0] — 2026-07-03

Initial complete v1: an audio engine with no human ear in the loop.

- `cochlea-score`: declarative score IR — integer ticks at 960 PPQ, tempo
  map, bar/beat builder, typed params, RON data form (`version: 1`) with
  embeddable `verify:` assertions.
- `cochlea-synth`: six presets over fundsp (`sine`, `saw_lead`,
  `square_bass`, `chord_pad`, `noise_hat`, `pluck`) plus an in-repo
  Schroeder reverb insert; counter-based RNG keyed `(seed, sample_index)`.
- `cochlea-render`: 64-sample block engine split at event boundaries,
  sample-accurate note scheduling, pure voice allocation/stealing,
  per-track stems, f64 master sum, WAV out.
- `cochlea-features`: schema-versioned JSON probe — integrated LUFS /
  momentary max / true peak (ebur128), spectral-flux onsets, YIN pitch,
  chroma + Krumhansl-Schmuckler key, silence/tail, clipping.
- `cochlea-spectro`: mel spectrogram PNGs (hand-rolled HTK-style
  filterbank — a documented choice over Slaney's, see the crate's
  `mel` module — viridis, time ruler, caller-supplied markers), tiled
  contact sheets, image diff for Tier-3 sentinels.
- `cochlea-verify`: assertion DSL over renders (`integrated_lufs`,
  `true_peak_below`, `onset_at`, `pitch_matches_score`, `monotone`,
  `no_discontinuity`, `silent_after`) with JSON reports.
- `cochlea` CLI: `render` / `probe` / `lint` / `spectro`; exit codes
  0 ok, 1 assertion failures, 2 usage/IO.
- The three-tier determinism contract, CI-enforced: byte-identical PCM on
  the pinned target (libm-only transcendentals via clippy bans, no
  fast-math, no implicit FMA, denormals honored, fixed summation order),
  cross-platform feature tolerances, spectrogram sentinels. Confirmed
  byte-identical across aarch64-macos and x86_64-linux by CI.
