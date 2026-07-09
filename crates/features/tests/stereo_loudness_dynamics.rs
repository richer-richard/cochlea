//! Integration tests for `cochlea_features::stereo_image` and
//! `cochlea_features::loudness_dynamics`. Fixtures are synthesized here
//! with `libm`, never a synth dependency — mirrors `tests/probe.rs`'s
//! style.

use cochlea_features::{Audio, loudness_dynamics, stereo_image};

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
    let report = stereo_image(&audio);

    let correlation = report
        .correlation
        .expect("identical channels have variance");
    assert!(correlation >= 0.999, "correlation = {correlation}");
    let width = report.width.expect("identical channels have energy");
    assert!(width <= 0.001, "width = {width}");
    let balance = report.balance.expect("identical channels have energy");
    assert!(balance.abs() <= 0.001, "balance = {balance}");
}

#[test]
fn out_of_phase_channels_read_as_maximally_wide() {
    let tone = sine_wave(440.0, 0.5, 2.0, SR);
    let inverted: Vec<f32> = tone.iter().map(|&s| -s).collect();
    let audio = stereo_audio(tone, inverted, SR);
    let report = stereo_image(&audio);

    let correlation = report
        .correlation
        .expect("out-of-phase channels have variance");
    assert!(correlation <= -0.999, "correlation = {correlation}");
    let width = report.width.expect("out-of-phase channels have energy");
    assert!(width >= 0.999, "width = {width}");
    let balance = report
        .balance
        .expect("equal-amplitude channels have energy");
    assert!(balance.abs() <= 0.001, "balance = {balance}");
}

#[test]
fn left_only_signal_reads_fully_left_with_half_width() {
    let tone = sine_wave(440.0, 0.5, 2.0, SR);
    let silence = vec![0.0f32; tone.len()];
    let audio = stereo_audio(tone, silence, SR);
    let report = stereo_image(&audio);

    // R is a hard zero, so its variance is zero: correlation is undefined.
    assert_eq!(report.correlation, None);
    let balance = report.balance.expect("left channel alone has energy");
    assert!(balance <= -0.999, "balance = {balance}");
    // mid = L/2, side = L/2 exactly whenever R = 0 for every sample, so
    // mid_energy == side_energy identically — width is exactly 0.5, not an
    // approximation, regardless of the left channel's actual waveform.
    let width = report.width.expect("left channel alone has energy");
    assert!((width - 0.5).abs() <= 1e-9, "width = {width}");
}

#[test]
fn mono_input_has_no_stereo_image() {
    let audio = mono_audio(sine_wave(440.0, 0.5, 1.0, SR), SR);
    let report = stereo_image(&audio);
    assert_eq!(report.correlation, None);
    assert_eq!(report.width, None);
    assert_eq!(report.balance, None);
}

#[test]
fn stereo_silence_has_no_stereo_image() {
    let n = SR as usize * 2;
    let audio = stereo_audio(vec![0.0f32; n], vec![0.0f32; n], SR);
    let report = stereo_image(&audio);
    assert_eq!(report.correlation, None);
    assert_eq!(report.width, None);
    assert_eq!(report.balance, None);
}

// ----------------------------------------------------------- loudness dynamics

#[test]
fn constant_level_tone_has_near_zero_loudness_range() {
    let amplitude = libm::pow(10.0, -18.0 / 20.0);
    let audio = mono_audio(sine_wave(997.0, amplitude, 10.0, SR), SR);
    let report = loudness_dynamics(&audio);

    let lra = report
        .lra
        .expect("a steady 10 s tone should have a defined LRA");
    assert!(
        lra <= 1.0,
        "lra = {lra} (expected near 0 for constant loudness)"
    );

    assert!(
        !report.short_term.is_empty(),
        "a 10 s buffer should have short-term points"
    );
    for point in &report.short_term {
        let lufs = point
            .lufs
            .unwrap_or_else(|| panic!("point at {} ms should be voiced", point.time_ms));
        assert!(
            (lufs - (-21.0)).abs() <= 2.0,
            "short-term LUFS {lufs} at {} ms far from the steady-state ~-21 LUFS",
            point.time_ms
        );
    }
}

#[test]
fn loud_then_quiet_has_larger_loudness_range_than_constant() {
    let loud_amp = libm::pow(10.0, -10.0 / 20.0);
    let quiet_amp = libm::pow(10.0, -40.0 / 20.0);
    let mut samples = sine_wave(997.0, loud_amp, 5.0, SR);
    samples.extend(sine_wave(997.0, quiet_amp, 5.0, SR));
    let audio = mono_audio(samples, SR);
    let report = loudness_dynamics(&audio);

    let lra = report
        .lra
        .expect("a loud-then-quiet buffer should have a defined LRA");
    // EBU R128 LRA is a percentile-based statistic (10th-95th percentile of
    // gated short-term loudness), not a raw max-min span, so a 30 dB step
    // doesn't read back as the full nominal jump — this just needs to be
    // clearly above the constant-tone case's near-zero LRA (asserted <=1.0
    // LU in the test above).
    assert!(lra > 2.0, "lra = {lra}, expected a large loudness range");

    // The short-term curve should show a clear drop from the loud half to
    // the quiet half.
    let first = report
        .short_term
        .first()
        .and_then(|p| p.lufs)
        .expect("first short-term point should be voiced");
    let last = report
        .short_term
        .last()
        .and_then(|p| p.lufs)
        .expect("last short-term point should be voiced");
    assert!(
        first - last > 5.0,
        "expected the curve to drop from loud ({first}) to quiet ({last})"
    );
}

#[test]
fn short_buffer_has_no_short_term_points() {
    // Under the 3 s short-term window: no point is ever "ready."
    let audio = mono_audio(sine_wave(440.0, 0.5, 1.0, SR), SR);
    let report = loudness_dynamics(&audio);
    assert!(
        report.short_term.is_empty(),
        "short_term: {:?}",
        report.short_term
    );
}

#[test]
fn empty_audio_never_panics() {
    let audio = mono_audio(Vec::new(), SR);
    let report = loudness_dynamics(&audio);
    assert_eq!(report.lra, None);
    assert!(report.short_term.is_empty());
}
