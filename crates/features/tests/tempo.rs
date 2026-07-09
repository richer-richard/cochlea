//! Integration tests for `cochlea_features::estimate_tempo`. Fixtures are
//! synthesized here with `libm`, never a synth dependency — mirrors
//! `tests/probe.rs`'s style.

use cochlea_features::{TempoOpts, estimate_tempo};

mod common;
use common::*;

/// A click track at a steady `bpm`, `seconds` long. First click at one
/// interval in, not t=0 — mirrors `tests/probe.rs`'s click-track fixture,
/// which avoids the same known edge effect (an onset at t=0 has no
/// preceding STFT frame, so the detector/beat-tracker's earliest frame is
/// biased; see the project's onset-at-zero notes).
fn click_track_at_bpm(bpm: f64, seconds: f64, sample_rate: u32) -> Vec<f32> {
    let interval_s = 60.0 / bpm;
    let mut onset_times = Vec::new();
    let mut t = interval_s;
    while t < seconds {
        onset_times.push(t);
        t += interval_s;
    }
    click_track(&onset_times, seconds, sample_rate)
}

#[test]
fn click_track_120_bpm_detected_with_tight_beat_grid() {
    let audio = mono_audio(click_track_at_bpm(120.0, 12.0, SR), SR);
    let report = estimate_tempo(&audio, &TempoOpts::default());

    let bpm = report
        .bpm
        .expect("a regular click track should have a detected tempo");
    assert!((bpm - 120.0).abs() <= 1.0, "bpm = {bpm}");
    assert!(
        report.clear_rhythm,
        "confidence={} beats_ms={:?}",
        report.confidence, report.beats_ms
    );

    assert!(
        report.beats_ms.len() >= 2,
        "beats_ms: {:?}",
        report.beats_ms
    );
    for pair in report.beats_ms.windows(2) {
        let spacing = pair[1] - pair[0];
        assert!(
            (spacing - 500.0).abs() <= 5.0,
            "beat spacing {spacing} ms too far from the true 500 ms interval: {:?}",
            report.beats_ms
        );
    }
}

#[test]
fn click_track_90_bpm_detected_without_octave_error() {
    let audio = mono_audio(click_track_at_bpm(90.0, 12.0, SR), SR);
    let report = estimate_tempo(&audio, &TempoOpts::default());

    let bpm = report
        .bpm
        .expect("a regular click track should have a detected tempo");
    assert!((bpm - 90.0).abs() <= 1.0, "bpm = {bpm}");
    // Octave-error guard: the log-Gaussian tempo prior (centered at 120
    // BPM) should keep the estimate from locking onto 90's double-tempo.
    assert!(
        (bpm - 180.0).abs() > 5.0,
        "bpm {bpm} looks like an octave error onto 180"
    );
    assert!(report.clear_rhythm, "{:?}", report);
}

#[test]
fn steady_tone_has_no_clear_rhythm() {
    let audio = mono_audio(sine_wave(440.0, 0.5, 8.0, SR), SR);
    let report = estimate_tempo(&audio, &TempoOpts::default());
    assert!(
        !report.clear_rhythm,
        "a sustained tone has no attacks and shouldn't read as a clear rhythm: {:?}",
        report
    );
}

#[test]
fn silence_is_degenerate_not_a_false_rhythm() {
    let audio = mono_audio(vec![0.0f32; SR as usize * 4], SR);
    let report = estimate_tempo(&audio, &TempoOpts::default());
    assert_eq!(report.bpm, None);
    assert_eq!(report.confidence, 0.0);
    assert!(!report.clear_rhythm);
    assert!(report.beats_ms.is_empty());
}

#[test]
fn empty_audio_never_panics_and_reports_undefined_tempo() {
    let audio = mono_audio(Vec::new(), SR);
    let report = estimate_tempo(&audio, &TempoOpts::default());
    assert_eq!(report.bpm, None);
    assert_eq!(report.confidence, 0.0);
    assert!(!report.clear_rhythm);
    assert!(report.beats_ms.is_empty());
}

#[test]
fn tempo_opts_bounds_are_chainable_and_respected() {
    // A narrow search window that excludes 120 BPM entirely — the winning
    // lag must fall inside [60, 100] BPM regardless of what the unbounded
    // search would have preferred.
    let audio = mono_audio(click_track_at_bpm(120.0, 12.0, SR), SR);
    let opts = TempoOpts::default().with_min_bpm(60.0).with_max_bpm(100.0);
    let report = estimate_tempo(&audio, &opts);
    let bpm = report.bpm.expect("should still find a peak in-range");
    assert!(
        (60.0..=100.0).contains(&bpm),
        "bpm {bpm} outside the requested [60, 100] range"
    );
}

/// Reversed BPM bounds normalize by swapping — the caller's intent is the
/// range between them, not a ~1 BPM sliver around the larger value (which
/// is what a naive clamp used to search).
#[test]
fn reversed_bpm_bounds_are_swapped_not_collapsed() {
    let audio = mono_audio(click_track_at_bpm(120.0, 12.0, SR), SR);
    let reversed = estimate_tempo(
        &audio,
        &TempoOpts::default().with_min_bpm(300.0).with_max_bpm(30.0),
    );
    let ordered = estimate_tempo(
        &audio,
        &TempoOpts::default().with_min_bpm(30.0).with_max_bpm(300.0),
    );
    assert_eq!(
        reversed.bpm, ordered.bpm,
        "swapped bounds must search the same range: {reversed:?} vs {ordered:?}"
    );
    let bpm = reversed.bpm.expect("click track has a tempo");
    assert!((bpm - 120.0).abs() <= 1.0, "bpm = {bpm}");
}
