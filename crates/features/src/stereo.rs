//! Stereo image: mid/side width, zero-lag L/R correlation, and left/right
//! level balance — whole-buffer aggregate metrics over interleaved stereo
//! samples. No score/synth involved (`docs/plan.md`).

use serde::{Deserialize, Serialize};

use crate::audio::Audio;

/// Stereo-image metrics for one [`Audio`] buffer. Plain struct, no own
/// schema version — parallel API, meant to be embedded into a future
/// `Report` schema bump rather than stand alone (mirrors
/// [`crate::TempoReport`]'s status). See [`analyze_stereo`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StereoReport {
    /// Mid/side energy width, `0.0..=1.0`. `0.0` = mono-identical channels,
    /// `~0.5` = uncorrelated channels, `1.0` = pure out-of-phase. See
    /// [`analyze_stereo`] for the exact formula. Always defined (defaults
    /// to `0.0` — "no side signal" — when both channels are digital
    /// silence, rather than being `Option`).
    pub width: f64,
    /// Zero-lag normalized cross-correlation of the left/right channels,
    /// `-1.0..=1.0`. `1.0` = identical, `0.0` = decorrelated, `-1.0` =
    /// fully out of phase (`R = -L`). `None` when either channel is
    /// digital silence (undefined, never `NaN`).
    pub correlation: Option<f64>,
    /// Left/right level balance, `-1.0..=1.0`: `(r_rms - l_rms) / (l_rms +
    /// r_rms)`. Negative = left-heavy, positive = right-heavy. `None` when
    /// both channels are digital silence (undefined, never `NaN`).
    pub balance: Option<f64>,
}

/// Whole-buffer stereo-image metrics. `None` unless `audio.channels == 2`
/// — there's no stereo image to measure for mono or >2-channel input (this
/// crate has no surround-specific handling).
///
/// `width` comes from the mid/side decomposition (`mid = (l+r)/2`, `side =
/// (l-r)/2`): `width = side_rms / (mid_rms + side_rms)`. `correlation` is
/// the standard Pearson coefficient of the L/R sample streams. `balance` is
/// `(r_rms - l_rms) / (l_rms + r_rms)`.
pub fn analyze_stereo(audio: &Audio) -> Option<StereoReport> {
    if audio.channels != 2 {
        return None;
    }

    let n = audio.samples.len() / 2;
    if n == 0 {
        return Some(StereoReport {
            width: 0.0,
            correlation: None,
            balance: None,
        });
    }

    let mut sum_l = 0.0f64;
    let mut sum_r = 0.0f64;
    let mut sum_l2 = 0.0f64;
    let mut sum_r2 = 0.0f64;
    let mut sum_lr = 0.0f64;
    let mut mid_sq_sum = 0.0f64;
    let mut side_sq_sum = 0.0f64;

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
        mid_sq_sum += mid * mid;
        side_sq_sum += side * side;
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

    let mid_rms = (mid_sq_sum / count).sqrt();
    let side_rms = (side_sq_sum / count).sqrt();
    let width = if mid_rms + side_rms > 0.0 {
        (side_rms / (mid_rms + side_rms)).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let rms_l = (sum_l2 / count).sqrt();
    let rms_r = (sum_r2 / count).sqrt();
    let balance =
        (rms_l + rms_r > 0.0).then(|| ((rms_r - rms_l) / (rms_l + rms_r)).clamp(-1.0, 1.0));

    Some(StereoReport {
        width,
        correlation,
        balance,
    })
}
