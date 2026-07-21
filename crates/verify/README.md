# cochlea-verify

The assertion layer of
[cochlea](https://github.com/richer-richard/cochlea): a chainable DSL
over a finished render — loudness targets, true-peak headroom, onset
timing, per-note pitch, authored-curve monotonicity *and* rendered
brightness (did the sweep audibly happen?), click detection, silence,
tempo, grid-based rhythm clarity, stereo width, loudness range, section
count — with JSON reports and a RON data form embeddable in a score's
`verify:` block, so `cochlea render score.ron --verify` exits nonzero on
any miss and an agent can retry without a human ear.

```rust
use cochlea_verify::{VerifyExt, Tol, Cents};
let report = rendered.verify(&score)
    .integrated_lufs(-14.0, Tol(0.5))
    .true_peak_below(-1.0)
    .pitch_matches_score("lead", Cents(10.0))
    .run();
assert!(report.passed);
```

Docs: <https://richer-richard.github.io/cochlea/>.

License: MIT OR Apache-2.0.
