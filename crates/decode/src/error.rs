//! Everything that can go wrong turning a file path into
//! [`cochlea_features::Audio`].

use std::path::PathBuf;

use thiserror::Error;

/// Errors from [`crate::load`].
#[derive(Debug, Error)]
pub enum DecodeError {
    /// Couldn't open the file at all.
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// WAV decode failure, forwarded from `cochlea_features::Audio::from_wav`.
    #[error(transparent)]
    Wav(#[from] cochlea_features::AudioError),
    /// FLAC probe/demux/decode failure.
    #[error("decoding FLAC: {0}")]
    Flac(#[from] symphonia::core::errors::Error),
    /// The file probed as FLAC but has no audio track — malformed input;
    /// a well-formed FLAC file always has exactly one.
    #[error("FLAC file has no audio track")]
    NoAudioTrack,
    /// The FLAC stream's STREAMINFO block didn't report a bit depth. Not
    /// needed for sample normalization (see the `flac` module docs), but a
    /// stream missing it is unusual enough to flag rather than ignore.
    #[error("FLAC stream is missing its STREAMINFO bit depth")]
    UnknownBitDepth,
    /// The FLAC stream's STREAMINFO didn't declare a sample rate or
    /// channel count. These are what a zero-packet (truncated or
    /// metadata-only) stream falls back to — without them there's no
    /// defensible `Audio` shape to return at all.
    #[error("FLAC stream is missing its STREAMINFO sample rate or channel count")]
    MissingStreamInfo,
    /// A decoded packet's sample rate or channel count disagreed with the
    /// stream's STREAMINFO declaration — malformed input; FLAC streams
    /// have one fixed spec.
    #[error("FLAC packet spec disagrees with the stream's STREAMINFO")]
    InconsistentStream,
    /// Defensive: the FLAC decoder is documented to always produce 32-bit
    /// integer PCM buffers; this fires only if that internal contract ever
    /// changes out from under us.
    #[error("decoded FLAC buffer wasn't 32-bit integer PCM")]
    UnexpectedSampleFormat,
    /// Neither the extension nor the file's leading magic bytes identify a
    /// format this crate decodes.
    #[error(
        "unrecognized audio file: extension {0:?} and content match neither WAV nor FLAC \
         (expected a \"wav\"/\"flac\" extension, or RIFF/fLaC magic bytes)"
    )]
    UnsupportedExtension(String),
}
