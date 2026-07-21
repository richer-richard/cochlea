# cochlea-synth

Instruments for [cochlea](https://github.com/richer-richard/cochlea)'s
deterministic offline renderer: eight presets over
[fundsp](https://crates.io/crates/fundsp) (`sine`, `saw_lead`,
`square_bass`, a genuinely stereo `chord_pad`, `noise_hat`, `pluck`,
`kick`, `snare`) plus an in-repo Schroeder reverb insert, a typed
automatable-parameter registry, and a counter-based RNG keyed
`(seed, sample_index)` so all noise is random-access.

Determinism rules in force: `libm`-only transcendentals, voices ticked
sample-by-sample (never fundsp's SIMD block path), no fundsp
`feedback()`/`fdn()` (they set thread-global x86 FTZ), pure-arithmetic
envelope closures. See the workspace's determinism contract:
<https://richer-richard.github.io/cochlea/determinism.html>.

License: MIT OR Apache-2.0.
