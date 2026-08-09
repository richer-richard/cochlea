//! Render errors.

use thiserror::Error;

#[derive(Debug, Error)]
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
        "track {name:?} cannot be written as a stem file: a stem name must be a single \
         path component ({reason}). Rename the track, or write the mix without --stems."
    )]
    UnwritableStemName { name: String, reason: &'static str },

    #[error("WAV write failed: {0}")]
    Wav(#[from] hound::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
