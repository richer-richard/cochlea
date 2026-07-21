//! Score authoring and parsing errors.

use thiserror::Error;

/// Everything that can go wrong building, resolving, or parsing a score.
///
/// The chainable builder methods panic on these (authoring errors are bugs
/// at the call site, matching fenestra's builder conventions); the `try_`
/// variants and the RON loader return them.
#[derive(Debug, Error)]
pub enum ScoreError {
    #[error("{what} {value} out of range {min}..={max}")]
    OutOfRange {
        what: &'static str,
        value: f64,
        min: f64,
        max: f64,
    },

    #[error(
        "duration {num}/{den} does not resolve to a whole tick at {ppq} PPQ \
         (ticks = {ppq} * 4 * {num} / {den} must be exact)"
    )]
    NonIntegerTick { num: u32, den: u32, ppq: u32 },

    #[error("time signature unit {unit} must divide {ppq} PPQ * 4 exactly")]
    BadTimeSignatureUnit { unit: u32, ppq: u32 },

    #[error("bar and beat are 1-based; got bar {bar}, beat {beat}")]
    ZeroBasedPosition { bar: u32, beat: u32 },

    #[error("beat {beat} exceeds the {beats}/{unit} time signature")]
    BeatOutOfSignature { beat: u32, beats: u32, unit: u32 },

    #[error("unknown track {0:?}")]
    UnknownTrack(String),

    #[error("duplicate track {0:?}")]
    DuplicateTrack(String),

    #[error("unparseable pitch {0:?} (expected e.g. \"A4\", \"C#3\", \"Bb2\")")]
    BadPitch(String),

    #[error("MIDI pitch {0} out of range 0..=127")]
    PitchOutOfRange(i32),

    #[error("unparseable duration {0:?} (expected e.g. \"1/4\", \"3/16\", \"1/8.\", \"1/4t\")")]
    BadDur(String),

    #[error("velocity 0 is not a note (MIDI reserves it for note-off); use 1..=127")]
    ZeroVelocity,

    #[error("a note needs a positive duration")]
    ZeroDuration,

    #[error("automation needs at least one key")]
    EmptyKeys,

    #[error("two automation keys share tick {tick} — a track is a function of tick")]
    DuplicateKeyTick { tick: u64 },

    #[error("unsupported score version {0} (this build reads version 1)")]
    UnsupportedVersion(u32),

    #[error("MIDI import: {0}")]
    Midi(String),

    #[error("RON parse error: {0}")]
    Parse(#[from] ron::error::SpannedError),

    #[error("RON serialize error: {0}")]
    Serialize(#[from] ron::Error),
}
