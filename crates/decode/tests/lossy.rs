//! Lossy-path integration tests: mp3 and ogg/vorbis load, dispatch by
//! extension and by magic bytes, and — the property that actually matters
//! for an analysis tool — the *features* of the decoded audio match the
//! lossless original's, even though the samples can't. Fixtures are the
//! committed WAV tones re-encoded with ffmpeg/lame (see fixtures/).

use std::path::PathBuf;

use cochlea_features::{ProbeOpts, probe};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn mp3_mono_loads_with_the_declared_shape() {
    let audio = cochlea_decode::load(&fixture("tone_mono.mp3")).unwrap();
    assert_eq!(audio.sample_rate, 8_000);
    assert_eq!(audio.channels, 1);
    // Codec delay/padding make lossy durations approximate, never exact —
    // compare against the WAV source it was encoded from, loosely.
    let source = cochlea_decode::load(&fixture("tone_mono_16.wav")).unwrap();
    let dur_s = audio.duration_ms() / 1000.0;
    let source_s = source.duration_ms() / 1000.0;
    assert!(
        (dur_s - source_s).abs() <= 0.3 * source_s + 0.1,
        "duration {dur_s}s vs source {source_s}s"
    );
    assert!(audio.samples.iter().all(|s| s.is_finite()));
    assert!(audio.samples.iter().any(|&s| s.abs() > 0.05), "not silence");
}

#[test]
fn mp3_stereo_and_ogg_load_with_their_channel_counts() {
    let mp3 = cochlea_decode::load(&fixture("tone_stereo.mp3")).unwrap();
    assert_eq!((mp3.sample_rate, mp3.channels), (8_000, 2));
    let ogg = cochlea_decode::load(&fixture("tone_stereo.ogg")).unwrap();
    assert_eq!((ogg.sample_rate, ogg.channels), (8_000, 2));
    assert!(ogg.samples.iter().any(|&s| s.abs() > 0.05), "not silence");
}

/// The point of the lossy path: analysis features survive the codec. The
/// same tone probed from the WAV original and its mp3/ogg encodings must
/// read the same pitch (well under a cent apart in practice — the codecs
/// mangle phase and edges, not a steady tone's frequency).
#[test]
fn features_survive_lossy_encoding() {
    let wav = probe(
        &cochlea_decode::load(&fixture("tone_stereo_16.wav")).unwrap(),
        &ProbeOpts::default(),
    );
    let f0_wav = wav.pitch.median_f0_hz.expect("tone has a pitch");

    for name in ["tone_stereo.mp3", "tone_stereo.ogg"] {
        let lossy = probe(
            &cochlea_decode::load(&fixture(name)).unwrap(),
            &ProbeOpts::default(),
        );
        let f0 = lossy.pitch.median_f0_hz.unwrap_or_else(|| {
            panic!("{name}: no pitch detected");
        });
        let cents = 1200.0 * libm::log2(f0 / f0_wav);
        assert!(
            cents.abs() < 5.0,
            "{name}: pitch drifted {cents:.2} cents ({f0} vs {f0_wav})"
        );
    }
}

/// Extension-less copies still load — dispatch falls back to magic bytes
/// (ID3/MPEG sync for mp3, OggS for ogg).
#[test]
fn magic_byte_sniffing_recognizes_lossy_files() {
    let dir = std::env::temp_dir().join(format!("cochlea-lossy-sniff-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    for (name, bare) in [
        ("tone_mono.mp3", "mp3_no_ext"),
        ("tone_stereo.ogg", "ogg_no_ext"),
    ] {
        let target = dir.join(bare);
        std::fs::copy(fixture(name), &target).unwrap();
        let audio = cochlea_decode::load(&target)
            .unwrap_or_else(|e| panic!("{bare}: sniffing failed: {e}"));
        assert_eq!(audio.sample_rate, 8_000);
    }
    std::fs::remove_dir_all(&dir).ok();
}
