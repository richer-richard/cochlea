# cochlea v1 plan: crate-by-crate public API surface

Written in Phase 0, before implementation. Where implementation reality
diverges, this file is updated in the same PR (deviations get a **Deviation**
marker). Determinism decisions and the dependency audits live in
`docs/determinism.md`; the contract summary lives in the README.

## Phase 0 decisions (deltas and pins)

- **fenestra-anim 0.1.0 verified** on crates.io. It provides everything
  required: `mul_div(u64, u64, u64, Rounding{Floor,Ceil,Round}) -> u64`
  (u128 intermediate, panics on div-by-zero and u64 overflow), typed
  `Track<T: Interpolate>` / `Key<T>` / `key(at, value).ease(e)`, easing set
  (`linear`, `hold`, `ease_in`, `ease_out`, `ease_in_out`, `spring`,
  `CubicBezier`, `Spring`), `Interpolate` (f32 and (f32, f32)), and serde
  behind the `serde` feature. MSRV 1.88, `unsafe_code = "forbid"`.
- **`keys!` lives in cochlea-score, not upstream.** fenestra-anim exports the
  `key()` constructor; the `keys![...]` macro from the kickoff sketch is
  authoring sugar over *cochlea tick positions* (`bar(1).beat(1)` etc.),
  which fenestra-anim knows nothing about. Defining it here is not a fork of
  easing math — it expands to `fenestra_anim::key(ticks, value).ease(e)`.
  No upstream PR needed.
- **Spring easing is rejected in v1 automation.** `Track::sample(frame, fps)`
  grounds springs in seconds via an integer `fps`; automation runs in tick
  space where "ticks per second" is tempo-dependent and generally
  non-integer. Rather than evaluate springs subtly wrong, score validation
  rejects `Ease::Spring` on automation tracks (linear/hold/bezier are fine —
  they never read `fps`). Phase 2 can PR fractional-rate sampling upstream.
- **Tempo is stored as integer nanoseconds per quarter note** (`u64`),
  converted from `Bpm(f64)` once at authoring time (`round`), like MIDI's
  µs-per-quarter but 1000× finer. All tick→sample math after that point is
  exact rational (see score crate below). Valid BPM range [1.0, 4000.0],
  validated.
- **Denormal policy: honor denormals everywhere** — no FTZ/DAZ, no
  algorithmic offsets. Full rationale in `docs/determinism.md`; short form:
  IEEE 754 fully specifies denormal arithmetic, so *not* flushing is the
  most deterministic and the most cross-platform-uniform choice, and offline
  rendering doesn't care about the occasional slow filter tail. We never
  call fundsp's `prevent_denormals()`.
- **Pins** (resolved on crates.io 2026-07-03): fundsp 0.23.0
  (default-features off, `std` only — keeps symphonia and fft-convolver
  out), ebur128 0.1.10, rustfft 6.4.1, hound 3.5.1, ron 0.12.2, libm 0.2.16,
  image 0.25.10 (png only), serde 1.0.228, rayon 1.12.0, clap 4.6.1,
  proptest 1.11. Toolchain pinned 1.95.0; MSRV 1.88 (floor set by
  fenestra-anim and image).
- **Naming deviation, recorded:** the user-facing name `Instrument` belongs
  to the score IR (`score::Instrument::preset("saw_lead")` — what a score
  *references*, serializable data). The synth-side trait (how a name becomes
  sound: param registry + fundsp voice graphs) is `synth::Patch`. The
  kickoff sketch used `Instrument` for both sides; one name for two types in
  two crates would be permanently confusing at call sites.
- **Stereo throughout v1.** Every track renders 2 channels; mono voice
  graphs are panned center. Analysis that wants mono (YIN) downmixes
  `(l + r) / 2`.
- **Audit-driven rules** (details in `docs/determinism.md`, enforced via
  `clippy.toml` bans): voices tick sample-by-sample, never fundsp
  `process()`; fundsp `feedback()`/`fdn()` combinators banned (internal
  thread-global x86 FTZ); ADSR/voice graphs freshly constructed per note,
  never pooled-and-reset (fundsp ADSR closure state survives `reset()`);
  `pluck` is a hand-rolled Karplus-Strong `AudioNode` (fundsp's `Pluck`
  seeds its excitation from stateful funutd RNG) with counter-RNG
  excitation; fundsp `noise()`/`hold()` unused — all noise is the in-repo
  counter RNG; analysis FFTs construct `FftPlannerScalar` explicitly;
  fundsp presets use the `prelude64` constructors (f64 internal state, f32
  node interface); ebur128 true/sample peaks return linear amplitude — we
  convert to dBTP/dBFS via libm, and `-inf` loudness readings mean
  "silence/no data", not errors.
- **The reverb insert is in-repo** (P2 finding): every fundsp stereo reverb
  constructor (`reverb_stereo`, `reverb4_stereo_delays`) builds on `fdn()`
  internally, which flips thread-global x86 FTZ — and clippy bans can't see
  call sites *inside* fundsp. The `reverb` insert is a Freeverb-style
  Schroeder (`cochlea-synth`), pure arithmetic per tick, tail ~2.5 s.
  Envelopes are likewise our own piecewise-linear ADSR evaluated through
  `fundsp::envelope` closures (pure arithmetic; the offline schedule knows
  note length, so no live gate state exists at all).

## Dependency graph (acyclic, law-checked in CI)

```
score    → fenestra-anim, ron, serde
synth    → score, fundsp, libm
render   → score, synth, hound, rayon
features → ebur128, rustfft, hound, libm, serde        (NEVER score/synth)
spectro  → rustfft, hound, image, libm                  (NEVER score/synth)
decode   → features, symphonia                          (NEVER score/synth)
verify   → score, features, render
cli      → everything
mcp      → score, synth, render, features, spectro, verify
```

---

## crates/score

Newtypes: `Ticks(pub u64)`, `Ppq(pub u32)`, `SampleRate(pub u32)`,
`Bpm(pub f64)`, `Vel(pub u8)` (1..=127), `Pitch(pub u8)` (MIDI number, consts
like `Pitch::A4 = 69`, `FromStr` for "A4"/"C#3"/"Bb2", `fn hz(self) -> f64`
via libm), `Dur { num: u32, den: u32 }` (fraction of a whole note —
`Dur::quarter() = 1/4`, `eighth`, `half`, `whole`, `sixteenth`,
`.dotted()` (×3/2), `.triplet()` (×2/3), `Dur::ticks(u64)` escape hatch;
resolved against PPQ exactly: `ticks = ppq * 4 * num / den` via `mul_div`).

Positions: `Pos` — built by the terse constructors from the sketch:

```rust
pub fn bar(n: u32) -> Pos;              // 1-based
impl Pos {
    pub fn beat(self, b: u32) -> Pos;   // 1-based within the bar
    pub fn plus(self, d: Dur) -> Pos;   // sub-beat offset, exact
}
```

`Pos` resolves to `Ticks` against `(Ppq, TimeSignature)` — 4/4 in v1 but the
math takes `(num, den)` so time-signature *support* isn't load-bearing
anywhere. Resolution is exact integer math; a `Pos` that lands on a
non-integer tick (impossible at 960 PPQ for any `Dur` with den | 3840) is a
validation error, not a rounding.

Tempo map:

```rust
pub struct TempoMap { /* sorted (Ticks, NanosPerQuarter) steps, ppq, sample_rate */ }
impl TempoMap {
    pub fn sample_at(&self, t: Ticks) -> u64;   // THE conversion — see below
    pub fn tick_at(&self, sample: u64) -> Ticks; // inverse, Floor
    pub fn ns_at(&self, t: Ticks) -> u64;
}
```

`sample_at` is the **one documented rounding rule**: per tempo segment `i`
starting at tick `T_i` with anchor sample `S_i`,

```
sample_at(t) = S_i + mul_div(t - T_i, npq_i * sr, ppq * 1_000_000_000, Rounding::Round)
```

Anchors are computed left-to-right with the same formula, so each tempo
change rounds at most once (±0.5 samples, no accumulation) and within a
segment there is zero drift. Monotonicity and exactness vs a direct u128
reference are property-tested. `u64` range check: worst case
`npq * sr = 6e10 × 192_000 ≈ 1.2e16 < u64::MAX`; enforced by the BPM and
sample-rate validation ranges (SR in [8_000, 192_000]).

Score builder (mirrors the sketch):

```rust
let score = Score::new(SampleRate(48_000), Ppq(960))
    .time_signature(4, 4)
    .tempo(Ticks(0), Bpm(120.0))
    .track("lead", Instrument::preset("saw_lead"))
    .insert("lead", Insert::preset("reverb"))      // per-track insert chain
    .note("lead", bar(1).beat(1), Dur::quarter(), Pitch::A4, Vel(96))
    .automate("lead", Param::CUTOFF_HZ,
        keys![(bar(1), 400.0, ease_in_out()), (bar(3), 4_000.0)]);
```

- `score::Instrument` — serializable reference: `Instrument::preset(&str)` |
  `Instrument::custom(&str)` (a *name* resolved from a caller-supplied bank
  at render time; the closure itself lives in synth — keeps score pure data,
  and RON containing a custom name fails rendering with a clear error unless
  the bank supplies it. Code-only, documented as non-serializable in spirit:
  lint warns on custom refs in `.ron` files).
- `Param(&'static str | owned)` newtype with consts `Param::CUTOFF_HZ`,
  `Param::GAIN`, `Param::PAN`, `Param::custom("...")`. Snake_case strings in
  RON.
- Automation: `keys![...]` expands to fenestra-anim keys; positions are
  `Pos`/`Ticks`, values f32, ease defaults linear. Stored per (track, param).
  Sampled at block starts by the renderer (control rate ≈1.3 ms at 64
  samples / 48 kHz — documented); blocks split at event boundaries keep
  note-ons/offs sample-accurate.
- Notes: `Note { at: Ticks, dur: Ticks, pitch: Pitch, vel: Vel }` post-
  resolution; builder keeps `Pos`/`Dur` authoring forms in the RON mirror.

RON data form (`version = 1`, round-trip tested both directions, one
committed example under `examples/scores/`):

```ron
Score(
    version: 1,
    sample_rate: 48000,
    ppq: 960,
    time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [
        Track(
            name: "lead",
            instrument: Preset("saw_lead"),
            inserts: [Preset("reverb")],
            notes: [Note(at: (1, 1), dur: "1/4", pitch: "A4", vel: 96)],
            automation: [
                Auto(param: "cutoff_hz", keys: [
                    Key(at: (1, 1), value: 400.0, ease: EaseInOut),
                    Key(at: (3, 1), value: 4000.0),
                ]),
            ],
        ),
    ],
    verify: [],   // assertion data forms, see crates/verify
)
```

Durations/pitches are strings in RON ("1/4", "1/8.", "A4", "C#3"); positions
are `(bar, beat)` tuples with an optional third sub-beat element
`(bar, beat, "1/16")`. Ease names: `Linear | Hold | EaseIn | EaseOut |
EaseInOut | Bezier(x1, y1, x2, y2)`.

Static validation (`score.validate(&impl Catalog) -> Vec<LintFinding>`):
overlapping notes on a mono instrument, params outside declared ranges,
automation targeting unknown params, tracks with no events, BPM/SR out of
range, spring ease on automation, custom instrument refs in data form.
`Catalog` is a small trait (param specs + polyphony per instrument name)
implemented by synth's registry — keeps score free of synth while letting
lints know param ranges.

Tests: unit (bar/beat/dur resolution exactness, tempo anchors), property
(monotone `sample_at`, zero drift over 1e9 ticks vs u128 reference,
`tick_at(sample_at(t)) == t` where exact), RON round-trip.

---

## crates/synth

```rust
pub trait Patch: Send + Sync {
    fn name(&self) -> &str;
    fn params(&self) -> &[ParamSpec];          // typed registry
    fn polyphony(&self) -> Polyphony;          // Mono | Poly(NonZeroU8)
    fn release_ticks_hint(&self) -> f64;       // release tail in seconds — voice lifetime is static
    fn voice(&self, note: VoiceCtx) -> Box<dyn AudioUnit>;  // fresh fundsp graph per note
}

pub struct ParamSpec { pub param: Param, pub unit: Unit, pub range: RangeInclusive<f32>, pub default: f32 }
pub struct VoiceCtx { pub pitch: Pitch, pub vel: Vel, pub sample_rate: SampleRate,
                      pub note_seed: u64,     // hash(track, note index) — feeds counter RNG
                      pub start_sample: u64 } // absolute, for (seed, sample_index) noise keying
```

- Param application: each automatable param is a fundsp shared variable
  (`shared()`/`var()`) wired into the voice graph; the renderer sets them at
  block starts. Settings are plain f32 stores — deterministic.
- `presets()` registry: `sine`, `saw_lead` (filtered saw + ADSR),
  `square_bass` (mono), `chord_pad` (detuned saws + LPF), `noise_hat`
  (counter-RNG noise + fast envelope), `pluck` (Karplus-Strong, stateful but
  deterministic; excitation from the counter RNG, not fundsp's noise).
  Registry implements `score::Catalog` for lints.
- `Insert` presets: `reverb` (fundsp reverb, fixed parameters, applied
  per-track post voice-sum). Registry entry like patches.
- **Counter RNG** (in-repo, the only randomness in the workspace):

```rust
pub fn crng(seed: u64, index: u64) -> u64;    // SplitMix64-style finalizer of seed ^ mix(index)
pub fn crng_f32(seed: u64, index: u64) -> f32; // uniform [-1, 1)
```

  Noise is a pure function of `(seed, sample_index)` — random access, no
  state. fundsp's `noise()`/`hold()`/`Pluck` are not used (audited: funutd-
  backed, stateful); the hat's noise source is a custom `AudioNode` over
  `crng`, and pluck is a hand-rolled Karplus-Strong node with `crng`
  excitation.
- All transcendentals in our DSP code go through `libm`. fundsp internals
  are covered by the audit in determinism.md.

Tests: two identical voices produce identical buffers; `crng` reference
vectors; param registry ranges match preset defaults; each preset renders
non-silence and decays after release.

---

## crates/render

```rust
pub fn render(score: &Score) -> Result<Rendered, RenderError>;                    // presets only
pub fn render_with(score: &Score, bank: &PatchBank) -> Result<Rendered, RenderError>; // + customs

pub struct Rendered {
    // interleaved stereo f32; mix defined as f64-sum of the f32 stems in
    // fixed track order, then f64→f32 — so mix == sum(stems) byte-exactly
    pub fn mix(&self) -> &[f32];
    pub fn stems(&self) -> impl Iterator<Item = (&str, &[f32])>;
    pub fn sample_rate(&self) -> SampleRate;
    pub fn write_wav(&self, path: &Path) -> Result<...>;      // 32-bit float WAV (hound)
    pub fn write_stems(&self, dir: &Path) -> Result<...>;
}
```

- **Event schedule** (pure): score → per-track sorted `Vec<Event>` with
  sample indices resolved once via `TempoMap::sample_at` (the single
  rounding). Events: NoteOn/NoteOff. Ordering: by sample, then track order,
  then note index — total and documented.
- **Blocks**: fixed 64-sample max, split at event boundaries. Automation
  sampled at block-start ticks (`TempoMap::tick_at` of block start) and
  written to shared params before the block ticks.
- **Voices**: per-track pool sized by `Patch::polyphony`. Allocation and
  stealing (oldest note) are functions of the schedule alone; voice lifetime
  is `note span + release tail` computed statically. Voices tick
  sample-by-sample (`AudioUnit::tick`), never fundsp block `process()` —
  one code path, one rounding story (see determinism.md on SIMD).
- **Track render**: voices sum into an f64 stereo accumulator in voice-index
  order → insert chain (f32 in fundsp) → f64 track gain/pan → f32 stem.
- **Master**: stems summed at f64 in fixed track order → f32 mix.
- **Parallelism**: rayon over tracks; each track fully independent;
  deterministic fold in track order afterward. `parallel == serial` tested
  byte-for-byte.

Tests: same score twice → byte-equal; parallel == serial; stems f64-sum ==
mix byte-equal; voice stealing scenario (poly 2, 3 overlapping notes →
oldest stolen, schedule-derived); golden PCM SHA-256 on the Tier 1 target
(cfg-gated).

---

## crates/features

Input type (no score, no synth — ever):

```rust
pub struct Audio { pub samples: Vec<f32> /* interleaved */, pub channels: u16, pub sample_rate: u32 }
impl Audio { pub fn from_wav(path: &Path) -> Result<Self, ...>;
             pub fn mono(&self) -> Vec<f32>; }

pub fn probe(audio: &Audio, opts: &ProbeOpts) -> Report;
```

`Report` (serde, `schema_version: 1`):

```json
{
  "schema_version": 1,
  "source": { "sample_rate": 48000, "channels": 2, "samples": 480000, "duration_ms": 10000.0 },
  "loudness": { "integrated_lufs": -14.2, "momentary_max_lufs": -10.1,
                "true_peak_dbtp": -1.3, "sample_peak_dbfs": -1.5 },
  "onsets":   { "count": 17, "times_ms": [...] },
  "pitch":    { "voiced_ratio": 0.82, "median_f0_hz": 440.1,
                "segments": [ { "start_ms": 0.0, "end_ms": 500.0, "f0_hz": 440.1, "midi_nearest": 69, "cents_off": 0.4 } ] },
  "key":      { "tonic": "C", "mode": "major", "confidence": 0.87, "chroma": [/* 12 */] },
  "silence":  { "leading_ms": 0.0, "trailing_ms": 812.5, "last_audible_sample": 441000, "floor_dbfs": -60.0 },
  "clipping": { "clipped_samples": 0, "true_peak_over_0dbtp": false }
}
```

- **Loudness**: ebur128 (`I | M | TRUE_PEAK | SAMPLE_PEAK`), stereo default
  channel map. Integrated LUFS, momentary max, true peak dBTP.
- **Onsets**: STFT (rustfft, 1024/256 hop, Hann via libm), half-wave-
  rectified spectral flux, median-based adaptive threshold, peak picking
  with minimum inter-onset gap. Times in ms (sample-derived).
- **Pitch**: YIN (difference function + CMNDF, threshold 0.1, parabolic
  interpolation), 2048 window / 512 hop on mono downmix; per-segment median
  f0. Run per stem for multi-voice material — the *caller* (cli) does that;
  features stays per-buffer.
- **Key**: 12-bin chroma from STFT magnitude (log-frequency weighting),
  Krumhansl-Schmuckler correlation against 24 major/minor templates →
  (tonic, mode, confidence = top correlation vs runner-up margin folded in).
- **Silence/tail**: windowed RMS (50 ms hop) under floor (default −60 dBFS);
  last-audible sample. **Clipping**: |sample| ≥ 1.0 count; true peak > 0
  dBTP flag.
- Score-context enrichment (nearest tick for onsets, cents-vs-score) is NOT
  here — verify/cli join features output against the score.

Tests (fixtures synthesized in-test with libm, no synth dep): 440 Hz sine →
A4 within 1 cent, LUFS of a −18 dBFS sine ≈ −18 LUFS (K-weighting at 1 kHz
≈ 0), click track → onsets at known samples ±2 ms, C major triad → C major,
tail detection on a gated burst, clipping counter on a driven square.

---

## crates/spectro

```rust
pub struct SpectroOpts { fft: usize /*2048*/, hop: usize /*512*/, mels: usize /*128*/,
                         floor_db: f32 /*-80*/, fmin: f32, fmax: f32 }
pub fn mel_spectrogram(audio: &Audio, opts: &SpectroOpts) -> MelImage;    // matrix + axis metadata
pub fn render_png(img: &MelImage, ruler: Ruler, markers: &[Marker]) -> RgbImage;
pub fn contact_sheet(img: &MelImage, markers: &[Marker], per_tile: usize) -> RgbImage;

pub struct Marker { pub sample: u64, pub label: String }   // bar markers WITHOUT score types
```

Hand-rolled mel filterbank (Slaney-style, documented), log magnitude with dB
floor, 256-entry viridis LUT (const table in-repo), time ruler in seconds,
caller-supplied bar markers. Contact sheets tile N-bar sections vertically so
an agent reviews a whole piece in one vision call. `Audio` type is re-used
from features? — **no**: spectro must not depend on features either? (law
only covers score/synth; a features dep would be harmless but keep spectro
standalone: it defines its own tiny input or takes `&[f32], sr` directly.
Decision: functions take `(samples: &[f32], channels, sample_rate)` plain
args; cli/verify adapt.)

Tier 3 sentinels: committed PNGs for fixture signals; image diff with
per-pixel tolerance + max-fraction-differing threshold (helper in this
crate, used by demo tests).

---

## crates/decode

```rust
pub fn load(path: &Path) -> Result<Audio, DecodeError>;   // Audio = cochlea_features::Audio
```

Wave 2: lossless-only real-world file input. Dispatches on file extension:
`.wav`/`.wave` delegates straight to `cochlea_features::Audio::from_wav`
(hound); `.flac` goes through symphonia's bundled FLAC reader+decoder (the
`symphonia` crate, `default-features = false, features = ["flac"]` — no
other format/codec, so no lossy decode sneaks in via a shared feature).
Depends on `cochlea-features` only for the `Audio` type; never score/synth,
same law as `features`/`spectro`.

FLAC is lossless by spec, so a correct decode reproduces the source PCM
exactly — but `symphonia-bundle-flac` left-justifies every sample into the
full 32-bit range regardless of the stream's true bit depth (its own
`decode_inner` comment: "the decoder uses a 32bit sample format as a common
denominator"). Normalizing by always dividing by 2^31 (not by a bit-depth-
derived scale) is what makes FLAC decode land on the same `f32` bits as the
WAV twin — verified, not assumed, by `tests/sample_exact.rs` against tiny
committed FLAC fixtures with WAV twins (`tests/fixtures/`).

mp3/ogg (lossy) are explicitly next, not this round (`docs/superpowers/
specs/2026-07-09-agent-audio-v2-design.md` §2/§6).

---

## crates/verify

```rust
use cochlea_verify::VerifyExt;   // extension trait on Rendered

let report = rendered.verify(&score)
    .integrated_lufs(-14.0, Tol(0.5))
    .true_peak_below(-1.0)
    .onset_at("drums", bar(17).beat(1), Ms(5.0))
    .pitch_matches_score("lead", Cents(10.0))
    .monotone("lead", Param::CUTOFF_HZ, bar(1)..bar(3), Direction::Rising)
    .no_discontinuity("lead", Db(40.0))
    .silent_after(bar(64))
    .run();                       // -> VerifyReport

pub struct VerifyReport { pub passed: bool, pub failures: Vec<Failure>, ... } // serde JSON
```

- Newtypes `Tol(f32)`, `Ms(f32)`, `Cents(f32)`, `Db(f32)`.
- `onset_at`: nearest detected onset on that track's stem within tolerance.
- `pitch_matches_score`: YIN per note window on the stem vs score pitch.
- `monotone`: the *authored automation curve* sampled at block rate over the
  range must be monotone in the given direction (validates the authored
  sweep; spectral verification would conflate instrument response).
- `no_discontinuity`: max sample-to-sample jump (in dB of |Δ|) away from
  note on/off boundaries ± a guard window — click detector.
- `silent_after`: windowed RMS below floor for everything after the tick.
- Every assertion is also a serde data form embeddable in score RON under
  `verify:`; `Verifier::from_specs(&[VerifySpec])` builds the same run. CLI
  `cochlea render score.ron --verify` runs them, writes the JSON failure
  report to stdout (or `--report path`), exits nonzero on failure.

---

## crates/cli (`cochlea` binary)

```
cochlea render score.ron --out mix.wav [--stems dir/] [--verify] [--report report.json]
cochlea probe input.{wav,flac} [--json report.json] [--spectro spec.png]
                               [--digest] [--segments timeline.json] [--window-ms 1000]
cochlea diff a.{wav,flac} b.{wav,flac} [--json compare.json] [--tier2] [--window-ms 1000]
cochlea lint score.ron
cochlea spectro input.{wav,flac} --out spec.png [--sheet --bars-per-tile 8]
```

clap derive; `probe` ships in P3, the rest complete in P4; `probe
--digest`/`--segments` and `diff` land in v2 wave 1 (the token-cheap read
path: digest text instead of JSON, feature-space diff with a
byte-identical / tier2-equivalent / different verdict). `probe` with no
flags prints the JSON report to stdout. Exit codes: 0 ok, 1 verify/lint
failures (and `diff --tier2` when the verdict is not equivalent), 2
usage/IO errors. `--window-ms` rejects non-finite or sub-1 ms values at
the flag boundary (NaN defeats downstream range checks; sub-millisecond
windows would round to one-sample segments and explode the timeline), and
distinct output flags pointing at one path are a usage error, not a
silent last-write-wins.

The `cochlea-mcp` sibling binary (crates/mcp, v2 wave 1) serves the same
pipeline as MCP tools over newline-delimited JSON-RPC 2.0 on stdio:
render_score, probe_wav, spectrogram, lint_score, probe_digest,
audio_diff. Hand-rolled protocol, no async runtime; see `docs/mcp.md`.

---

## Demos (P5, `demos/` as workspace tests or examples)

1. `metronome` — click track; unit test asserts scheduled events land
   sample-exact; probe onsets within 2 ms.
2. `chord_pad` — chord progression on `chord_pad`; chroma/key assertions.
3. `title_cue` — 10 s cinematic sting: filter sweep (automation), volume
   envelope; asserts LUFS target, monotone cutoff, `silent_after`.

Each demo = a `.ron` score + a test running render → verify → probe, plus a
committed spectrogram sentinel.
