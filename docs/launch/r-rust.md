# r/rust

Draft only — do not post without Richard's explicit go (`checklist.md`).
Post as a text post (not a link post) so the technical framing shows in
the feed; put the repo link in the body.

## Title

```
Cochlea: a headless, deterministic audio engine for AI agents — pure Rust, no unsafe, no ffmpeg
```

## Body

The block below is fenced with four backticks so the Rust snippets
inside it (fenced normally, with three) render correctly in this file —
when copying into Reddit's editor, copy only the inner content, not the
four-backtick delimiter lines themselves.

````
I've been building an audio engine whose only consumer is an AI agent,
not a speaker. No realtime path, no audio device, no GUI — it renders a
declarative score to PCM offline, then exposes the result as a JSON
feature report and small spectrogram PNGs so an agent can "listen"
without an ear. Repo: https://github.com/richer-richard/cochlea

Posting here mainly for the Rust-specific choices, since that's most of
what makes this interesting to build rather than just interesting to
use.

**A war story on trusting a dependency's docs vs. testing against
reality.** I just added FLAC input via `symphonia`. My first pass
normalized decoded samples the "obvious" way: divide the raw integer by
a bit-depth-derived scale (32768 for 16-bit, etc.), mirroring the same
convention the WAV path already uses. Failed my own sample-exact test
immediately, off by roughly 2x. Turned out `symphonia-bundle-flac`
always left-justifies decoded samples into the full 32-bit range
regardless of the stream's true bit depth — straight from its own
source comment: "the decoder uses a 32bit sample format as a common
denominator." The codec params' `bits_per_sample` field is informational
only, not a scaling factor. Fix was to always divide by 2^31
unconditionally, which is provably (and now test-verified across
16-bit mono, 16-bit stereo, and 24-bit stereo fixtures) bit-identical to
the WAV path's per-depth convention — a left-shift by `(32-bits)`
followed by divide-by-`2^31` is the same exact power-of-two rescaling as
divide-by-`2^(bits-1)`. Would've shipped silently-wrong audio if the
test hadn't been sample-exact rather than "close enough."

**Determinism is enforced at compile time, not by convention.** The
workspace bans `f32::sin`/`cos`/`exp`/etc. via `clippy.toml`'s
`disallowed-methods` — DSP code has to go through `libm` instead, because
std's transcendentals aren't guaranteed bit-stable across libc versions
the way libm's scalar implementations are. `mul_add` is banned too (no
accidental FMA). fundsp's SIMD `process()` path is banned outright — I
found it provably diverges from its own scalar `tick()` path, so voices
tick sample-by-sample everywhere, no exceptions. Denormals are honored
rather than flushed (flush-to-zero is a realtime-performance hack that
can't even be done uniformly across x86 and aarch64, and this workspace
renders offline — there's no reason to eat the correctness cost for a
speed win nobody needs). Full audit trail, including the fundsp source
dive that found the SIMD divergence, is in `docs/determinism.md`.

**`cargo-deny` enforces a dependency-direction law, not just license
hygiene.** Nothing in the workspace may ever depend on a GUI, GPU, or
audio-device crate — `cpal`, `winit`, `wgpu`, etc. are hard-denied in
`deny.toml`, checked every CI run. It's a structural guarantee that the
"headless, no ear in the loop" pitch can't quietly rot as the crate graph
grows.

**Integer time, not float seconds.** Ticks are `u64` at 960 PPQ; BPM
converts once to integer nanoseconds-per-quarter at authoring time, and
tick-to-sample conversion is exact rational `u64`/`u128` arithmetic
(`mul_div` with defined rounding), applied once at event-schedule time.
No accumulated floating-point clock anywhere — property-tested drift-
free over 10^9 ticks.

**Workspace shape**: 9 crates — `score` (the IR), `synth` (six presets
over `fundsp`), `render` (the block engine), `features` (LUFS/onsets/
pitch/key via `ebur128` + hand-rolled YIN/chroma), `spectro` (mel
spectrograms, CPU-only), `decode` (lossless WAV/FLAC input via
`symphonia`, the crate the war story above is from), `verify` (an
assertion DSL, RON-embeddable), `cli`, and an MCP stdio server (`mcp` —
hand-rolled JSON-RPC over stdin/stdout, no tokio, because offline batch
tools don't need an async runtime). `features` and `spectro` depend on
neither `score` nor `synth` — checked via `cargo tree` in CI — so
`cochlea probe` works on any WAV or FLAC you hand it, no score in sight.

Small taste of the API:

```rust
let score = Score::new(SampleRate(48_000), Ppq(960))
    .track("lead", Instrument::preset("saw_lead"))
    .note("lead", bar(1).beat(1), Dur::quarter(), Pitch::A4, Vel(96));

let rendered = cochlea_render::render(&score)?;
let report = rendered.verify(&score)
    .true_peak_below(-1.0)
    .pitch_matches_score("lead", Cents(10.0))
    .run();
assert!(report.passed);
```

And the CLI side, probing a real FLAC file end to end:

```
$ cochlea probe drum_loop.flac --digest
cochlea digest: 0.500s  1ch  8000Hz
loudness: integrated=-9.51  momentary_max=-9.51  true_peak=-6.02
key: C major (conf 0.00)  pitch: voiced=100%  median=440.6Hz (A4 +2.5c)
onsets: count=0  rate=0.00/s
...
```

Would genuinely like pushback on the determinism approach in particular
— I'm fairly confident in the byte-identical claim on the pinned CI
target, less confident I've found every edge case (the FLAC
left-justification thing above being exhibit A for "less confident than
I was yesterday"). Also open to "this crate boundary is wrong" feedback
on the workspace split.
````

Notes for whoever posts this:

- The FLAC CLI output block is real (`cochlea probe` against one of the
  crate's own test fixtures, run 2026-07-09) — re-run before posting if
  it's been a while, both to confirm it's still accurate and because a
  slightly different real number is more credible than a stale one.
- r/rust in particular will push on "banned via clippy config" claims —
  make sure `clippy.toml` and the CI workflow are both actually public
  and linkable by the time this posts, since someone will go check.
- The war story is the strongest hook for this specific audience —
  Rust programmers love "I trusted the types, tested anyway, and the
  test caught something real" stories more than feature lists. Don't
  cut it for length if this needs trimming; cut something else first.
