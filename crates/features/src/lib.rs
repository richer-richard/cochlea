//! Feature extraction over PCM: loudness (ebur128), onsets (spectral flux),
//! pitch (YIN), chroma/key (Krumhansl-Schmuckler), silence/tail, clipping.
//! One schema-versioned JSON report. Works on arbitrary WAVs — this crate
//! depends on neither the score IR nor the synth.
//!
//! JSON has no representation for `Infinity`/`NaN`. Wherever a measurement
//! is undefined — silence gives ebur128 a `-inf` LUFS reading, a buffer
//! with no voiced frames has no pitch, a chroma vector with no in-range
//! spectral energy has no clear key candidate — the corresponding
//! [`Report`] field is `Option<f64>`/`null` rather than a non-finite float.
//! See `docs/plan.md` (`crates/features`) for the canonical schema sketch.

mod audio;
mod clipping;
mod compare;
mod digest;
mod key;
mod loudness;
mod onsets;
mod pitch;
mod report;
mod segments;
mod silence;
mod stereo;
mod stft;
mod tempo;

pub use audio::{Audio, AudioError};
pub use compare::{
    Analysis, COMPARE_SCHEMA_VERSION, CompareReport, KeyDelta, KeySummary, LoudnessDelta,
    OnsetMatch, PitchDelta, SegmentDelta, Verdict, compare, compare_text, compare_with_identity,
    samples_identical,
};
pub use digest::digest_text;
pub use loudness::loudness_range;
pub use report::{
    ClippingReport, KeyReport, LoudnessReport, Mode, OnsetsReport, PitchClass, PitchReport,
    PitchSegment, ProbeOpts, Report, SilenceReport, SourceInfo,
};
pub use segments::{
    BandEnergy, SEGMENTS_SCHEMA_VERSION, Segment, SegmentOpts, SegmentTimeline, segment_timeline,
};
pub use stereo::{StereoReport, analyze_stereo};
pub use tempo::{TempoOpts, TempoReport, estimate_tempo};

/// Schema version of [`Report`]'s JSON form. Bump and document here on any
/// breaking change to the report shape.
pub const SCHEMA_VERSION: u32 = 1;

/// Run every extractor over `audio` and assemble the schema-versioned
/// report. Infallible: undefined measurements (silence, no voiced frames,
/// no tonal energy, a malformed `EbuR128` construction) surface as
/// `None`/empty fields rather than errors or panics.
pub fn probe(audio: &Audio, opts: &ProbeOpts) -> Report {
    let mono = audio.mono();

    let loudness = loudness::analyze(audio);
    let onsets = onsets::analyze(&mono, audio.sample_rate);
    let pitch = pitch::analyze(&mono, audio.sample_rate);
    let key = key::analyze(&mono, audio.sample_rate);
    let silence = silence::analyze(&mono, audio.sample_rate, opts.silence_floor_dbfs);
    let clipping = clipping::analyze(audio, loudness.true_peak_dbtp);

    Report {
        schema_version: SCHEMA_VERSION,
        source: SourceInfo {
            sample_rate: audio.sample_rate,
            channels: audio.channels,
            samples: audio.frames(),
            duration_ms: audio.duration_ms(),
        },
        loudness,
        onsets,
        pitch,
        key,
        silence,
        clipping,
    }
}
