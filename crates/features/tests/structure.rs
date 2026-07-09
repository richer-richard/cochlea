//! Integration tests for `cochlea_features::detect_structure`. Fixtures
//! are synthesized here with `libm`, never a synth dependency — mirrors
//! `tests/probe.rs`'s style.

use cochlea_features::{Audio, StructureOpts, detect_structure};

const SR: u32 = 48_000;

/// A mono sine wave at `freq_hz`, constant `amplitude`, `seconds` long.
fn sine_wave(freq_hz: f64, amplitude: f64, seconds: f64, sample_rate: u32) -> Vec<f32> {
    let n = (seconds * f64::from(sample_rate)).round() as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(sample_rate);
            let phase = 2.0 * std::f64::consts::PI * freq_hz * t;
            (amplitude * libm::sin(phase)) as f32
        })
        .collect()
}

fn mono_audio(samples: Vec<f32>, sample_rate: u32) -> Audio {
    Audio {
        samples,
        channels: 1,
        sample_rate,
    }
}

#[test]
fn ab_fixture_finds_one_boundary_near_the_switch() {
    let mut samples = sine_wave(440.0, 0.5, 8.0, SR);
    samples.extend(sine_wave(3000.0, 0.5, 8.0, SR));
    let audio = mono_audio(samples, SR);

    let report = detect_structure(&audio, &StructureOpts::default());
    assert_eq!(
        report.boundaries_ms.len(),
        1,
        "boundaries: {:?}",
        report.boundaries_ms
    );
    assert_eq!(report.section_count, 2);

    let boundary_s = report.boundaries_ms[0] / 1000.0;
    assert!(
        (boundary_s - 8.0).abs() <= 1.5,
        "boundary at {boundary_s} s, expected near 8.0 s"
    );
    assert!(
        report.confidence > 0.0,
        "confidence = {}",
        report.confidence
    );
}

#[test]
fn aba_fixture_finds_two_boundaries_near_each_switch() {
    let mut samples = sine_wave(440.0, 0.5, 8.0, SR);
    samples.extend(sine_wave(3000.0, 0.5, 8.0, SR));
    samples.extend(sine_wave(440.0, 0.5, 8.0, SR));
    let audio = mono_audio(samples, SR);

    let report = detect_structure(&audio, &StructureOpts::default());
    assert_eq!(
        report.boundaries_ms.len(),
        2,
        "boundaries: {:?}",
        report.boundaries_ms
    );
    assert_eq!(report.section_count, 3);

    let first_s = report.boundaries_ms[0] / 1000.0;
    let second_s = report.boundaries_ms[1] / 1000.0;
    assert!(
        (first_s - 8.0).abs() <= 1.5,
        "first boundary at {first_s} s, expected near 8.0 s"
    );
    assert!(
        (second_s - 16.0).abs() <= 1.5,
        "second boundary at {second_s} s, expected near 16.0 s"
    );
}

#[test]
fn steady_tone_has_no_boundaries() {
    let audio = mono_audio(sine_wave(440.0, 0.5, 24.0, SR), SR);
    let report = detect_structure(&audio, &StructureOpts::default());
    assert!(
        report.boundaries_ms.is_empty(),
        "boundaries: {:?}",
        report.boundaries_ms
    );
    assert_eq!(report.section_count, 1);
    assert_eq!(report.confidence, 0.0);
}

#[test]
fn silence_is_degenerate_not_a_false_structure() {
    let audio = mono_audio(vec![0.0f32; SR as usize * 24], SR);
    let report = detect_structure(&audio, &StructureOpts::default());
    assert!(report.boundaries_ms.is_empty());
    assert_eq!(report.section_count, 1);
    assert_eq!(report.confidence, 0.0);
}

#[test]
fn empty_audio_never_panics_and_reports_zero_sections() {
    let audio = mono_audio(Vec::new(), SR);
    let report = detect_structure(&audio, &StructureOpts::default());
    assert!(report.boundaries_ms.is_empty());
    assert_eq!(report.section_count, 0);
    assert_eq!(report.confidence, 0.0);
}

#[test]
fn short_audio_is_one_section_not_a_panic() {
    // Well under 2 * KERNEL_HALF_WIDTH (16) one-second frames.
    let audio = mono_audio(sine_wave(440.0, 0.5, 3.0, SR), SR);
    let report = detect_structure(&audio, &StructureOpts::default());
    assert!(report.boundaries_ms.is_empty());
    assert_eq!(report.section_count, 1);
}

#[test]
fn max_sections_cap_is_respected_on_many_alternations() {
    // 12 alternating 4 s blocks (48 s total) — comfortably more candidate
    // boundaries than a small max_sections cap allows.
    let mut samples = Vec::new();
    for i in 0..12u32 {
        let freq = if i % 2 == 0 { 440.0 } else { 3000.0 };
        samples.extend(sine_wave(freq, 0.5, 4.0, SR));
    }
    let audio = mono_audio(samples, SR);

    let opts = StructureOpts::default().with_max_sections(4);
    let report = detect_structure(&audio, &opts);
    assert!(
        report.section_count <= 4,
        "section_count = {}, boundaries: {:?}",
        report.section_count,
        report.boundaries_ms
    );
    // Boundaries must stay in ascending order after the strongest-first
    // cap re-sorts them chronologically.
    for pair in report.boundaries_ms.windows(2) {
        assert!(
            pair[0] < pair[1],
            "boundaries out of order: {:?}",
            report.boundaries_ms
        );
    }
}

#[test]
fn structure_opts_are_chainable() {
    let opts = StructureOpts::default()
        .with_frame_ms(500.0)
        .with_max_sections(6);
    assert_eq!(opts.frame_ms, 500.0);
    assert_eq!(opts.max_sections, 6);
}
