//! Integration tests for the schema-v4 hearing additions: melody note
//! events (the compose loop's read-back half), the MFCC timbre digest, and
//! the `Audio::window` zoom lens.

use cochlea_features::{ProbeOpts, probe};

mod common;
use common::*;

/// Three half-second tones back to back — the smallest melody worth
/// reading back as notes.
fn three_note_line() -> Vec<f32> {
    let mut samples = Vec::new();
    for freq in [440.0, 523.251, 659.255] {
        samples.extend(sine_wave(freq, 0.5, 0.5, SR));
    }
    samples
}

/// A naive sawtooth — aliasing is irrelevant here, it just needs to be a
/// spectrally rich signal at the same pitch/level as the sine fixture.
fn saw_wave(freq_hz: f64, amplitude: f64, seconds: f64, sample_rate: u32) -> Vec<f32> {
    let n = (seconds * f64::from(sample_rate)).round() as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(sample_rate);
            let phase = (t * freq_hz).fract();
            (amplitude * (2.0 * phase - 1.0)) as f32
        })
        .collect()
}

#[test]
fn melody_reads_back_the_notes_that_were_played() {
    let report = probe(&mono_audio(three_note_line(), SR), &ProbeOpts::default());
    let names: Vec<&str> = report
        .pitch
        .melody
        .iter()
        .map(|n| n.name.as_str())
        .collect();
    assert_eq!(names, ["A4", "C5", "E5"], "{:?}", report.pitch.melody);

    // Note timing tracks the authored 0.5 s boundaries to within a couple
    // of analysis windows (~43 ms each at 48 kHz).
    let starts: Vec<f64> = report.pitch.melody.iter().map(|n| n.start_ms).collect();
    for (start, expected) in starts.iter().zip([0.0, 500.0, 1000.0]) {
        assert!(
            (start - expected).abs() <= 60.0,
            "starts {starts:?} vs authored 0/500/1000 ms"
        );
    }
    // Pitch centers are dead-on for clean sines.
    for note in &report.pitch.melody {
        assert!(
            note.cents_off.abs() <= 5.0,
            "clean sine should sit on the note center: {note:?}"
        );
    }
}

#[test]
fn melody_is_empty_for_unpitched_input() {
    let report = probe(
        &mono_audio(click_track(&[0.5, 1.0, 1.5], 2.5, SR), SR),
        &ProbeOpts::default(),
    );
    // Clicks are transients, not held pitches — no melody note survives
    // the minimum-duration floor.
    assert!(report.pitch.melody.is_empty(), "{:?}", report.pitch.melody);
    let silent = probe(&mono_audio(silence(2.0, SR), SR), &ProbeOpts::default());
    assert!(silent.pitch.melody.is_empty());
}

#[test]
fn timbre_separates_waveforms_and_matches_itself() {
    let sine = probe(
        &mono_audio(sine_wave(440.0, 0.5, 2.0, SR), SR),
        &ProbeOpts::default(),
    );
    let sine_again = probe(
        &mono_audio(sine_wave(440.0, 0.5, 2.0, SR), SR),
        &ProbeOpts::default(),
    );
    let saw = probe(
        &mono_audio(saw_wave(440.0, 0.5, 2.0, SR), SR),
        &ProbeOpts::default(),
    );

    let dist = |a: &cochlea_features::Report, b: &cochlea_features::Report| {
        let (ta, tb) = (a.timbre.as_ref().unwrap(), b.timbre.as_ref().unwrap());
        let sum: f64 = ta
            .mfcc_mean
            .iter()
            .zip(tb.mfcc_mean.iter())
            .skip(1)
            .map(|(x, y)| (x - y) * (x - y))
            .sum();
        libm::sqrt(sum)
    };

    assert_eq!(
        dist(&sine, &sine_again),
        0.0,
        "identical input must produce identical MFCCs"
    );
    let separation = dist(&sine, &saw);
    assert!(
        separation > 1.0,
        "same note, same level, different waveform must separate in MFCC space: {separation}"
    );
    // A static tone's timbre barely moves frame to frame.
    let stds = &sine.timbre.as_ref().unwrap().mfcc_std;
    assert!(
        stds.iter().skip(1).all(|&s| s < 1.0),
        "static tone should have near-constant MFCCs: {stds:?}"
    );
}

#[test]
fn timbre_is_absent_for_too_short_input() {
    let report = probe(&mono_audio(silence(0.01, SR), SR), &ProbeOpts::default());
    assert!(report.timbre.is_none());
}

#[test]
fn window_cuts_frame_exact_slices_and_reports_start() {
    let audio = mono_audio(three_note_line(), SR);
    let (cut, start_ms) = audio.window(0.5, Some(1.0));
    assert_eq!(start_ms, 500.0);
    assert_eq!(cut.frames(), SR as usize / 2);
    // The cut really is the middle note: probing it alone reads C5.
    let report = probe(&cut, &ProbeOpts::default().with_start_ms(start_ms));
    assert_eq!(report.source.start_ms, 500.0);
    let names: Vec<&str> = report
        .pitch
        .melody
        .iter()
        .map(|n| n.name.as_str())
        .collect();
    assert_eq!(names, ["C5"], "{:?}", report.pitch.melody);

    // Clamping: out-of-range and inverted windows degrade, never panic.
    let (all, s) = audio.window(-1.0, None);
    assert_eq!(s, 0.0);
    assert_eq!(all.frames(), audio.frames());
    let (empty, _) = audio.window(5.0, Some(2.0));
    assert_eq!(empty.frames(), 0);
}

/// `peak_dbfs_between` — the plain numeric level measurement the transcribe
/// path uses to estimate a note's velocity. Its documented edge behavior
/// (clamped bounds, inverted or empty windows, silence, a degenerate rate)
/// is asserted here directly rather than only through the CLI round trip.
mod peak_level {
    use cochlea_features::{Audio, peak_dbfs_between};

    fn tone(amplitude: f32) -> Audio {
        Audio {
            samples: vec![amplitude; 48_000],
            channels: 1,
            sample_rate: 48_000,
        }
    }

    #[test]
    fn full_scale_reads_zero_dbfs() {
        let db = peak_dbfs_between(&tone(1.0), 0.0, 1000.0);
        assert!(db.abs() < 1e-9, "expected ~0 dBFS, got {db}");
    }

    #[test]
    fn half_scale_reads_about_minus_six() {
        let db = peak_dbfs_between(&tone(0.5), 0.0, 1000.0);
        assert!((db + 6.0206).abs() < 0.01, "expected ~-6.02 dBFS, got {db}");
    }

    #[test]
    fn silence_reads_negative_infinity() {
        assert_eq!(
            peak_dbfs_between(&tone(0.0), 0.0, 1000.0),
            f64::NEG_INFINITY
        );
    }

    #[test]
    fn an_inverted_or_empty_window_reads_as_silence() {
        let audio = tone(1.0);
        // end before start, and a zero-width window: both empty, not errors.
        assert_eq!(peak_dbfs_between(&audio, 500.0, 100.0), f64::NEG_INFINITY);
        assert_eq!(peak_dbfs_between(&audio, 200.0, 200.0), f64::NEG_INFINITY);
    }

    #[test]
    fn bounds_are_clamped_to_the_buffer() {
        let audio = tone(1.0);
        // Finite bounds past either edge clamp rather than panicking.
        assert!(peak_dbfs_between(&audio, -1e9, 1e9).abs() < 1e-9);
        assert_eq!(peak_dbfs_between(&audio, 5000.0, 6000.0), f64::NEG_INFINITY);
    }

    #[test]
    fn a_non_finite_bound_reads_as_zero_like_audio_window() {
        // Same convention as `Audio::window`'s `to_frame`: a non-finite
        // bound is degenerate input and resolves to 0, so it never indexes
        // out of range. `+inf` is therefore *not* a spelling of "to the
        // end" (Audio::window uses `None` for that) — it yields an empty
        // window, which reads as silence.
        let audio = tone(1.0);
        assert_eq!(
            peak_dbfs_between(&audio, f64::NAN, f64::NAN),
            f64::NEG_INFINITY
        );
        assert_eq!(
            peak_dbfs_between(&audio, f64::NEG_INFINITY, f64::INFINITY),
            f64::NEG_INFINITY
        );
    }

    #[test]
    fn a_zero_sample_rate_reads_as_silence_not_a_panic() {
        let audio = Audio {
            samples: vec![1.0; 100],
            channels: 1,
            sample_rate: 0,
        };
        assert_eq!(peak_dbfs_between(&audio, 0.0, 100.0), f64::NEG_INFINITY);
    }

    #[test]
    fn the_window_actually_selects_its_range() {
        // Loud only in the second half — a window over the first half must
        // not see it.
        let mut samples = vec![0.1f32; 48_000];
        samples.extend(vec![1.0f32; 48_000]);
        let audio = Audio {
            samples,
            channels: 1,
            sample_rate: 48_000,
        };
        let quiet = peak_dbfs_between(&audio, 0.0, 1000.0);
        let loud = peak_dbfs_between(&audio, 1000.0, 2000.0);
        assert!(
            (quiet + 20.0).abs() < 0.01,
            "first half ~-20 dBFS, got {quiet}"
        );
        assert!(loud.abs() < 1e-9, "second half ~0 dBFS, got {loud}");
    }

    #[test]
    fn stereo_is_measured_over_the_mono_downmix() {
        // Opposite-phase channels cancel in the downmix — the documented
        // behavior (it measures `mono()`, not per-channel peaks).
        let audio = Audio {
            samples: [1.0f32, -1.0].repeat(48_000),
            channels: 2,
            sample_rate: 48_000,
        };
        assert_eq!(peak_dbfs_between(&audio, 0.0, 1000.0), f64::NEG_INFINITY);
    }
}
