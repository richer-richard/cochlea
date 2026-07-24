//! Audio decode into the analysis pipeline's [`Audio`] type: WAV via
//! `cochlea_features::Audio::from_wav` (hound underneath), FLAC/mp3/
//! ogg-vorbis via symphonia (pure Rust — no ffmpeg, no system codecs).
//! Depends on `cochlea-features` only for [`Audio`] itself, never on
//! score/synth, so `cochlea probe some.mp3` stays score-free like the rest
//! of the read path (`docs/plan.md`'s dependency-direction law).
//!
//! Two distinct contracts, one per module:
//!
//! - **Lossless (`flac`)**: FLAC reconstructs the exact original PCM
//!   integers by spec, so decoding a FLAC file and its WAV twin must land
//!   on bit-identical [`Audio::samples`] — see `src/flac.rs` for how
//!   that's guaranteed (not just assumed) and `tests/sample_exact.rs` for
//!   the test enforcing it.
//! - **Lossy (`lossy`)**: mp3 and ogg/vorbis are analysis input, never
//!   render ground truth — same-build decoding is reproducible, but no
//!   exactness claim exists to make. See `src/lossy.rs`.
//!
//! Dispatch is by file extension (case-insensitive) as the fast path,
//! falling back to content sniffing (`RIFF`/`fLaC`/`OggS`/ID3-or-MPEG-sync
//! magic bytes) for anything else — a valid file without its conventional
//! suffix (temp file, pipeline artifact, byte stream dumped to disk) still
//! loads.

mod error;
mod flac;
mod lossy;

use std::io::Read;
use std::path::Path;

use cochlea_features::Audio;

pub use error::DecodeError;

/// Default ceiling on total decoded samples (interleaved across channels)
/// for [`load`]. One billion samples is roughly 4 GB held as `f32`, about
/// 2.9 hours of 48 kHz stereo or a full one-hour render at up to 96 kHz —
/// comfortably beyond any realistic probe target, so it only ever trips on
/// pathological input.
///
/// It exists because the read path (`probe`/`audio_diff`/`spectrogram`) has
/// no natural output bound the way `render` does (which caps at one hour of
/// output). A compressed decompression bomb — a few-KB `.ogg`/`.mp3` that
/// expands to hours of PCM — would otherwise allocate without limit, and even
/// an honest but enormous file would pin the process on the O(window²) YIN
/// pass with no timeout. The compressed decoders check this cap *as they
/// accumulate*, so a bomb is refused near the ceiling rather than after the
/// whole buffer materializes. A caller with a genuinely huge, trusted file
/// uses [`load_with_limit`] to raise or remove it.
pub const DEFAULT_MAX_SAMPLES: u64 = 1_000_000_000;

/// Loads `path` into [`Audio`], refusing input that decodes to more than
/// [`DEFAULT_MAX_SAMPLES`] interleaved samples (see that constant for why).
/// This is the read path's single front door; every `probe`/`diff`/
/// `spectrogram` load goes through it.
pub fn load(path: &Path) -> Result<Audio, DecodeError> {
    load_with_limit(path, Some(DEFAULT_MAX_SAMPLES))
}

/// Like [`load`], but with an explicit ceiling on total decoded samples:
/// `Some(n)` refuses anything past `n` interleaved samples with
/// [`DecodeError::TooLong`]; `None` removes the cap entirely (for a trusted,
/// known-bounded file). [`load`] is exactly `load_with_limit(path,
/// Some(DEFAULT_MAX_SAMPLES))`.
///
/// WAV (`.wav`/`.wave`) goes through `cochlea_features::Audio::from_wav`
/// (its declared length is checked against the cap before any samples are
/// read); FLAC (`.flac`) through this crate's bit-exact symphonia decoder;
/// mp3 (`.mp3`) and ogg/vorbis (`.ogg`/`.oga`) through the lossy symphonia
/// decoder (analysis input only — see the crate docs). Any other extension
/// (or none) is decided by the file's magic bytes; only a file that neither
/// names nor *is* a recognized format gets
/// [`DecodeError::UnsupportedExtension`].
pub fn load_with_limit(path: &Path, limit: Option<u64>) -> Result<Audio, DecodeError> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);

    let audio = match extension.as_deref() {
        Some("wav" | "wave") => load_wav(path, limit),
        Some("flac") => flac::decode(path, limit),
        Some("mp3") => lossy::decode(path, "mp3", limit),
        Some("ogg" | "oga") => lossy::decode(path, "ogg", limit),
        other => match sniff_magic(path)? {
            Some(Magic::Wav) => load_wav(path, limit),
            Some(Magic::Flac) => flac::decode(path, limit),
            Some(Magic::Mp3) => lossy::decode(path, "mp3", limit),
            Some(Magic::Ogg) => lossy::decode(path, "ogg", limit),
            None => Err(DecodeError::UnsupportedExtension(
                other.unwrap_or_default().to_string(),
            )),
        },
    }?;
    check_shape(audio)
}

/// One uniform guard over every decode path's output: no zero channels, no
/// zero sample rate, no ragged interleave. The WAV path is already refused at
/// its header by `from_wav`, but the FLAC and lossy decoders take shape from a
/// stream header a malformed file controls, so this is the single place that
/// guarantees no degenerate [`Audio`] ever leaves this crate to reach an
/// analyzer that assumes a well-formed shape.
fn check_shape(audio: Audio) -> Result<Audio, DecodeError> {
    if audio.channels == 0
        || audio.sample_rate == 0
        || !audio
            .samples
            .len()
            .is_multiple_of(usize::from(audio.channels))
    {
        return Err(DecodeError::DegenerateShape {
            channels: audio.channels,
            sample_rate: audio.sample_rate,
            samples: audio.samples.len(),
        });
    }
    Ok(audio)
}

/// WAV load with the sample cap applied to the header's *declared* length
/// before any samples are read — WAV is uncompressed, so the data-chunk size
/// bounds the true sample count and can't understate it. This refuses an
/// oversized WAV up front instead of after allocating it, then delegates the
/// actual conversion to the one place that owns it
/// (`cochlea_features::Audio::from_wav`).
fn load_wav(path: &Path, limit: Option<u64>) -> Result<Audio, DecodeError> {
    if let Some(limit) = limit {
        let reader = hound::WavReader::open(path).map_err(cochlea_features::AudioError::from)?;
        let declared = u64::from(reader.len());
        if declared > limit {
            return Err(DecodeError::TooLong {
                samples: declared,
                limit,
            });
        }
    }
    Audio::from_wav(path).map_err(DecodeError::Wav)
}

/// A recognized leading magic-byte signature.
enum Magic {
    Wav,
    Flac,
    Mp3,
    Ogg,
}

/// Reads the first four bytes and matches them against the WAV (`RIFF`),
/// FLAC (`fLaC`), Ogg (`OggS`), and mp3 (ID3 tag, or a bare MPEG audio
/// frame sync — 11 set bits) signatures. A file too short to have a
/// signature is simply unrecognized, not an error.
fn sniff_magic(path: &Path) -> Result<Option<Magic>, DecodeError> {
    let mut file = std::fs::File::open(path).map_err(|source| DecodeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut magic = [0u8; 4];
    let mut filled = 0;
    while filled < magic.len() {
        let n = file
            .read(&mut magic[filled..])
            .map_err(|source| DecodeError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if n == 0 {
            return Ok(None);
        }
        filled += n;
    }
    Ok(match &magic {
        b"RIFF" => Some(Magic::Wav),
        b"fLaC" => Some(Magic::Flac),
        b"OggS" => Some(Magic::Ogg),
        [b'I', b'D', b'3', _] => Some(Magic::Mp3),
        // MPEG frame sync: 11 set bits, and a valid (non-reserved) version/
        // layer nibble — loose by design; symphonia does the real parse.
        [0xFF, b1, _, _] if b1 & 0xE0 == 0xE0 => Some(Magic::Mp3),
        _ => None,
    })
}
