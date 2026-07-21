# cochlea score reference (RON data form, version 1)

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

## Instrument presets

- `chord_pad` (poly 16)
  - param `cutoff_hz` (Hz, 40..12000, default 1200)
  - param `gain` (linear, 0..4, default 1)
  - param `pan` (pan, -1..1, default 0)
- `kick` (mono)
  - param `gain` (linear, 0..4, default 1)
  - param `pan` (pan, -1..1, default 0)
- `noise_hat` (poly 8)
  - param `gain` (linear, 0..4, default 1)
  - param `pan` (pan, -1..1, default 0)
- `pluck` (poly 16)
  - param `gain` (linear, 0..4, default 1)
  - param `pan` (pan, -1..1, default 0)
- `saw_lead` (poly 8)
  - param `cutoff_hz` (Hz, 40..18000, default 2400)
  - param `gain` (linear, 0..4, default 1)
  - param `pan` (pan, -1..1, default 0)
- `sine` (poly 16)
  - param `gain` (linear, 0..4, default 1)
  - param `pan` (pan, -1..1, default 0)
- `snare` (poly 4)
  - param `gain` (linear, 0..4, default 1)
  - param `pan` (pan, -1..1, default 0)
- `square_bass` (mono)
  - param `gain` (linear, 0..4, default 1)
  - param `pan` (pan, -1..1, default 0)

Inserts (per-track effects): `reverb`.

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
