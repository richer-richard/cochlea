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
    /// Symphonia probe/demux/decode failure (FLAC, mp3, or ogg/vorbis).
    #[error("decoding: {0}")]
    Flac(#[from] symphonia::core::errors::Error),
    /// The file probed as a recognized container but has no audio track —
    /// malformed input.
    #[error("audio file has no audio track")]
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
    /// integer PCM buffers (and the lossy decoders one of the common
    /// integer/float layouts); this fires only if that contract ever
    /// changes out from under us.
    #[error("decoded buffer sample format isn't one this crate converts")]
    UnexpectedSampleFormat,
    /// A lossy decode produced a NaN or infinite sample — not audio, and
    /// poison for downstream analyzers (mirrors the WAV path's rejection
    /// in `cochlea_features::AudioError::NonFiniteSample`).
    #[error("non-finite decoded sample (NaN or infinity) at sample index {index}")]
    NonFiniteSample { index: usize },
    /// Neither the extension nor the file's leading magic bytes identify a
    /// format this crate decodes.
    #[error(
        "unrecognized audio file: extension {0:?} and content match none of WAV, FLAC, mp3, \
         or ogg (expected a wav/flac/mp3/ogg extension, or their magic bytes)"
    )]
    UnsupportedExtension(String),
    /// The file decodes to more samples than the read-path cap allows. The
    /// read path (probe/diff/spectrogram) has no natural output bound the way
    /// render does, so without a cap a few-KB compressed file that expands to
    /// hours of PCM (a "decompression bomb"), or a genuinely enormous file,
    /// would drive unbounded allocation and a very long O(window²) pitch pass.
    /// For a legitimately huge file, raise the ceiling with
    /// [`crate::load_with_limit`]. `samples` is the interleaved count reached
    /// when the cap tripped (for a compressed source it may be the point the
    /// running decode crossed the line, not the file's true length).
    #[error(
        "audio is too long to analyze: {samples} samples exceeds the {limit}-sample cap \
         (use load_with_limit to raise it for a genuinely large file)"
    )]
    TooLong { samples: u64, limit: u64 },
    /// The decoded audio has a degenerate shape — zero channels, zero sample
    /// rate, or an interleaved length not divisible by the channel count. A
    /// correct decoder shouldn't emit this, but a malformed header could;
    /// admitting it risks divide-by-zero and assertion panics in downstream
    /// analyzers and the mel spectrogram, so it's refused at the door.
    #[error(
        "degenerate audio shape: {channels} channels, {sample_rate} Hz, {samples} interleaved \
         samples"
    )]
    DegenerateShape {
        channels: u16,
        sample_rate: u32,
        samples: usize,
    },
}
