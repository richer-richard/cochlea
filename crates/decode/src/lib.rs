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

/// Loads `path` into [`Audio`]. WAV (`.wav`/`.wave`) goes through
/// `cochlea_features::Audio::from_wav`; FLAC (`.flac`) through this
/// crate's bit-exact symphonia decoder; mp3 (`.mp3`) and ogg/vorbis
/// (`.ogg`/`.oga`) through the lossy symphonia decoder (analysis input
/// only — see the crate docs). Any other extension (or none) is decided
/// by the file's magic bytes; only a file that neither names nor *is* a
/// recognized format gets [`DecodeError::UnsupportedExtension`].
pub fn load(path: &Path) -> Result<Audio, DecodeError> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);

    match extension.as_deref() {
        Some("wav" | "wave") => Audio::from_wav(path).map_err(DecodeError::Wav),
        Some("flac") => flac::decode(path),
        Some("mp3") => lossy::decode(path, "mp3"),
        Some("ogg" | "oga") => lossy::decode(path, "ogg"),
        other => match sniff_magic(path)? {
            Some(Magic::Wav) => Audio::from_wav(path).map_err(DecodeError::Wav),
            Some(Magic::Flac) => flac::decode(path),
            Some(Magic::Mp3) => lossy::decode(path, "mp3"),
            Some(Magic::Ogg) => lossy::decode(path, "ogg"),
            None => Err(DecodeError::UnsupportedExtension(
                other.unwrap_or_default().to_string(),
            )),
        },
    }
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
