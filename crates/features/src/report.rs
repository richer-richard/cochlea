//! The schema-versioned probe report and its request options. JSON field
//! names below are load-bearing — they're this crate's external contract
//! (`docs/plan.md`'s `crates/features` sketch) — and shouldn't be renamed
//! without bumping [`crate::SCHEMA_VERSION`].

use serde::{Deserialize, Serialize};

/// Tunables for [`crate::probe`]. Only the silence floor is exposed today;
/// the STFT/YIN/onset parameters are fixed algorithm constants documented
/// on their extractor modules, not user knobs (`docs/plan.md`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProbeOpts {
    /// RMS level, in dBFS, below which a silence-detector window counts as
    /// silent. Default `-60.0`.
    pub silence_floor_dbfs: f64,
}

impl Default for ProbeOpts {
    fn default() -> Self {
        Self {
            silence_floor_dbfs: -60.0,
        }
    }
}

impl ProbeOpts {
    /// Override the silence floor (dBFS).
    #[must_use]
    pub fn with_silence_floor_dbfs(mut self, floor_dbfs: f64) -> Self {
        self.silence_floor_dbfs = floor_dbfs;
        self
    }
}

/// Top-level probe report, `schema_version: 1`. Mirrors the JSON sketch in
/// `docs/plan.md` (`crates/features`) field-for-field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// Report schema version; see [`crate::SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Input metadata (rate, channels, length).
    pub source: SourceInfo,
    /// EBU R128 loudness/true-peak measurements.
    pub loudness: LoudnessReport,
    /// Spectral-flux onset times.
    pub onsets: OnsetsReport,
    /// YIN pitch track, segmented into voiced runs.
    pub pitch: PitchReport,
    /// Krumhansl-Schmuckler key estimate.
    pub key: KeyReport,
    /// Leading/trailing silence and last audible sample.
    pub silence: SilenceReport,
    /// Sample-clamp clipping count and true-peak-over-0dBTP flag.
    pub clipping: ClippingReport,
}

/// Input metadata.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SourceInfo {
    /// Samples per second, per channel.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Frame count (samples per channel) — not the interleaved sample
    /// count.
    pub samples: usize,
    /// Duration in milliseconds.
    pub duration_ms: f64,
}

/// EBU R128 loudness/true-peak. Fields are `None` where ebur128 reports
/// `-inf`/undefined (silence, or too little audio fed for the mode) — JSON
/// has no `Infinity`, so this crate never emits one
/// (`docs/determinism.md`'s ebur128 audit).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct LoudnessReport {
    /// Integrated (program) loudness, LUFS.
    pub integrated_lufs: Option<f64>,
    /// Running max of 400 ms momentary-loudness readings, LUFS.
    pub momentary_max_lufs: Option<f64>,
    /// True peak (rate-dependent oversampled), dBTP.
    pub true_peak_dbtp: Option<f64>,
    /// Sample peak (no oversampling), dBFS.
    pub sample_peak_dbfs: Option<f64>,
}

/// Onset times from half-wave-rectified spectral flux with an adaptive
/// (rolling-median) threshold. See the `onsets` module for the exact
/// frame-time convention used for `times_ms`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnsetsReport {
    /// `times_ms.len()`, for convenience.
    pub count: usize,
    /// Onset times, milliseconds, ascending.
    pub times_ms: Vec<f64>,
}

/// YIN pitch track over the mono downmix, segmented into contiguous voiced
/// runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchReport {
    /// Fraction of analysis hops YIN judged voiced (found a period under
    /// the absolute CMNDF threshold).
    pub voiced_ratio: f64,
    /// Median f0 across all voiced hops in the whole buffer. `None` if no
    /// hop was voiced.
    pub median_f0_hz: Option<f64>,
    /// Contiguous voiced runs, each with its own median f0.
    pub segments: Vec<PitchSegment>,
}

/// One contiguous voiced run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PitchSegment {
    /// Start time, milliseconds (first hop's frame-start time).
    pub start_ms: f64,
    /// End time, milliseconds (one hop past the last voiced hop's
    /// frame-start time, so consecutive segments tile with no gap).
    pub end_ms: f64,
    /// Median f0 across the run, Hz.
    pub f0_hz: f64,
    /// Nearest equal-tempered MIDI note number to `f0_hz`.
    pub midi_nearest: i32,
    /// Deviation of `f0_hz` from `midi_nearest`'s pitch, in cents.
    pub cents_off: f64,
}

/// Krumhansl-Schmuckler key estimate from STFT-magnitude chroma.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyReport {
    /// Estimated tonic pitch class.
    pub tonic: PitchClass,
    /// Estimated mode.
    pub mode: Mode,
    /// Pearson correlation of the chroma vector against the winning
    /// rotated Krumhansl-Kessler profile. Not a calibrated probability —
    /// higher is more confident, but the scale isn't linear.
    pub confidence: f64,
    /// 12-bin chroma vector (index 0 = C), normalized so its max bin is
    /// `1.0` (all-zero if no in-range spectral energy was found).
    pub chroma: [f64; 12],
}

/// A pitch class, serialized as its note name (`"C"`, `"C#"`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PitchClass {
    /// C
    #[serde(rename = "C")]
    C,
    /// C sharp / D flat
    #[serde(rename = "C#")]
    CSharp,
    /// D
    #[serde(rename = "D")]
    D,
    /// D sharp / E flat
    #[serde(rename = "D#")]
    DSharp,
    /// E
    #[serde(rename = "E")]
    E,
    /// F
    #[serde(rename = "F")]
    F,
    /// F sharp / G flat
    #[serde(rename = "F#")]
    FSharp,
    /// G
    #[serde(rename = "G")]
    G,
    /// G sharp / A flat
    #[serde(rename = "G#")]
    GSharp,
    /// A
    #[serde(rename = "A")]
    A,
    /// A sharp / B flat
    #[serde(rename = "A#")]
    ASharp,
    /// B
    #[serde(rename = "B")]
    B,
}

impl PitchClass {
    /// All twelve pitch classes, index == chroma bin == semitones above C.
    pub(crate) const ALL: [PitchClass; 12] = [
        PitchClass::C,
        PitchClass::CSharp,
        PitchClass::D,
        PitchClass::DSharp,
        PitchClass::E,
        PitchClass::F,
        PitchClass::FSharp,
        PitchClass::G,
        PitchClass::GSharp,
        PitchClass::A,
        PitchClass::ASharp,
        PitchClass::B,
    ];

    /// Note name as printed in text digests/comparisons (`"C"`, `"C#"`,
    /// ...) — the same spelling as this type's serde wire form.
    pub(crate) fn name(self) -> &'static str {
        match self {
            PitchClass::C => "C",
            PitchClass::CSharp => "C#",
            PitchClass::D => "D",
            PitchClass::DSharp => "D#",
            PitchClass::E => "E",
            PitchClass::F => "F",
            PitchClass::FSharp => "F#",
            PitchClass::G => "G",
            PitchClass::GSharp => "G#",
            PitchClass::A => "A",
            PitchClass::ASharp => "A#",
            PitchClass::B => "B",
        }
    }
}

/// Major or minor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Major mode.
    Major,
    /// Minor (natural/Aeolian) mode.
    Minor,
}

/// Windowed-RMS silence/tail detection over the mono downmix.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SilenceReport {
    /// Leading silence, milliseconds, before the first window above
    /// `floor_dbfs`. Equal to the full duration if no window is audible.
    pub leading_ms: f64,
    /// Trailing silence, milliseconds, after the last window above
    /// `floor_dbfs`. Equal to the full duration if no window is audible.
    pub trailing_ms: f64,
    /// Frame index of the last sample covered by the last audible window.
    /// `None` if no window was above the floor.
    pub last_audible_sample: Option<usize>,
    /// The floor used for this report, dBFS (from [`ProbeOpts`]).
    pub floor_dbfs: f64,
}

/// Sample-clamp clipping.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ClippingReport {
    /// Count of interleaved samples with `|x| >= 1.0`.
    pub clipped_samples: usize,
    /// Whether true peak exceeded 0 dBTP.
    pub true_peak_over_0dbtp: bool,
}
