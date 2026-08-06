# cochlea-features

Feature extraction over PCM for [cochlea](https://github.com/richer-richard/cochlea):
one schema-versioned JSON report (v5) — integrated LUFS / momentary max /
true peak / loudness range (via [ebur128]), spectral-flux onsets, YIN
pitch with cents deviation plus a quantized melody (note events an agent
can diff against the score it wrote), an MFCC timbre digest, chroma +
Krumhansl-Schmuckler key, harmony (a chord timeline plus per-section key),
tempo (pulse clarity, octave-alternative
candidates, windowed stability) and rhythm (beat-grid alignment with a
straight-vs-triplet hypothesis test, offbeat ratio, a calibrated
`clear_rhythm`)
as deliberately separate axes — a drum solo changes its rhythm without
changing its speed, and the report can say so — stereo image
(width/correlation/balance), structural section boundaries, silence/tail,
clipping — plus a windowed segment timeline, a compact plain-text digest
sized for LLM context windows, and a feature-space `compare` API
(`compare`/`compare_text`/`samples_identical`) whose verdicts reuse the
workspace's Tier-2 tolerances: `byte-identical` / `tier2-equivalent` /
`different (dimensions…)`.

Works on **any** WAV — this crate depends on neither the score IR nor
the synth, which is why `cochlea probe` works on audio you didn't
render, no score in sight. (FLAC input goes through [`cochlea-decode`]
first, which produces the same `Audio` type this crate consumes — this
crate itself only reads WAV directly.)

```rust
use cochlea_features::{Audio, ProbeOpts, probe};

let audio = Audio::from_wav(std::path::Path::new("input.wav"))?;
let report = probe(&audio, &ProbeOpts::default());
println!("{:.1} LUFS", report.loudness.integrated_lufs.unwrap_or_default());
```

JSON has no `Infinity`/`NaN`: wherever a measurement is undefined (silence
gives ebur128 a `-inf` LUFS reading, a buffer with no voiced frames has no
pitch), the corresponding field is `null` rather than a non-finite float.

Part of the cochlea workspace — see the [main README] for the full
compose → render → probe → verify workflow.

[ebur128]: https://crates.io/crates/ebur128
[`cochlea-decode`]: https://crates.io/crates/cochlea-decode
[main README]: https://github.com/richer-richard/cochlea#readme

License: MIT OR Apache-2.0, at your option.
