//! Shared synthesized fixtures for the integration tests — one
//! implementation of the sine/click/silence builders that previously lived
//! as byte-identical copies in six test files. Fixtures use `libm` only,
//! never a synth dependency.
//!
//! Each test binary compiles this module independently and uses its own
//! subset of the helpers, so per-binary dead-code analysis would flag
//! whichever items that binary happens not to call — the allow below is
//! the standard `tests/common` accommodation for that, not a suppressed
//! real warning.
#![allow(dead_code)]

use cochlea_features::Audio;

pub const SR: u32 = 48_000;

pub fn sine_wave(freq_hz: f64, amplitude: f64, seconds: f64, sample_rate: u32) -> Vec<f32> {
    let n = (seconds * f64::from(sample_rate)).round() as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(sample_rate);
            let phase = 2.0 * std::f64::consts::PI * freq_hz * t;
            (amplitude * libm::sin(phase)) as f32
        })
        .collect()
}

pub fn silence(seconds: f64, sample_rate: u32) -> Vec<f32> {
    vec![0.0f32; (seconds * f64::from(sample_rate)).round() as usize]
}

pub fn click_track(onset_times_s: &[f64], total_s: f64, sample_rate: u32) -> Vec<f32> {
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

pub fn mono_audio(samples: Vec<f32>, sample_rate: u32) -> Audio {
    Audio {
        samples,
        channels: 1,
        sample_rate,
    }
}
