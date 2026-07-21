//! Error type for the crate's fallible operations: PNG encoding, to disk
//! or to memory.

use std::path::PathBuf;

/// Errors from `cochlea-spectro`.
///
/// The analysis and rendering functions ([`mel_spectrogram`](crate::mel_spectrogram),
/// [`render_png`](crate::render_png), [`contact_sheet`](crate::contact_sheet),
/// [`diff_images`](crate::diff_images)) are pure functions of their
/// in-memory arguments and never fail; [`write_png`](crate::write_png) and
/// [`encode_png`](crate::encode_png) run the PNG encoder, and
/// [`render_diff_png`](crate::render_diff_png) checks its two inputs share
/// an analysis axis — those are the fallible operations.
#[derive(Debug, thiserror::Error)]
pub enum SpectroError {
    /// Encoding or filesystem failure while writing a PNG to disk.
    #[error("failed to write PNG to {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    /// PNG encoding failure while encoding to memory.
    #[error("failed to encode PNG: {source}")]
    Encode {
        #[source]
        source: image::ImageError,
    },
    /// The two spectrograms handed to
    /// [`render_diff_png`](crate::render_diff_png) were computed with
    /// different analysis parameters (band count, hop, sample rate, or
    /// frequency range) — a per-band subtraction between different axes
    /// would be meaningless, so it's refused rather than guessed at.
    #[error(
        "spectrogram axes differ (band count, hop, sample rate, or frequency range) — \
         compute both sides with the same options and sample rate"
    )]
    AxisMismatch,
}
