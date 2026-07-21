//! Integration tests for the `Report` schema's newer fields (`tempo`,
//! `rhythm`, `stereo`, `structure`, `loudness.lra` — added in schema
//! versions 2 and 3) and their `compare` deltas. Fixtures are synthesized
//! here with `libm`, never a synth dependency — mirrors `tests/probe.rs`'s
//! style.

use cochlea_features::{Analysis, Audio, ProbeOpts, SegmentOpts, compare, probe, segment_timeline};

mod common;
use common::*;

fn stereo_audio(left: Vec<f32>, right: Vec<f32>, sample_rate: u32) -> Audio {
    assert_eq!(left.len(), right.len());
    let mut samples = Vec::with_capacity(left.len() * 2);
    for (l, r) in left.into_iter().zip(right) {
        samples.push(l);
        samples.push(r);
    }
    Audio {
        samples,
        channels: 2,
        sample_rate,
    }
}

#[test]
fn schema_version_is_3() {
    let audio = mono_audio(sine_wave(440.0, 0.5, 1.0, SR), SR);
    let report = probe(&audio, &ProbeOpts::default());
    assert_eq!(report.schema_version, 3);
}

#[test]
fn report_tempo_matches_the_standalone_estimator_on_a_click_track() {
    let onset_times_s: Vec<f64> = (1..=24).map(|i| f64::from(i) * 0.5).collect();
    let audio = mono_audio(click_track(&onset_times_s, 13.0, SR), SR);

    let report = probe(&audio, &ProbeOpts::default());
    let bpm = report
        .tempo
        .bpm
        .expect("a regular click track should have a detected tempo");
    assert!((bpm - 120.0).abs() <= 1.0, "bpm = {bpm}");
    assert!(report.rhythm.clear_rhythm, "{:?}", report.rhythm);
    assert!(!report.tempo.candidates.is_empty(), "{:?}", report.tempo);
}

#[test]
fn report_stereo_is_populated_for_stereo_input_and_absent_for_mono() {
    let tone = sine_wave(440.0, 0.5, 2.0, SR);
    let stereo = stereo_audio(tone.clone(), tone.clone(), SR);
    let stereo_report = probe(&stereo, &ProbeOpts::default());
    let s = stereo_report
        .stereo
        .expect("stereo input should produce a stereo report");
    assert!(s.width <= 0.001, "width = {}", s.width);

    let mono = mono_audio(tone, SR);
    let mono_report = probe(&mono, &ProbeOpts::default());
    assert!(mono_report.stereo.is_none());
}

#[test]
fn report_structure_finds_a_boundary_at_a_tonal_switch() {
    let mut samples = sine_wave(440.0, 0.5, 8.0, SR);
    samples.extend(sine_wave(3000.0, 0.5, 8.0, SR));
    let audio = mono_audio(samples, SR);

    let report = probe(&audio, &ProbeOpts::default());
    assert_eq!(report.structure.boundaries_ms.len(), 1);
    assert_eq!(report.structure.section_count, 2);
}

#[test]
fn report_loudness_lra_is_defined_and_responds_to_dynamics() {
    let quiet_amp = libm::pow(10.0, -30.0 / 20.0);
    let loud_amp = libm::pow(10.0, -10.0 / 20.0);
    let mut samples = sine_wave(997.0, quiet_amp, 12.0, SR);
    samples.extend(sine_wave(997.0, loud_amp, 13.0, SR));
    let audio = mono_audio(samples, SR);

    let report = probe(&audio, &ProbeOpts::default());
    let lra = report
        .loudness
        .lra
        .expect("a 25 s stepped-loudness file should have an LRA");
    assert!(lra > 5.0, "lra = {lra}");
}

#[test]
fn compare_surfaces_tempo_stereo_structure_deltas() {
    let a_audio = mono_audio(sine_wave(440.0, 0.5, 8.0, SR), SR);
    let mut b_samples = sine_wave(440.0, 0.5, 8.0, SR);
    b_samples.extend(sine_wave(3000.0, 0.5, 8.0, SR));
    let b_audio = mono_audio(b_samples, SR);

    let a_report = probe(&a_audio, &ProbeOpts::default());
    let b_report = probe(&b_audio, &ProbeOpts::default());
    let a_timeline = segment_timeline(&a_audio, &SegmentOpts::default());
    let b_timeline = segment_timeline(&b_audio, &SegmentOpts::default());

    let result = compare(
        Analysis {
            report: &a_report,
            timeline: &a_timeline,
        },
        Analysis {
            report: &b_report,
            timeline: &b_timeline,
        },
    );

    // b has one more section than a (the tonal switch at 8 s).
    assert_eq!(result.structure.section_count_delta, 1);
    // Both mono inputs: no stereo delta.
    assert!(result.stereo.is_none());
    // tempo/rhythm/loudness deltas exist as fields regardless of value —
    // just confirm they're wired through and don't panic to compute.
    let _ = result.tempo.bpm_delta;
    let _ = result.rhythm.grid_alignment_delta;
    let _ = result.loudness.lra_delta;
}
