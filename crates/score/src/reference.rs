//! The self-describing authoring reference: everything an agent needs to
//! write a valid cochlea score, generated as Markdown from the live
//! [`Catalog`] so the instrument/param sections can never go stale. This
//! one function feeds three surfaces — `cochlea reference` (CLI), the
//! `score_reference` MCP tool, and the book's Score Format page (kept in
//! sync by a bless-style test in the CLI crate) — so an agent connected to
//! any of them can learn the format in-band instead of needing the repo.

use std::fmt::Write as _;

use crate::validate::Catalog;

/// The full authoring reference as Markdown. Static grammar text plus the
/// instrument/insert catalog rendered from `catalog` (names, polyphony,
/// automatable params with units/ranges/defaults).
pub fn authoring_reference(catalog: &dyn Catalog) -> String {
    let mut out = String::new();
    out.push_str(GRAMMAR);
    write_catalog(&mut out, catalog);
    out.push_str(VERIFY_SPECS);
    out.push_str(EXAMPLE);
    out
}

/// The score grammar: static text, versioned with the RON data form.
const GRAMMAR: &str = r#"# cochlea score reference (RON data form, version 1)

A score is one `Score(...)` value in [RON](https://github.com/ron-rs/ron)
syntax. Render it with `cochlea render score.ron --out mix.wav` (add
`--verify` to run the embedded assertions), or via the `render_score` MCP
tool.

## Top level

```ron
Score(
    version: 1,                      // required, must be 1
    sample_rate: 48000,              // Hz
    ppq: 960,                        // ticks per quarter note (960 is standard)
    time_signature: (4, 4),          // optional, default (4, 4)
    tempo: [(tick: 0, bpm: 120.0)],  // tempo map: step changes at ticks
    tracks: [ Track(...), ... ],
    master: Master(...),             // optional master bus (below)
    verify: [ ... ],                 // optional embedded assertions (below)
)
```

Time is integer ticks at `ppq` per quarter note; positions in the data
form are `(bar, beat)` pairs, both 1-based. Anything off the tick grid is
an error, never a rounding.

## Tracks and notes

```ron
Track(
    name: "lead",                    // unique per score
    instrument: Preset("saw_lead"),  // one of the preset names below
    inserts: [Preset("reverb")],     // optional per-track effect chain
    notes: [
        Note(at: (1, 1), dur: "1/4", pitch: "A4", vel: 96),
        // `off:` shifts the position by a duration past the beat —
        // e.g. the second eighth of beat 1:
        Note(at: (1, 1), off: "1/8", dur: "1/8", pitch: "C5", vel: 80),
    ],
    automation: [ Auto(...), ... ],  // optional (below)
)
```

- `pitch`: note name + octave (`"C4"` = middle C, `"F#3"`, `"Bb2"`).
- `vel`: 1–127 (MIDI-style; velocity maps to amplitude squared).
- `dur`: a fraction of a whole note as a string — `"1/4"` quarter,
  `"3/16"`, dotted `"1/4."`, triplet `"1/8t"`.

## Automation

```ron
Auto(param: "cutoff_hz", keys: [
    Key(at: (1, 1), value: 400.0, ease: EaseInOut),
    Key(at: (3, 1), value: 4000.0),
])
```

- `param` must be automatable on the track's instrument (see the catalog
  below). Engine-level `"gain"` (linear, default 1.0) and `"pan"`
  (-1.0..1.0 constant-power, default 0.0) work on every track; a single
  `Key` sets a constant value.
- `ease` shapes the segment *leaving* that key: `Linear` (default),
  `Hold`, `EaseIn`, `EaseOut`, `EaseInOut`, `Bezier(x1, y1, x2, y2)`.
- Automation is control-rate: sampled every 64 samples (~1.3 ms at
  48 kHz). Note timing is sample-accurate.

## Master bus

```ron
master: Master(
    gain_db: 3.0,                    // optional, default 0.0 (-40..=24)
    limiter: Limiter(
        ceiling_db: -1.0,            // required (-40..=0)
        lookahead_ms: 5.0,           // optional, default 5.0 (0..=50)
        release_ms: 50.0,            // optional, default 50.0 (1..=1000)
    ),
)
```

Applied to the f64 stem sum after mixing: gain first, then a brick-wall
lookahead limiter whose *sample*-peak ceiling holds exactly (inter-sample
true peaks can read fractionally higher — leave ~1 dB of headroom under a
`TruePeakBelow` target). This is the tool for hitting loudness targets:
push with `gain_db`, let the limiter hold the ceiling, and assert both
with `IntegratedLufs` + `TruePeakBelow`. Omit `master:` entirely for a
untouched bus; per-track stems are always exported pre-master.
"#;

fn write_catalog(out: &mut String, catalog: &dyn Catalog) {
    out.push_str("\n## Instrument presets\n\n");
    for name in catalog.instrument_names() {
        let Some(info) = catalog.instrument(&name) else {
            continue;
        };
        let poly = match info.polyphony {
            crate::validate::Polyphony::Mono => "mono".to_string(),
            crate::validate::Polyphony::Poly(n) => format!("poly {n}"),
        };
        writeln!(out, "- `{name}` ({poly})").expect("String write is infallible");
        for p in &info.params {
            writeln!(
                out,
                "  - param `{}` ({}, {}..{}, default {})",
                p.param.as_str(),
                p.unit,
                p.min,
                p.max,
                p.default
            )
            .expect("String write is infallible");
        }
    }
    out.push_str("\nInserts (per-track effects): ");
    let inserts = catalog.insert_names();
    if inserts.is_empty() {
        out.push_str("none.\n");
    } else {
        let names: Vec<String> = inserts.iter().map(|n| format!("`{n}`")).collect();
        writeln!(out, "{}.", names.join(", ")).expect("String write is infallible");
    }
}

/// The verify-spec catalog: static text, versioned with the data form.
const VERIFY_SPECS: &str = r#"
## Embedded assertions (`verify:`)

`cochlea render score.ron --verify` (or `render_score` with
`verify: true`) runs these against the finished render and fails on any
miss. Positions are `(bar, beat)`.

```ron
verify: [
    IntegratedLufs(target: -14.0, tol: 0.5),      // mix loudness, LUFS
    TruePeakBelow(dbtp: -1.0),                     // headroom, dBTP
    OnsetAt(track: "drums", at: (17, 1), tol_ms: 5.0),
    PitchMatchesScore(track: "lead", tol_cents: 10.0),  // monophonic tracks
    Monotone(track: "pad", param: "cutoff_hz",
             from: (1, 1), to: (3, 1), direction: Rising),  // authored curve
    BrightnessRises(track: "pad", from: (1, 1), to: (3, 1),
                    min_ratio: 1.3),               // rendered audio actually brightens
    BrightnessFalls(track: "pad", from: (3, 1), to: (5, 1), min_ratio: 1.3),
    NoDiscontinuity(track: "lead", db: 40.0),      // click detector
    SilentAfter(at: (64, 1)),
    TempoIs(bpm: 110.0, tol_bpm: 2.0),             // optional min_bpm/max_bpm range
    HasClearRhythm(expected: true),                // grid-based rhythm trust flag
    GridAlignmentAtLeast(min: 0.9),                // fraction of onsets on the grid
    StereoWidthWithin(min: 0.03, max: 0.2),
    LraBelow(lu: 8.0),                             // loudness range, LU
    SectionCount(min: 1, max: 3),                  // detected structure sections
]
```

`Monotone` checks the *authored* automation curve (a score-side lint);
`BrightnessRises`/`BrightnessFalls` listen to the rendered stem's
spectral centroid — assert both to prove the sweep was written *and*
audibly happened.
"#;

const EXAMPLE: &str = r#"
## Worked example

```ron
Score(
    version: 1,
    sample_rate: 48000,
    ppq: 960,
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [
        Track(
            name: "kick",
            instrument: Preset("kick"),
            notes: [
                Note(at: (1, 1), dur: "1/8", pitch: "A1", vel: 112),
                Note(at: (1, 2), dur: "1/8", pitch: "A1", vel: 100),
                Note(at: (1, 3), dur: "1/8", pitch: "A1", vel: 112),
                Note(at: (1, 4), dur: "1/8", pitch: "A1", vel: 100),
            ],
        ),
        Track(
            name: "pad",
            instrument: Preset("chord_pad"),
            inserts: [Preset("reverb")],
            notes: [
                Note(at: (1, 1), dur: "1/1", pitch: "A2", vel: 74),
                Note(at: (1, 1), dur: "1/1", pitch: "C3", vel: 70),
                Note(at: (1, 1), dur: "1/1", pitch: "E3", vel: 70),
            ],
            automation: [
                Auto(param: "cutoff_hz", keys: [
                    Key(at: (1, 1), value: 400.0, ease: EaseInOut),
                    Key(at: (2, 1), value: 3000.0),
                ]),
            ],
        ),
    ],
    verify: [
        TruePeakBelow(dbtp: -1.0),
        TempoIs(bpm: 120.0, tol_bpm: 2.0),
        BrightnessRises(track: "pad", from: (1, 1), to: (2, 1), min_ratio: 1.2),
    ],
)
```

Loop: `lint_score` to catch authoring mistakes cheaply, `render_score`
(with `verify: true`), then `probe_audio`/`probe_digest` to read the
result and `spectrogram` to look at it.
"#;
