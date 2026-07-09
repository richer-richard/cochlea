//! Stereo image: phase correlation, mid/side width, and left/right level
//! balance — whole-buffer aggregate metrics over interleaved stereo
//! samples. No score/synth involved (`docs/plan.md`).

use serde::{Deserialize, Serialize};

use crate::audio::Audio;

/// Stereo-image metrics for one [`Audio`] buffer, `schema_version`-free
/// (parallel API, not embedded in [`crate::Report`] — see [`stereo_image`]
/// for why). `None` fields mean "no stereo image to measure": non-stereo
/// input (`channels != 2`) or digital silence both produce all-`None`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StereoReport {
    /// Pearson correlation coefficient of the left/right channels,
    /// `-1.0..=1.0`. `1.0` = identical (mono-compatible), `0.0` =
    /// decorrelated, `-1.0` = fully out of phase (`R = -L`).
    pub correlation: Option<f64>,
    /// Fraction of mid/side energy that's side, `0.0..=1.0`. `0.0` = mono
    /// (all energy in the mid channel), `1.0` = maximally wide (no shared
    /// mid content at all). See [`stereo_image`] for the exact formula.
    pub width: Option<f64>,
    /// Left/right level balance, `-1.0..=1.0`. Negative = left-heavy,
    /// positive = right-heavy, `0.0` = balanced.
    pub balance: Option<f64>,
}

fn undefined() -> StereoReport {
    StereoReport {
        correlation: None,
        width: None,
        balance: None,
    }
}

/// Whole-buffer stereo-image metrics. Only meaningful for `audio.channels
/// == 2`; anything else (mono, or >2 channels — this crate has no
/// surround-specific handling) returns all-`None`, as does digital
/// silence.
///
/// `width` is derived from the mid/side decomposition (`mid = (l+r)/2`,
/// `side = (l-r)/2`): `width = side_energy / (mid_energy + side_energy)`,
/// where `*_energy` is the buffer's mean squared amplitude. Because
/// `l^2+r^2 = 2*(mid^2+side^2)` exactly for every sample (the mid/side
/// transform preserves energy up to that constant factor), this is a
/// bounded `0.0..=1.0` fraction of total stereo energy that's "side" —
/// `0.0` at mono, `1.0` at maximally decorrelated — rather than an
/// unbounded mid/side ratio.
pub fn stereo_image(audio: &Audio) -> StereoReport {
    if audio.channels != 2 {
        return undefined();
    }

    let n = audio.samples.len() / 2;
    if n == 0 {
        return undefined();
    }

    let mut sum_l = 0.0f64;
    let mut sum_r = 0.0f64;
    let mut sum_l2 = 0.0f64;
    let mut sum_r2 = 0.0f64;
    let mut sum_lr = 0.0f64;
    let mut mid_energy = 0.0f64;
    let mut side_energy = 0.0f64;

    for frame in audio.samples.chunks_exact(2) {
        let l = f64::from(frame[0]);
        let r = f64::from(frame[1]);
        sum_l += l;
        sum_r += r;
        sum_l2 += l * l;
        sum_r2 += r * r;
        sum_lr += l * r;

        let mid = (l + r) / 2.0;
        let side = (l - r) / 2.0;
        mid_energy += mid * mid;
        side_energy += side * side;
    }

    let count = n as f64;
    let mean_l = sum_l / count;
    let mean_r = sum_r / count;
    // `.max(0.0)`: mathematically variance can't be negative, but the
    // sum-of-squares formula can dip a hair below zero on a near-constant
    // signal from floating-point cancellation.
    let var_l = (sum_l2 / count - mean_l * mean_l).max(0.0);
    let var_r = (sum_r2 / count - mean_r * mean_r).max(0.0);
    let cov = sum_lr / count - mean_l * mean_r;

    let correlation = (var_l > 0.0 && var_r > 0.0)
        .then(|| (cov / (var_l.sqrt() * var_r.sqrt())).clamp(-1.0, 1.0));

    let total_ms_energy = mid_energy + side_energy;
    let width = (total_ms_energy > 0.0).then(|| (side_energy / total_ms_energy).clamp(0.0, 1.0));

    let rms_l = (sum_l2 / count).sqrt();
    let rms_r = (sum_r2 / count).sqrt();
    let balance =
        (rms_l + rms_r > 0.0).then(|| ((rms_r - rms_l) / (rms_r + rms_l)).clamp(-1.0, 1.0));

    StereoReport {
        correlation,
        width,
        balance,
    }
}
