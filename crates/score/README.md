# cochlea-score

The score IR for [cochlea](https://github.com/richer-richard/cochlea), a
headless deterministic audio engine for AI agents: integer ticks at 960
PPQ, a tempo map of step changes, bar/beat math, tracks, notes,
per-parameter automation, embeddable verify assertions, and a RON data
form (round-trip tested both ways). Anything off the tick grid is an
error, never a rounding.

```rust
use cochlea_score::*;

let score = Score::new(SampleRate(48_000), Ppq(960))
    .tempo(Ticks(0), Bpm(120.0))
    .track("lead", Instrument::preset("saw_lead"))
    .note("lead", bar(1).beat(1), Dur::quarter(), Pitch::A4, Vel(96));
```

`authoring_reference()` generates the full score-format reference
(grammar, catalog, assertions) from a live instrument catalog — the same
text the `cochlea` CLI and MCP server serve to agents.

Render with [`cochlea-render`](https://crates.io/crates/cochlea-render);
docs at <https://richer-richard.github.io/cochlea/>.

License: MIT OR Apache-2.0.
