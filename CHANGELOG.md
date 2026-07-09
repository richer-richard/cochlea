# Changelog

All notable changes to the cochlea workspace. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the workspace
versions all crates together.

## [Unreleased]

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
