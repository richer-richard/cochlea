//! Error type for the crate's one fallible operation: writing a PNG to disk.

use std::path::PathBuf;

/// Errors from `cochlea-spectro`.
///
/// The analysis and rendering functions ([`mel_spectrogram`](crate::mel_spectrogram),
/// [`render_png`](crate::render_png), [`contact_sheet`](crate::contact_sheet),
/// [`diff_images`](crate::diff_images)) are pure functions of their
/// in-memory arguments and never fail; only [`write_png`](crate::write_png)
/// touches the filesystem and can produce this error.
#[derive(Debug, thiserror::Error)]
pub enum SpectroError {
    /// Encoding or filesystem failure while writing a PNG.
    #[error("failed to write PNG to {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
}
