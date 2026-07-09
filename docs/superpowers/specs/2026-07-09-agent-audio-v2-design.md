# cochlea v2 design: agents read real-world audio, plus launch plan

Date: 2026-07-09. Status: **implemented** same day (both waves built,
reviewed, gates-green; §5 promotion *execution* remains gated on Richard's
explicit go). Deviations from the letter of this spec, all deliberate:
Report v2 embeds a compact `TempoSummary` (full beat grid via
`estimate_tempo`); the short-term loudness curve was dropped as redundant
with the segment timeline (LRA shipped); `SectionCount` took a min/max
range; the drum demo asserts `HasClearRhythm(false)` — a layered groove
dilutes autocorrelation salience below the calibrated threshold, and the
honest assertion beat retuning the detector for one fixture.

## 0. Goal and locked decisions

cochlea becomes **the way AI agents read lossless audio without draining
context windows**: numbers and small images instead of megabytes of PCM.

Decisions locked with Richard:

- **Read scope**: any real-world file, lossless-first (WAV + FLAC), with
  music/production strength — including an explicit clear-rhythm detector —
  while also hardening our own renders. Still ffmpeg-free. Decode via
  `symphonia` (pure Rust, MPL-2.0, dependency-compatible with MIT/Apache);
  algorithms implemented from the literature and format specs, never
  translated from GPL sources.
- **Agent surface**: all three — CLI, MCP server, and the full library
  (published to crates.io).
- **Analysis scope**: segment timeline + digest, audio diff, stereo image +
  loudness range, structure detection, tempo/beat — plus small useful
  extras (per-segment spectral descriptors, DC/format reporting).
- **Sequencing**: two build waves, **one combined release** after both.

## 1. Wave 1 — the context-window play (WAV inputs)

### 1.1 `cochlea-features` additions (no change to the existing `Report`)

- `segments.rs` — windowed timeline (`SegmentTimeline`, own
  `schema_version: 1`; default 1000 ms windows): per segment RMS/peak dBFS
  (`Option<f64>`, null for digital silence), onset count (bucketed from the
  whole-file detector, frame-center times), silent flag
  (`silence_floor_dbfs`, default −60), YIN f0 + nearest MIDI note + cents,
  low/mid/high band-energy fractions (<250 Hz / 250–4000 Hz / >4 kHz).
- `digest.rs` — `digest_text(&Report, &SegmentTimeline) -> String`:
  deterministic plain-text digest. Header (duration/channels/rate), one
  line each for loudness / key+pitch / onsets / silence / clipping, then a
  timeline table **capped at ~40 rows** by deterministic bucket-merging
  (rms=max, peak=max, onsets=sum, silent=all, f0=median-of-voiced) with
  silent-run collapsing. A 3-minute WAV (~30 MB) reads as ~1 KB of text.
- `compare.rs` — feature-space diff: loudness deltas, greedy onset matching
  (50 ms gate; matched/mean/max offset/unmatched), pitch delta in cents,
  key change, per-segment RMS delta; verdicts `ByteIdentical` (caller
  supplies sample identity via `samples_identical(&Audio, &Audio)`),
  `Tier2Equivalent` (reuses the workspace Tier-2 tolerances: LUFS ≤ 0.1 LU,
  onsets ≤ 2 ms, pitch ≤ 5 cents), `Different { dimensions }`. Plus
  `compare_text` compact rendering. `CompareReport` has its own
  `schema_version: 1`.

### 1.2 CLI

`probe --digest`, `probe --segments <path|->` (JSON), and
`cochlea diff a.wav b.wav [--json path] [--tier2]` (`--tier2` exits 1 when
outside tolerance). Exit-code convention unchanged (0/1/2).

### 1.3 `cochlea-mcp`

Stdio MCP server, hand-rolled JSON-RPC 2.0 over newline-delimited stdio —
no async runtime, no new external deps. Tools: `render_score`, `probe_wav`,
`spectrogram`, `lint_score`, then `probe_digest` + `audio_diff` once 1.1
lands. Dispatch is a pure `handle_line(&str) -> Option<String>` for
unit-testing. Tool failures are `isError: true` text results; JSON-RPC
errors only for protocol faults (−32700/−32601/−32602). `docs/mcp.md`
documents Claude Code setup.

### 1.4 README

Hero + demo spectrograms rendered headlessly into `docs/assets/`, CI badge,
"How an agent listens" section (compose → render → probe → spectrogram →
verify) with a trimmed probe-JSON excerpt and the context-window economics
stated plainly.

## 2. Wave 2 — real-world files + musical hearing + publish-prep

- **`cochlea-decode` crate**: `load(path) -> Audio`; WAV via hound, FLAC
  via symphonia. Lift the `symphonia` entry from `deny.toml` `[bans]` (it
  was a phase-2 deferral, not a law) with an updated reason string. Tiny
  committed FLAC fixtures with WAV twins; decode asserted sample-exact.
  Lossless only this round (mp3/ogg later). Depends on `cochlea-features`
  for the `Audio` type; `features`/`spectro` stay score/synth-free.
- **`tempo.rs`**: onset-envelope autocorrelation + dynamic-programming beat
  tracking (Ellis 2007): BPM, beat grid, confidence, and
  `clear_rhythm: bool` from autocorrelation peak salience.
- **`stereo.rs`**: width (mid/side energy ratio), L/R correlation, balance.
- **`loudness.rs`**: adds LRA and the short-term loudness curve (ebur128).
- **`structure.rs`**: Foote self-similarity novelty over per-second
  chroma+energy → unlabeled section boundaries S1..Sn with confidence.
- **`Report` bumps to `schema_version: 2`** once, adding
  tempo/stereo/lra/structure; demos/tests updated in the same change.
- **Harden our renders**: new `VerifySpec` variants (in `cochlea-score`) +
  `Verifier` methods: `TempoIs`, `HasClearRhythm`, `StereoWidthWithin`,
  `LraBelow`, `SectionCount`; a new drum-groove demo with its own golden
  PCM hash + spectrogram sentinel extends Tier-1 coverage over the new DSP.
- **Publish-prep**: per-crate metadata/READMEs, docs.rs config,
  `cargo publish --dry-run` across the graph in dependency order
  (fenestra-anim is already on crates.io).

## 3. Determinism posture for the new code

All new DSP obeys the existing bans (libm transcendentals,
`FftPlannerScalar`, no `mul_add`, denormals honored). Digest/compare text
is byte-deterministic per platform; analysis floats remain Tier-2 across
platforms — same honesty as today. No wall clock anywhere, including MCP
responses.

## 4. Testing

Synthesized fixtures in-test (house style): click tracks at known BPM for
tempo (±1 BPM), AB-pattern fixtures for structure boundaries, known-phase
stereo fixtures for correlation/width, exact-string digest snapshot, diff
verdict matrix (identical / level-shift / detune / added onset), MCP
protocol conformance + one real render end-to-end, FLAC-vs-WAV twin
equality. Demo suite and golden hash stay green throughout; the rhythm
demo adds a new golden.

## 5. Launch and promotion (drafts now; EXECUTION gated on Richard's go)

1. **Pre-flight polish**: README with screenshots, docs/mcp.md, CHANGELOG,
   license files verified, issue labels + a few good-first-issues, social
   preview image (a spectrogram).
2. **Flip public** after both waves are gates-green.
3. **crates.io + docs.rs**: publish in dependency order; Richard runs or
   explicitly authorizes the publishes.
4. **Ecosystem listings**: awesome-rust (Audio), awesome-mcp-servers, the
   MCP community registry, This Week in Rust submission.
5. **Launch posts, drafted for Richard to post**: Show HN, r/rust, X
   thread. Authentic voice; no astroturfing, no fake engagement — organic
   only.
6. **Later, with explicit go**: Claude-in-Chrome screenshots of the MCP
   tools running inside Claude Code, for docs and posts.

## 6. Out of scope (unchanged from v1 list unless stated)

Realtime/cpal, MIDI, sampled instruments, lossy-format decode (mp3/ogg —
next), tempo ramps, GUI/GPU. FLAC *encoding* is out; WAV remains the write
format.
