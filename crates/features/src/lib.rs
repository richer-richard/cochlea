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
mod structure;
mod tempo;
mod util;

pub use audio::{Audio, AudioError};
pub use compare::{
    Analysis, COMPARE_SCHEMA_VERSION, CompareReport, KeyDelta, KeySummary, LoudnessDelta,
    OnsetMatch, PitchDelta, SegmentDelta, StereoDelta, StructureDelta, TempoDelta, Verdict,
    compare, compare_text, compare_with_identity, samples_identical,
};
pub use digest::digest_text;
pub use loudness::loudness_range;
pub use report::{
    ClippingReport, KeyReport, LoudnessReport, Mode, OnsetsReport, PitchClass, PitchReport,
    PitchSegment, ProbeOpts, Report, SilenceReport, SourceInfo, TempoSummary,
};
pub use segments::{
    BandEnergy, SEGMENTS_SCHEMA_VERSION, Segment, SegmentOpts, SegmentTimeline, segment_timeline,
    validate_window_ms,
};
pub use stereo::{StereoReport, analyze_stereo};
pub use structure::{StructureOpts, StructureReport, detect_structure};
pub use tempo::{TempoOpts, TempoReport, estimate_tempo};

/// Schema version of [`Report`]'s JSON form. Bump and document here on any
/// breaking change to the report shape.
///
/// - `1`: initial shape (`docs/plan.md`'s sketch) — loudness (integrated /
///   momentary max / true peak / sample peak), onsets, pitch, key,
///   silence, clipping.
/// - `2`: added `loudness.lra`, `tempo`, `stereo`, `structure` (Wave 2's
///   tempo/beat, stereo-image, and structure-detection analyzers).
pub const SCHEMA_VERSION: u32 = 2;

/// Run every extractor over `audio` and assemble the schema-versioned
/// report. Infallible: undefined measurements (silence, no voiced frames,
/// no tonal energy, a malformed `EbuR128` construction) surface as
/// `None`/empty fields rather than errors or panics.
pub fn probe(audio: &Audio, opts: &ProbeOpts) -> Report {
    let mono = audio.mono();

    // One onsets-grade STFT feeds both the onset detector and the tempo
    // estimator (which also reuses the onset report for its rate floor) —
    // previously each consumer recomputed an identical 1024/256 transform,
    // tripling the heaviest per-probe work for byte-identical results.
    let onset_stft = stft::Stft::compute(&mono, audio.sample_rate, onsets::FFT_SIZE, onsets::HOP);

    let loudness = loudness::analyze(audio);
    let onsets = onsets::analyze_stft(&onset_stft);
    let pitch = pitch::analyze(&mono, audio.sample_rate);
    let key = key::analyze(&mono, audio.sample_rate);
    let silence = silence::analyze(&mono, audio.sample_rate, opts.silence_floor_dbfs);
    let clipping = clipping::analyze(audio, loudness.true_peak_dbtp);
    let duration_s = mono.len() as f64 / f64::from(audio.sample_rate.max(1));
    let tempo = summarize_tempo(tempo::estimate_from_parts(
        &onset_stft,
        &onsets,
        duration_s,
        audio.sample_rate,
        &TempoOpts::default(),
    ));
    let stereo = stereo::analyze_stereo(audio);
    let structure = structure::detect_structure(audio, &StructureOpts::default());

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
        tempo,
        stereo,
        structure,
    }
}

/// Reduce a full [`TempoReport`] to the compact [`TempoSummary`] embedded
/// in [`Report`] — drops `beats_ms`, keeping only its length and mean
/// interval (see [`TempoSummary`]'s docs on why).
fn summarize_tempo(full: TempoReport) -> TempoSummary {
    let beat_count = full.beats_ms.len();
    let mean_beat_interval_ms = (beat_count >= 2).then(|| {
        let span = full.beats_ms[beat_count - 1] - full.beats_ms[0];
        span / (beat_count - 1) as f64
    });

    TempoSummary {
        bpm: full.bpm,
        confidence: full.confidence,
        clear_rhythm: full.clear_rhythm,
        beat_count,
        mean_beat_interval_ms,
    }
}
