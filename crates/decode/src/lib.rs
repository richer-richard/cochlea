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
//! insensitive) — the two decode paths this crate has, selected the same
//! way `symphonia`'s own examples pick a [`Hint`](symphonia::core::formats::probe::Hint).

mod error;
mod flac;

use std::path::Path;

use cochlea_features::Audio;

pub use error::DecodeError;

/// Loads `path` into [`Audio`]. WAV (`.wav`/`.wave`) goes through
/// `cochlea_features::Audio::from_wav`; FLAC (`.flac`) through this crate's
/// own symphonia-backed decoder. Any other extension (or none) is a clear
/// [`DecodeError::UnsupportedExtension`] rather than a guess at the format.
pub fn load(path: &Path) -> Result<Audio, DecodeError> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);

    match extension.as_deref() {
        Some("wav" | "wave") => Audio::from_wav(path).map_err(DecodeError::Wav),
        Some("flac") => flac::decode(path),
        Some(other) => Err(DecodeError::UnsupportedExtension(other.to_string())),
        None => Err(DecodeError::UnsupportedExtension(String::new())),
    }
}
