//! Integration tests for `schema_version: 2`'s new `Report` fields
//! (`tempo`, `stereo`, `structure`, `loudness.lra`) and their `compare`
//! deltas. Fixtures are synthesized here with `libm`, never a synth
//! dependency — mirrors `tests/probe.rs`'s style.

use cochlea_features::{Analysis, Audio, ProbeOpts, SegmentOpts, compare, probe, segment_timeline};

const SR: u32 = 48_000;

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

fn click_track(onset_times_s: &[f64], total_s: f64, sample_rate: u32) -> Vec<f32> {
    let n = (total_s * f64::from(sample_rate)).round() as usize;
    let mut buf = vec![0.0f32; n];
    let tone_hz = 1000.0;
    let burst_len = (0.010 * f64::from(sample_rate)).round() as usize;
    let decay_len = (0.020 * f64::from(sample_rate)).round() as usize;
    let decay_tau_s = 0.005;

    for &t0 in onset_times_s {
        let start = (t0 * f64::from(sample_rate)).round() as usize;
        for i in 0..burst_len {
            let Some(sample) = buf.get_mut(start + i) else {
                break;
            };
            let t = i as f64 / f64::from(sample_rate);
            *sample = (0.9 * libm::sin(2.0 * std::f64::consts::PI * tone_hz * t)) as f32;
        }
        for i in 0..decay_len {
            let Some(sample) = buf.get_mut(start + burst_len + i) else {
                break;
            };
            let t = i as f64 / f64::from(sample_rate);
            let decay = libm::exp(-t / decay_tau_s);
            let phase = 2.0
                * std::f64::consts::PI
                * tone_hz
                * (burst_len as f64 / f64::from(sample_rate) + t);
            *sample = (0.9 * decay * libm::sin(phase)) as f32;
        }
    }
    buf
}

fn mono_audio(samples: Vec<f32>, sample_rate: u32) -> Audio {
    Audio {
        samples,
        channels: 1,
        sample_rate,
    }
}

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
fn schema_version_is_2() {
    let audio = mono_audio(sine_wave(440.0, 0.5, 1.0, SR), SR);
    let report = probe(&audio, &ProbeOpts::default());
    assert_eq!(report.schema_version, 2);
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
    assert!(report.tempo.clear_rhythm, "{:?}", report.tempo);
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
    // tempo/loudness deltas exist as fields regardless of value — just
    // confirm they're wired through and don't panic to compute.
    let _ = result.tempo.bpm_delta;
    let _ = result.loudness.lra_delta;
}
