# Determinism: decisions, audits, and rationale

This file records the Phase 0 determinism decisions and the dependency
audits behind them. The contract itself (three tiers) is summarized in
the README; this is the evidence and the fine print.

## The three tiers, restated precisely

- **Tier 1 — byte-identical PCM** for identical inputs on the pinned target:
  `x86_64-unknown-linux-gnu`, toolchain `1.95.0` (`rust-toolchain.toml`).
  Two renders of the same score are byte-equal on *any* machine (tested on
  every CI platform); committed golden hashes are asserted on the pinned
  target (and on aarch64-macos, the bless machine). Empirical note: the
  first Tier 1 CI run (2026-07-03) matched the aarch64-macos-blessed hash
  exactly — with every DSP path on libm + pure arithmetic, the render is
  in practice byte-identical across these architectures, stronger than the
  tier promises. The *contract* remains per-pinned-target so a future
  divergence is a re-bless, not a breach.
- **Tier 2 — feature tolerances across platforms**: integrated LUFS within
  0.1 LU, onsets within 2 ms, pitch within 5 cents. These absorb the
  cross-target float differences Tier 1 does not promise away.
- **Tier 3 — spectrogram sentinels**: image diff with per-pixel tolerance
  and a max-fraction-differing threshold.

## Why "audio is a fold" shapes everything

Filters, envelopes, delays, and reverbs carry state; sample N depends on all
samples before it. So the reproducibility unit is the whole render, not the
sample. Consequences: fixed summation order everywhere (voices in index
order, stems in track order), one rounding rule at the tick→sample boundary,
and stochastic sources that are *random-access* (counter-based) so they
don't inherit the fold's ordering sensitivity.

## Denormal policy: honor denormals everywhere

**Decision: no FTZ/DAZ, no algorithmic offsets.** We never call fundsp's
`prevent_denormals()` (`fundsp/src/denormal.rs` — it sets x86 MXCSR to
0x9fc0, i.e. FTZ|DAZ, and is a no-op on aarch64). The audit found fundsp
calls it *itself* inside the `Feedback`/`Feedback2`/FDN node family — with
no restore, so one tick of a `feedback()` node flips FTZ on thread-globally
on x86. Those combinators are therefore banned via clippy
`disallowed-methods`; the stock `Reverb` doesn't use them and is fine.
(ebur128 also flips FTZ inside its K-weighting loop, but *scoped* with a
restore-on-drop, analysis-side only — accepted, see its audit section.)

Rationale, in order:

1. **Determinism.** IEEE 754 fully specifies denormal (subnormal) results
   for `+ − × ÷` and `sqrt`. Honoring them is bit-deterministic on one
   machine *and* bit-uniform across architectures. Flushing is neither: the
   kickoff's FTZ option can't even be implemented uniformly (x86 MXCSR vs
   aarch64 FPCR; fundsp only covers x86, and our workspace forbids the
   `unsafe` needed to touch FPCR ourselves).
2. **The performance argument doesn't apply offline.** FTZ exists so
   realtime reverb/filter tails don't blow the audio callback budget. We
   render offline; a rare 10–100× slowdown on a handful of samples in a
   decaying tail costs milliseconds of wall clock, not glitches.
3. **The f64 master bus makes denormals rarer where it matters.** f32
   subnormals start at ~1.2e−38; summing at f64 keeps the bus far from its
   own subnormal range (~2.2e−308).

Trade-off accepted: fundsp's f32 filter internals may briefly process
subnormal state values at tail ends, slower than flushed. If profiling ever
shows a pathological case, the phase 2 revisit is an *algorithmic* fix
applied uniformly (documented tiny offset), never an FPU-flag fix.

## Toolchain and codegen pins

- `rust-toolchain.toml` pins `1.95.0`; CI installs exactly that via rustup's
  toolchain-file support. Bumping the toolchain is a deliberate commit that
  re-blesses golden hashes if they move.
- No fast-math: we never enable `-ffast-math`-style flags and don't set
  `RUSTFLAGS` codegen options that relax float semantics.
- No implicit FMA: Rust never contracts `a * b + c` into fma by default;
  `mul_add` is in the clippy `disallowed-methods` ban list so fusion is
  always an explicit, `#[expect]`-documented choice.
- Std float transcendentals are banned in this workspace by `clippy.toml`
  `disallowed-methods` (delegated to platform libm — different results per
  OS/libc). DSP code uses the `libm` crate, which is pure-Rust and
  bit-stable across platforms. Std `sqrt` is exempt: IEEE-exact, hardware
  instruction everywhere.

## fenestra-anim 0.1.0 (audited by reading the published source)

What the timebase and automation path actually execute:

- `mul_div(a, b, c, Rounding) -> u64` (`src/rational.rs`): u128
  intermediate, explicit `Floor | Ceil | Round` (ties up), panics on `c == 0`
  and on u64 overflow of the result. Pure integer math — exact and
  platform-independent. This is the only tick→sample primitive we use.
- `Track<T>::sample(frame, fps)` → `locate()` (`src/track.rs`): segment
  lookup by `partition_point` (integer), progress
  `u = (run as f64 / span as f64) as f32` (exact-division semantics,
  deterministic), then easing:
  - `Ease::Linear`, `Ease::Hold`: pure arithmetic. Deterministic everywhere.
  - `Ease::Bezier` (`src/bezier.rs`): fixed 16-iteration Newton solve using
    only `* + - /` and `clamp` — no transcendentals. Deterministic
    everywhere.
  - `Ease::Spring` (`src/spring.rs`): std f32 `exp/cos/sin/sqrt` — platform
    libm, NOT cross-platform bit-stable. **v1 rejects springs on automation
    tracks at validation** (also for the seconds-grounding reason in
    `docs/plan.md`), so no fenestra-anim transcendental runs in any v1
    signal path. Springs stay available to non-audio consumers upstream.
- `Interpolate for f32`: `a + (b - a) * t`, pure arithmetic.

Conclusion: cochlea's automation evaluation (linear/hold/bezier over ticks)
is bit-deterministic across platforms, stronger than Tier 1 requires.

## fundsp 0.23.0 audit

Manifest facts (verified in the vendored crate): depends on `libm` directly;
`wide` (SIMD f32x8) is unconditional; `funutd` is the RNG dependency;
default features `files` (symphonia) and `fft` (fft-convolver) are disabled
in our workspace pin (`default-features = false, features = ["std"]`), which
keeps symphonia out of Cargo.lock entirely (also enforced by a deny.toml
ban).

Audit findings (full-source read of the vendored crate; per-node-family
detail with file:line citations retained in the audit transcript, digest
here):

- **The node interface is f32, full stop.** `AudioNode::tick`/`process` are
  hardcoded to `Frame<f32, _>` (`audionode.rs:75,79`); `prelude64` only
  upgrades *internal* state precision (`Sine<f64>` phase, filter state) and
  still emits f32 per sample. Consequence: fundsp renders per-voice f32;
  cochlea's f64 buses (voice sum, master sum) live outside fundsp's node
  boundary, in `render`. We use the `prelude64` node constructors where
  offered (f64 internal accumulators cost nothing offline).
- **`tick()` and `process()` provably diverge.** `Sine::tick` computes
  `sin` via `libm::sinf` (`oscillator.rs:71` → `lib.rs:480`);
  `Sine::process` computes it via `wide`'s f32x8 SIMD polynomial
  (`oscillator.rs:82` → `lib.rs:632`) — different algorithms, no
  bit-equality contract. SIMD `floor`/`ceil` are even approximations
  (`(x ± 0.4999999).round()`, `lib.rs:326-332`). **Rule: voices tick
  sample-by-sample; `AudioNode::process`/`AudioUnit::process` are in the
  clippy `disallowed-methods` ban list.** One code path, one rounding story.
- **Scalar transcendentals all route through libm** — exhaustively
  confirmed: `Num`/`Float`/`Real` impls for f32/f64 call
  `libm::{sinf,cosf,tanf,expf,exp2f,logf,log2f,log10f,tanhf,atanf,powf,...}`
  (`lib.rs:168-280,444-594,773-825`). No std float intrinsics in the scalar
  path. Per-sample transcendentals in nodes we use: `Sine::tick` (sin),
  `Moog::tick`/`Rez::tick` (tanh) — all libm. Biquad/Lowpole/Highpole
  coefficient math (tan/sin/cos/exp) runs at construction/set-parameter
  time only. None of Biquad, Moog, Rez, Reverb, WaveSynth, Pluck, ADSR
  override `process()` anyway — the divergence risk is concentrated in
  oscillators like `Sine`; the tick-only ban covers everything uniformly.
- **No entropy anywhere.** Zero hits for
  `SystemTime|thread_rng|rand::|Instant|getrandom|OsRng` across the crate;
  no `rand` dependency. All node seeding derives from the structural graph
  hash chained through `ping()`/`AttoHash` (deterministic function of node
  IDs and combinator positions), overridable per node via
  `Setting::Seed(u64)`. Two structurally identical graphs get identical
  output. Cochlea presets set explicit seeds/phases anyway so sound
  identity survives graph refactors.
- **fundsp's own denormal handling is x86-only, scattered, and
  side-effectful**: `prevent_denormals()` is called from exactly one node
  family — `Feedback`/`Feedback2`/FDN (`feedback.rs:129,136,241,248,357,370`)
  — and sets MXCSR FTZ|DAZ *thread-globally* with no restore, a no-op on
  aarch64. Under the honor-denormals policy those combinators are **banned**
  (`feedback`, `feedback2`, `fdn`, `fdn2`, `prevent_denormals` in the clippy
  ban list). **Verified consequence: every fundsp stereo reverb constructor
  is off-limits** — `reverb_stereo` builds `fdn::<U32>` directly
  (`prelude.rs:1755`) and `reverb4_stereo_delays` builds two `fdn`s
  (`prelude.rs:1938-1939`); the clippy ban cannot see those *internal* call
  sites, so the rule is: no fundsp reverb constructors at all. The reverb
  insert is instead an in-repo Freeverb-style Schroeder (8 damped combs +
  4 allpasses per channel, `cochlea-synth/src/nodes.rs`), pure arithmetic
  per tick.
- **ADSR reset is a trap**: `adsr_live`'s closure captures `attacked` +
  two `Shared` cells that `reset()` cannot see (`adsr.rs:29-33`). Voices are
  therefore always freshly constructed per note, never pooled-and-reset —
  which the render engine does anyway by design.
- **`Net`/`Sequencer` use hashmaps only for keyed lookup** (execution order
  is a Vec-based topological sort, `net.rs:834-917`), but their live-edit
  frontends exist for realtime use; cochlea builds static `An<_>` graphs and
  never uses `Net` commit APIs.
- **funutd (fundsp's RNG dep) stays out of our signal path**: fundsp's
  `noise()`/`pluck()`/`hold()` nodes (funutd-seeded, and `Pluck`'s
  excitation is a stateful `funutd::Rnd` replay) are not used by cochlea
  presets. Noise and Karplus-Strong excitation come from the in-repo
  counter RNG keyed `(seed, sample_index)`; the KS voice is hand-rolled
  (delay line + damping FIR as a custom `AudioNode`) rather than fundsp's
  `Pluck`. funutd remains in Cargo.lock as an unused-at-runtime transitive
  dep.
- Versions that can move last-bit float behavior — `fundsp`, `libm`,
  `wide`, `funutd` — are pinned by Cargo.lock (committed); toolchain pinned;
  no `-C target-cpu=native` anywhere.

Per-preset Tier 1 verdicts (all under the tick-only rule): `sine` safe
(libm sin); `saw_lead`/`square_bass`/`chord_pad` safe (WaveSynth's
interpolation is pure arithmetic, filters libm-at-construction, ADSR fresh
per voice — implemented as our own piecewise-linear closure over note time
rather than fundsp's `adsr_live`, sidestepping its closure-state reset trap
entirely); `noise_hat` safe (counter-RNG noise + filter); `pluck` safe
(hand-rolled KS, counter-RNG excitation); `reverb` insert safe (in-repo
Schroeder — fundsp's reverb constructors are all FDN-based and banned, see
above).

## ebur128 0.1.10 audit

API facts the features crate builds on (file:line citations in the audit
transcript):

- Construct `EbuR128::new(channels, rate, mode)`; we use
  `Mode::I | Mode::TRUE_PEAK` (TRUE_PEAK implies SAMPLE_PEAK and M).
  Feed interleaved `add_frames_f32(&[f32])`. Default 2-channel map is
  `[Left, Right]` — correct for us, no `set_channel` needed.
- Readouts: `loudness_global()` → LUFS (needs `Mode::I`),
  `loudness_momentary()` → LUFS over the last 400 ms (momentary *max* is
  ours to track: feed in 100 ms chunks and take the running max of
  readings), `true_peak(ch)`/`sample_peak(ch)` → **linear amplitude** — we
  convert to dBTP/dBFS via `20·log10` (libm) ourselves.
- Gating confirmed BS.1770-4: absolute −70 LUFS gate (energies below the
  first histogram boundary are discarded before recording,
  `history.rs:264-267`, boundary derived from the −70 LUFS energy) and
  relative −10 LU gate (`history.rs:320-334`), 400 ms blocks at 75%
  overlap.
- Silence/not-enough-audio is **not an error**: `loudness_*` return
  `Ok(-inf)`, peaks `Ok(0.0)`. The features crate maps `-inf` to a JSON
  `null`-with-reason, never an error.
- True peak: rate-dependent oversampling (4× below 96 kHz), 48-tap
  Hanning-windowed-sinc polyphase FIR with coefficients computed once at
  construction; FIR math is f32; no SIMD; the `precision-true-peak`
  feature (FMA in the FIR) stays **off**.
- Internal math is otherwise f64. One arch-conditional path, flagged: the
  K-weighting filter loop enables **scoped** hardware FTZ on x86_64
  (`filter.rs:376-428`, restored on drop) and approximates it on other
  arches by flushing filter state below `f64::EPSILON` at block boundaries.
  So loudness readings can differ at ULP level between x86_64 and aarch64.
  This is analysis-side only (never touches PCM), scoped (does not leak
  MXCSR state to our thread beyond the call), and absorbed by the Tier 2
  0.1 LU tolerance with ~5 orders of magnitude of headroom. Accepted.

## rustfft 6.4.1 audit

- `FftPlanner::new()` does **runtime CPU-feature dispatch** (AVX+FMA → SSE4.1
  → NEON → WASM → scalar). Same binary, different machine ⇒ different FFT
  bits; and a cloud CI runner migration could silently flip the path on the
  "pinned" target.
- **Decision: analysis code constructs `FftPlannerScalar` explicitly,
  everywhere** (`rustfft::FftPlanner::new` is in the clippy ban list). One
  code path on every machine, immune to runner-hardware drift. Our FFTs are
  small (1024–2048 points at audio hop rates); scalar is more than fast
  enough offline, and this choice makes features/spectro deterministic
  per-binary, not just per-platform.
- Planning is a pure function of `(len, direction)` for the scalar planner;
  `process()` panics on length mismatch (caller invariant); output is
  unnormalized (we only use magnitudes, and normalize where needed,
  consistently).
- rustfft is used only in `features` and `spectro` — never in the PCM
  render path — so even its residual cross-toolchain variance is bounded by
  Tier 2 tolerances and Tier 3 image diffs by construction. MSRV 1.61,
  deps purely numeric.

## Rounding rules (the complete list)

1. `Bpm(f64)` → integer nanoseconds-per-quarter (`round`), once, at
   authoring/parse time. The stored score is exact from then on.
2. Tick → sample: `mul_div(Δticks, npq · sr, ppq · 1e9, Rounding::Round)`
   per tempo segment with left-to-right integer anchors; applied once at
   event-schedule time. Property-tested monotone and drift-free.
3. Sample → tick (block starts → automation domain): same segment math with
   `Rounding::Floor` (a block start belongs to the tick it is inside).
4. f64 master bus → f32 output samples: default Rust `as f32`
   (round-to-nearest-even), documented here, applied at the very end.

Nothing else in the pipeline rounds between integer domains.

## v2 analyzers and decode (wave 2 additions)

The wave-2 surface introduces no new determinism mechanism — it reuses the
existing rules — but each addition deserves its line in this ledger:

- **FLAC decode (`cochlea-decode`, symphonia 0.6, FLAC feature only).**
  FLAC reconstruction is pure integer arithmetic by spec; the only place a
  decode could diverge from a WAV twin is float normalization.
  `symphonia-bundle-flac` left-justifies every sample into 32 bits (its
  own documented "common denominator"), so the crate divides by `2^31`
  unconditionally — exactly equal to the WAV path's per-depth
  `2^(bits-1)` divide (both are the same power-of-two rescale, exact in
  IEEE 754). Enforced bit-for-bit by committed WAV/FLAC twin fixtures.
  Zero-packet streams take their shape from STREAMINFO and their sample
  vector stays empty — no fabricated rates.
- **Tempo (`tempo.rs`), stereo (`stereo.rs`), structure (`structure.rs`),
  segment timeline (`segments.rs`).** All transcendentals via `libm`; the
  onsets-grade STFT they share comes from the same `FftPlannerScalar`
  path as everything else; autocorrelation and the novelty kernel use
  fixed ascending summation order; every float sort uses `total_cmp`. The
  analyzers are pure functions of the buffer — `probe()` computes shared
  intermediates (STFT, onset report, YIN track) once and fans them out,
  which is a pure refactor precisely *because* the passes were
  deterministic duplicates.
- **Non-finite input policy.** Float WAVs can legally encode NaN/±inf;
  IEEE 754 comparison semantics would let a single NaN sample masquerade
  as silence in one analyzer and a real peak in another. `Audio::from_wav`
  rejects non-finite samples at ingestion (`NonFiniteSample`), so no
  analyzer ever sees one. Degenerate *options* (NaN window/frame lengths,
  non-finite silence floors) are guarded with `is_finite()` checks — a
  plain `<= 0.0` range check is NaN-blind.
- **Text outputs (digest, compare).** Fixed-precision `{:.N}` formatting
  only, stable field order, no wall clock, no hash-map iteration — byte-
  deterministic per platform for identical input; the numbers inside stay
  Tier-2 across platforms like every other analysis float.

## 0.2.0 additions (2026-07-21)

- **Tempo/rhythm split.** Pulse clarity (mean-removed, length-unbiased
  autocovariance), tempo candidates, windowed stability, and the
  grid-alignment rhythm analyzer are all pure arithmetic + `libm` over
  the same shared STFT, fixed summation order throughout. The Ellis
  beat-DP envelope is normalized by its own standard deviation —
  a pure function of the buffer, so normalization changes nothing about
  determinism, only the penalty calibration.
- **`kick`/`snare` presets, stereo `chord_pad`.** Drum envelopes are
  rational `1/(1+t/tau)` decays — pure arithmetic per sample, no new
  transcendental call sites; the pad's two saws pan through the same
  constant-power `fd::pan` used everywhere. **Golden re-bless:** the
  drum-groove golden moved to `0xE613_FEF7_9664_B529` (deliberate DSP +
  score change: real kit, per-track pans, stereo pad); first-light was
  untouched (it uses none of the changed patches) and its hash did not
  move — an incidental cross-check that the preset work leaked into
  nothing it shouldn't have.
- **Banded structure similarity.** Computes exactly the entries the
  checkerboard kernel reads, in the same order, same arithmetic —
  byte-identical novelty curves to the full matrix, minus the O(n²)
  waste. The frame-count cap changes the effective frame length only as
  a deterministic function of input length.
- **Spectral centroid (`centroid.rs`).** Weighted mean over the shared
  scalar-FFT magnitudes; a silent frame's centroid is `None`, never a
  ratio of float-noise sums.
- **MCP base64/PNG.** `encode_png` runs the identical `image` encoder as
  the file path; base64 is a pure byte mapping — the inline spectrogram
  and the PNG on disk are the same bytes.

## 0.3.0 additions (2026-07-22)

- **Master bus (gain + limiter).** The bus becomes `Σ stems (f64) →
  master → single f32 rounding` — rounding rule 4 is unchanged and still
  applied exactly once. A default master returns before touching a
  sample, so master-less scores render byte-identically to 0.2.0 (both
  golden hashes confirmed unchanged). The limiter is pure arithmetic
  plus two `libm` calls at setup (`pow` for the ceiling, `exp` for the
  release coefficient): per-frame peaks, a forward sliding-window
  maximum via a monotonic deque (integer logic, no float accumulation),
  and a one-pole release. Gain is clamped by `ceiling / windowed_peak`
  at every frame, so the sample-peak ceiling is exact by construction on
  every platform — no fast-math, no FMA, nothing target-dependent.
- **Melody + timbre analyzers.** Melody is integer quantization and run
  grouping over the existing YIN track (`libm::log2`/`exp2` only, the
  same calls the pitch report already made). MFCC is a hand-rolled mel
  filterbank + orthonormal DCT-II over the shared scalar-FFT
  magnitudes — `libm` transcendentals, fixed summation order, and a
  per-frame dynamic-range floor that is a pure function of the frame.
- **Triplet-grid rhythm hypothesis.** Grid geometry only — the same
  classifier run at two subdivision counts, winner by aligned count with
  a deterministic tie-break (straight). No new signal pass.
- **Lossy decode (mp3/ogg) is Tier-2-adjacent, not Tier-1.** The lossy
  path is analysis input only: symphonia is pure Rust and our feature
  set keeps its `opt-simd` runtime dispatch *off*, so decoding the same
  file with the same build is reproducible — but no bit-exactness claim
  is made (the codec discarded the original samples), and nothing lossy
  can reach the render path. The FLAC module and its WAV-twin
  bit-exactness test are untouched.
- **MIDI import.** Integer tick arithmetic end to end: SMF division
  becomes PPQ verbatim, deltas sum in u64 with overflow checks, tempo
  metas convert through the same `Bpm → ns/quarter` rounding rule 1 as
  authored tempos. No float time anywhere in the parser.
- **Spectrogram overlays and diffs.** Drawing is integer pixel writes at
  positions derived from sample offsets; the diff image is per-cell f32
  subtraction of two already-computed spectrograms. `MelSpec::hz_band`
  uses the same HTK mel formula as the filterbank (`libm`), so overlay
  placement is as deterministic as the spectrogram itself.

## 0.4.0 additions (2026-07-24)

- **Harmony (chords + per-section key).** Reads the same shared chroma
  STFT the key estimate already builds. Chord detection is a cosine match
  against L2-normalized binary templates in fixed order; the per-section
  key runs the exact Krumhansl-Schmuckler correlation the global key uses.
  Only `libm::log2` and the exempt std `sqrt` — no new transcendental call
  sites, fixed summation order throughout.
- **Loudness timeline + short-term max.** ebur128 is fed in fixed-size
  chunks and polled at a fixed hop; the running maxima are tracked in
  order. Same audit as integrated LUFS — `-inf`/undefined maps to `null`,
  never a non-finite float.
- **Integer PCM output (`WavBitDepth`).** 16/24-bit WAV is a deterministic
  clamp to `[-1, 1]`, scale by `2^frac`, round to nearest, clamp to the
  signed range — no dither (dither would trade determinism for a lower
  noise floor, the wrong trade here). Float32 stays the render's ground
  truth; the integer paths are a lossy convenience, not a second source of
  truth.
- **`fm_bell` preset.** Harmonic FM (a `sine` frequency-modulated by a
  harmonic-ratio `sine`), amplitude and modulation-index both driven by
  the same control-rate ADSR envelopes every other voice uses — pure
  arithmetic per sample, `libm` only at construction. It uses none of the
  patches `first_light`/`drum_groove` play, so both golden hashes are
  unchanged (the same incidental cross-check the drum presets got).
- **MIDI export.** Score ticks become SMF ticks verbatim (PPQ is the file
  division); only the tempo value is microsecond-quantized, which is all
  SMF can carry. Delta-times are integer VLQ. No float time anywhere in
  the writer — the exact inverse of the import path's integer arithmetic.
- **Hardening.** The authored-tick bound (`Ticks::MAX`) is a pure integer
  comparison; the decode sample caps are integer counts checked as they
  accumulate; the MCP `catch_unwind` backstop wraps dispatch only and
  never touches the render fold. None of the three affects rendered bytes.

## 0.5.0 additions (2026-08-06)

- **`marimba` and `organ` presets.** Both are pure arithmetic over exact
  harmonics: `marimba` sums modal sine partials under `1/(1+t/tau)²` decays
  (plus a short linear fade before retirement, the same guard `pluck` uses);
  `organ` is a weighted sine sum under the piecewise-linear ADSR. No new
  transcendental call sites, no fundsp `feedback`/`fdn`/`process`. Golden
  hashes are unchanged — no flagship score plays them.
- **Loudness/beat-grid surfaces.** No new DSP — they reuse the existing
  `loudness_timeline` and `estimate_tempo` analyzers (ebur128 polled at a
  fixed hop; the shared scalar-FFT tempo path), served over new CLI flags and
  MCP tools.

## 0.6.0 additions (2026-08-07)

- **`transcribe` (audio → score).** Analysis times are inherently `f64`
  milliseconds — a tracker measures frames, not ticks — so this path has one
  float→integer conversion, and it is confined to exactly one place:
  `ms_to_ticks` in `crates/score/src/transcribe.rs`, applied **once per note
  boundary** and immediately rounded to `u64` with `libm::round`. Nothing
  accumulates in float: quantization, the minimum-duration floor, and the
  `Ticks::MAX` bound are all integer arithmetic on the rounded value. This
  is the same shape as the existing authoring-time rounding (`Bpm` →
  integer ns per quarter): float in, integer immediately, integer
  thereafter.
  - Non-finite and negative inputs are rejected at that boundary rather
    than propagated, so a NaN from an analyzer can never reach the tick
    domain.
  - Ordering is deterministic: observations sort by
    `(start_ms, midi)` with `total_cmp`, so equal starts break ties by
    pitch rather than by input order.
  - No new DSP and no new transcendental call sites — velocity estimation
    is `libm::log10` over an existing peak fold, and the render path is
    untouched. Golden hashes are unchanged.
- **MIDI time-signature denominator.** The importer's `1u32 << payload[1]`
  became `checked_shl`. This is a malformed-input fix, not a numeric one:
  the shifted value never reaches the render fold (an out-of-range
  denominator is warned about and the default 4/4 kept), so rendered bytes
  are unaffected.

## 0.7.1 additions (2026-08-13)

- **Position arithmetic is checked, and the answer is the same everywhere.**
  `Pos::resolve`'s `(bar - 1) * ticks_per_bar` was unchecked with both
  factors caller-supplied. In debug that panicked; in release — the profile
  every published binary is built with — it *wrapped*, which is the worse
  outcome for this workspace: the same score loaded to different ticks under
  different build profiles, i.e. a determinism failure dressed as an
  overflow bug. Now `checked_mul`/`checked_add` plus the existing
  `Ticks::MAX` bound, so a position either resolves to one tick on every
  build and every host, or is refused. No rendered sample changes; the
  golden PCM hashes are unchanged.
- **Case folding joins the portability rule.** The "one score means one
  thing on every host" argument recorded below for stem *names* now also
  covers the paths those names collide with: two outputs differing only by
  case are refused on Linux too, because they are one file on macOS and
  Windows. Same trade, same reason — the answer should not depend on which
  machine asked. The NFC/NFD limit below is unchanged.

## 0.7.0 additions (2026-08-11)

- **Stem-name validation: no bit-level change, one cross-platform rule.**
  The stem-name fix (`cochlea_render::stem_file_name`, plus containment of
  the resolved stem path) touches only *which files a render is willing to
  write*, never a sample. No DSP, no new transcendental call sites, no
  change to the schedule or the bus. Both golden PCM hashes are unchanged
  and were re-confirmed on the Tier 1 target by CI.
- **Why the name rule is enforced on every platform, not per-host.** A
  score is portable data, so the checks that only *bite* on Windows — `\`
  as a separator, `:` as a drive or alternate-data-stream marker, the
  `<>"|?*` set, the `CON`/`NUL`/`COM1` device names — are refused on Unix
  too. The alternative is worse than a false rejection: `C:` was previously
  accepted on Unix and rejected on Windows, so one score exported stems on
  the Tier 1 target and errored on a Tier 2 one. That is the same
  commitment as the bit-exact render, applied to the filesystem edge — the
  answer should not depend on which machine asked.
  - Known limit, recorded rather than half-solved: names colliding only
    under Unicode normalization (NFC vs NFD) are *not* caught, because
    normalization would mean a new dependency in a workspace that keeps its
    graph deliberately small. Case-insensitive collisions are caught.
