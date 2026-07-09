//! Integration tests for `cochlea_features::analyze_stereo` and
//! `cochlea_features::loudness_range`. Fixtures are synthesized here with
//! `libm`, never a synth dependency — mirrors `tests/probe.rs`'s style.

use cochlea_features::{Audio, analyze_stereo, loudness_range};

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

/// Interleave two equal-length mono channels into stereo `Audio`.
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

// ------------------------------------------------------------------ stereo

#[test]
fn identical_channels_read_as_mono_compatible() {
    let tone = sine_wave(440.0, 0.5, 2.0, SR);
    let audio = stereo_audio(tone.clone(), tone, SR);
    let report = analyze_stereo(&audio).expect("stereo input should produce a report");

    assert!(report.width <= 0.001, "width = {}", report.width);
    let correlation = report
        .correlation
        .expect("identical channels have variance");
    assert!(correlation >= 0.999, "correlation = {correlation}");
    let balance = report.balance.expect("identical channels have energy");
    assert!(balance.abs() <= 0.001, "balance = {balance}");
}

#[test]
fn out_of_phase_channels_read_as_maximally_wide() {
    let tone = sine_wave(440.0, 0.5, 2.0, SR);
    let inverted: Vec<f32> = tone.iter().map(|&s| -s).collect();
    let audio = stereo_audio(tone, inverted, SR);
    let report = analyze_stereo(&audio).expect("stereo input should produce a report");

    assert!(report.width >= 0.999, "width = {}", report.width);
    let correlation = report
        .correlation
        .expect("out-of-phase channels have variance");
    assert!(correlation <= -0.999, "correlation = {correlation}");
}

#[test]
fn left_only_signal_reads_fully_left_balance() {
    let tone = sine_wave(440.0, 0.5, 2.0, SR);
    let silence = vec![0.0f32; tone.len()];
    let audio = stereo_audio(tone, silence, SR);
    let report = analyze_stereo(&audio).expect("stereo input should produce a report");

    // balance = (r_rms - l_rms) / (l_rms + r_rms); r_rms = 0 here, so this
    // reads -1 (fully left) under this module's documented sign
    // convention.
    let balance = report.balance.expect("left channel alone has energy");
    assert!(balance <= -0.999, "balance = {balance}");
}

#[test]
fn decorrelated_channels_read_low_correlation_and_midrange_width() {
    // Different frequencies on each channel: no linear relationship, so
    // correlation should sit near zero (loose bound — two finite-length
    // sinusoids are never perfectly decorrelated) and width somewhere
    // between the mono and fully-wide extremes.
    let audio = stereo_audio(
        sine_wave(440.0, 0.5, 2.0, SR),
        sine_wave(3000.0, 0.5, 2.0, SR),
        SR,
    );
    let report = analyze_stereo(&audio).expect("stereo input should produce a report");

    let correlation = report.correlation.expect("both channels have variance");
    assert!(correlation.abs() <= 0.2, "correlation = {correlation}");
    assert!(
        (0.2..=0.8).contains(&report.width),
        "width = {}",
        report.width
    );
}

#[test]
fn mono_input_has_no_stereo_report() {
    let audio = mono_audio(sine_wave(440.0, 0.5, 1.0, SR), SR);
    assert!(analyze_stereo(&audio).is_none());
}

#[test]
fn stereo_silence_has_undefined_correlation_and_balance() {
    let n = SR as usize * 2;
    let audio = stereo_audio(vec![0.0f32; n], vec![0.0f32; n], SR);
    let report = analyze_stereo(&audio).expect("channels == 2, so this is Some even for silence");
    assert_eq!(report.correlation, None);
    assert_eq!(report.balance, None);
}

// --------------------------------------------------------------- loudness range

#[test]
fn stepped_loudness_has_a_large_range() {
    let quiet_amp = libm::pow(10.0, -30.0 / 20.0);
    let loud_amp = libm::pow(10.0, -10.0 / 20.0);
    let mut samples = sine_wave(997.0, quiet_amp, 12.0, SR);
    samples.extend(sine_wave(997.0, loud_amp, 13.0, SR));
    let audio = mono_audio(samples, SR);

    let lra = loudness_range(&audio).expect("a 25 s stepped-loudness file should have an LRA");
    // Measured: ~20.0 LU (a clean match to the 20 dB step, since LRA's
    // percentile gating has plenty of blocks on each side of a hard,
    // sustained level change to work with) — asserting loosely (>5.0)
    // rather than pinning the exact value, per the task brief.
    assert!(lra > 5.0, "lra = {lra}, expected a large loudness range");
}

#[test]
fn constant_level_tone_has_near_zero_loudness_range() {
    let amplitude = libm::pow(10.0, -18.0 / 20.0);
    let audio = mono_audio(sine_wave(997.0, amplitude, 25.0, SR), SR);

    let lra = loudness_range(&audio).expect("a steady 25 s tone should have an LRA");
    assert!(
        lra < 1.0,
        "lra = {lra}, expected near 0 for constant loudness"
    );
}

#[test]
fn short_file_has_a_defined_near_zero_range_not_an_error() {
    // ebur128's LRA never errors for present-but-brief audio (measured:
    // even a single sample reads `Some(0.0)`) — insufficient history for a
    // meaningful range reads as "no range" (0.0), not "undefined". `None`
    // is reserved for genuinely absent audio; see the test below.
    let audio = mono_audio(sine_wave(440.0, 0.5, 1.0, SR), SR);
    let lra = loudness_range(&audio).expect("present audio always has a defined LRA");
    assert!(
        lra.abs() < 1e-9,
        "lra = {lra}, expected exactly ~0 for 1 s of audio"
    );
}

#[test]
fn empty_audio_has_no_loudness_range() {
    let audio = mono_audio(Vec::new(), SR);
    assert_eq!(loudness_range(&audio), None);
}
