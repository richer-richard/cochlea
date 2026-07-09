//! Lossless decode into the analysis pipeline's [`Audio`] type: WAV via
//! `cochlea_features::Audio::from_wav` (hound underneath), FLAC via
//! symphonia (pure Rust, MPL-2.0 — no ffmpeg, no system codecs). Depends on
//! `cochlea-features` only for [`Audio`] itself, never on score/synth, so
//! `cochlea probe some.flac` stays score-free like the rest of the read
//! path (`docs/plan.md`'s dependency-direction law).
//!
//! FLAC is lossless: a correct decoder reconstructs the exact original PCM
//! integers by spec, so decoding a FLAC file and its WAV twin must land on
//! bit-identical [`Audio::samples`] — see the crate-private `flac` module's
//! docs (`src/flac.rs`) for how that's actually guaranteed (not just
//! assumed), and `tests/sample_exact.rs` for the test that enforces it.
//!
//! Dispatch is by file extension (`.wav`/`.wave` vs `.flac`, case-
//! insensitive) as the fast path, falling back to content sniffing (the
//! `RIFF`/`fLaC` magic bytes) for anything else — a valid WAV without a
//! `.wav` suffix (temp file, pipeline artifact, byte stream dumped to
//! disk) loads the way it did when the CLI read WAVs by content alone.

mod error;
mod flac;

use std::io::Read;
use std::path::Path;

use cochlea_features::Audio;

pub use error::DecodeError;

/// Loads `path` into [`Audio`]. WAV (`.wav`/`.wave`) goes through
/// `cochlea_features::Audio::from_wav`; FLAC (`.flac`) through this crate's
/// own symphonia-backed decoder. Any other extension (or none) is decided
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
        other => match sniff_magic(path)? {
            Some(Magic::Wav) => Audio::from_wav(path).map_err(DecodeError::Wav),
            Some(Magic::Flac) => flac::decode(path),
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
}

/// Reads the first four bytes and matches them against the WAV (`RIFF`)
/// and FLAC (`fLaC`) signatures. A file too short to have a signature is
/// simply unrecognized, not an error.
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
        _ => None,
    })
}
