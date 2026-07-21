//! Verification assertions as pure data, resolved to ticks. The `verify`
//! crate interprets these against a render; they live here so the RON data
//! form can embed them without the score crate depending on verify.

use crate::param::Param;
use crate::time::Ticks;

/// Direction for [`VerifySpec::Monotone`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MonotoneDir {
    Rising,
    Falling,
}

/// One embeddable assertion. Positions are resolved ticks; the RON form
/// authors them as `(bar, beat)` and resolution happens at load.
#[derive(Debug, Clone, PartialEq)]
pub enum VerifySpec {
    /// Integrated loudness of the mix within `tol` LU of `target` LUFS.
    IntegratedLufs { target: f64, tol: f64 },
    /// True peak of the mix at or below `dbtp` dBTP.
    TruePeakBelow { dbtp: f64 },
    /// A detected onset on `track`'s stem within `tol_ms` of `at`.
    OnsetAt {
        track: String,
        at: Ticks,
        tol_ms: f64,
    },
    /// Every note on `track` reads (YIN, per note window) within
    /// `tol_cents` of its scored pitch.
    PitchMatchesScore { track: String, tol_cents: f64 },
    /// The authored automation curve for `param` on `track` is monotone
    /// over `from..to` in the given direction, sampled at block rate.
    Monotone {
        track: String,
        param: Param,
        from: Ticks,
        to: Ticks,
        direction: MonotoneDir,
    },
    /// No sample-to-sample jump louder than `db` (of |Δ|, dBFS) on
    /// `track`'s stem away from note boundaries — the click detector.
    NoDiscontinuity { track: String, db: f64 },
    /// Windowed RMS stays below the silence floor from `at` to the end of
    /// the render.
    SilentAfter { at: Ticks },
    /// The mix's estimated tempo (`cochlea_features::estimate_tempo`) is
    /// within `tol_bpm` of `bpm`. `min_bpm`/`max_bpm` optionally override
    /// the detector's search range (default 30–300 BPM) — the escape hatch
    /// for material above ~170 BPM, where the octave-error prior otherwise
    /// favors the half-time subharmonic.
    TempoIs {
        bpm: f64,
        tol_bpm: f64,
        min_bpm: Option<f64>,
        max_bpm: Option<f64>,
    },
    /// The mix's rhythm trustworthiness
    /// (`cochlea_features::RhythmReport::clear_rhythm` — the grid-based
    /// rule) equals `expected` — asserts a clear, articulated rhythm when
    /// `true`, or asserts the *absence* of one when `false`.
    HasClearRhythm { expected: bool },
    /// The mix's rhythm grid alignment
    /// (`cochlea_features::RhythmReport::grid_alignment`, the fraction of
    /// detected onsets on the beat-subdivision grid, `0.0..=1.0`) is at
    /// least `min`.
    GridAlignmentAtLeast { min: f64 },
    /// `track`'s spectral centroid (brightness) audibly rises across
    /// `from..to` on the *rendered stem*: the median centroid over the
    /// range's last quarter must be at least `min_ratio` times the median
    /// over its first quarter. The output-side companion to
    /// [`VerifySpec::Monotone`], which validates only the authored curve.
    BrightnessRises {
        track: String,
        from: Ticks,
        to: Ticks,
        min_ratio: f64,
    },
    /// [`VerifySpec::BrightnessRises`] mirrored: the first quarter's
    /// median centroid must be at least `min_ratio` times the last's.
    BrightnessFalls {
        track: String,
        from: Ticks,
        to: Ticks,
        min_ratio: f64,
    },
    /// The mix's stereo width (`cochlea_features::StereoReport::width`,
    /// `0.0..=1.0`) falls within `[min, max]`.
    StereoWidthWithin { min: f64, max: f64 },
    /// The mix's EBU R128 loudness range (LRA) is at or below `lu` LU.
    LraBelow { lu: f64 },
    /// The mix's detected structural section count
    /// (`cochlea_features::StructureReport::section_count`) falls within
    /// `[min, max]`.
    SectionCount { min: usize, max: usize },
}
