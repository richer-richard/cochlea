//! Render errors.

use thiserror::Error;

/// Marked `#[non_exhaustive]`: this enum grows whenever the engine learns a
/// new way to refuse work, and a downstream `match` should not break each
/// time. Adding that attribute (and the variants below) is itself a
/// breaking change — see the CHANGELOG entry for the release that made it.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RenderError {
    #[error("unknown instrument {name:?} on track {track:?} (available presets: {available})")]
    UnknownInstrument {
        track: String,
        name: String,
        available: String,
    },

    #[error("unknown insert {name:?} on track {track:?}")]
    UnknownInsert { track: String, name: String },

    #[error(
        "render length {samples} samples exceeds the v1 cap of one hour at {sample_rate} Hz — \
         split the score"
    )]
    TooLong { samples: u64, sample_rate: u32 },

    #[error(
        "track {name:?} cannot be written as a stem file: a stem name must be a portable \
         file name ({reason}). Rename the track, or write the mix without --stems."
    )]
    UnwritableStemName { name: String, reason: &'static str },

    #[error(
        "tracks {first:?} and {second:?} differ only by case, so their stems are one file \
         on macOS and Windows and one would be lost. Rename one of them."
    )]
    CollidingStemNames { first: String, second: String },

    #[error("WAV write failed: {0}")]
    Wav(#[from] hound::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
