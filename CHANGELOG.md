# Changelog

All notable changes to the cochlea workspace. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the workspace
versions all crates together.

## [0.2.0] — 2026-07-21

> **Versioning note, honestly stated.** The crates published to crates.io
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
- Tempo with fewer than two detected onsets is now honestly `null` — a
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
