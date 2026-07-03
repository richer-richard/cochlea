# Determinism: decisions, audits, and rationale

This file records the Phase 0 determinism decisions and the dependency
audits behind them. The contract itself (three tiers) is summarized in
`CLAUDE.md`; this is the evidence and the fine print.

## The three tiers, restated precisely

- **Tier 1 — byte-identical PCM** for identical inputs on the pinned target:
  `x86_64-unknown-linux-gnu`, toolchain `1.95.0` (`rust-toolchain.toml`).
  Two renders of the same score are byte-equal on *any* machine (tested on
  every CI platform); committed golden hashes are only asserted on the
  pinned target, because codegen (instruction selection, libm vs system
  differences) varies across targets.
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
  ban list). The stock `Reverb` (`reverb.rs`) is hand-rolled
  allpass/delay structure, does not touch `prevent_denormals`, and is fine.
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
per voice); `noise_hat` safe (counter-RNG noise + filter); `pluck` safe
(hand-rolled KS, counter-RNG excitation); `reverb` insert safe (fixed
constructor parameters, construction-time libm only, no
`generate_reverb()` — that is a dev-time genetic-search tool with its own
RNG surface, never called).

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
