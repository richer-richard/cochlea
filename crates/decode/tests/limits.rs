//! The read-path sample cap (adversarial review, Finding 3). The read path
//! has no natural output bound the way render does, so `load` refuses input
//! that decodes past a ceiling — a decompression bomb or a genuinely enormous
//! file — instead of allocating without limit. Each decode path enforces it:
//! WAV from its declared header length (before any samples are read), FLAC and
//! the lossy codecs incrementally as they accumulate.

use std::path::PathBuf;

use cochlea_decode::{DecodeError, load, load_with_limit};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
}

/// A cap below the fixture's true length trips `TooLong` on every decode
/// path — WAV via the header peek, FLAC and lossy via the running check.
#[test]
fn every_decode_path_honors_a_tight_limit() {
    for name in [
        "tone_mono_16.wav",
        "tone_stereo_24.wav",
        "tone_mono_16.flac",
        "tone_stereo_24.flac",
        "tone_mono.mp3",
        "tone_stereo.ogg",
    ] {
        let err = load_with_limit(&fixture(name), Some(1))
            .expect_err(&format!("{name} must exceed a 1-sample cap"));
        assert!(
            matches!(err, DecodeError::TooLong { limit: 1, .. }),
            "{name}: expected TooLong, got {err:?}"
        );
    }
}

/// The default cap is far above these small fixtures, so ordinary `load`
/// succeeds, and an explicit `None` removes the cap entirely.
#[test]
fn ordinary_files_load_under_the_default_and_uncapped() {
    for name in ["tone_mono_16.wav", "tone_stereo_24.flac", "tone_stereo.ogg"] {
        load(&fixture(name)).unwrap_or_else(|err| panic!("{name} under default cap: {err}"));
        load_with_limit(&fixture(name), None)
            .unwrap_or_else(|err| panic!("{name} uncapped: {err}"));
    }
}

/// A cap exactly at the fixture's length admits it; one below refuses it —
/// the boundary is off-by-one clean, not a coarse guess.
#[test]
fn the_limit_boundary_is_exact_for_wav() {
    let audio = load(&fixture("tone_mono_16.wav")).unwrap();
    let exact = audio.samples.len() as u64;

    load_with_limit(&fixture("tone_mono_16.wav"), Some(exact))
        .expect("a cap equal to the true length must admit the file");

    let err = load_with_limit(&fixture("tone_mono_16.wav"), Some(exact - 1))
        .expect_err("one below the true length must be refused");
    assert!(matches!(err, DecodeError::TooLong { limit, samples }
        if limit == exact - 1 && samples == exact));
}
