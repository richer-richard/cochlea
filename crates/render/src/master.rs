//! The master-bus stage: output gain and a brick-wall lookahead limiter,
//! run over the f64 stem sum before the final f32 conversion. Pure
//! arithmetic plus `libm::pow`/`libm::exp` — no fundsp, no state beyond
//! this function, deterministic like everything else in the fold.
//!
//! ## Limiter design (documented choice)
//!
//! Offline rendering removes the classic limiter constraint: the whole
//! buffer is available, so "lookahead" is a sliding *forward* window
//! maximum over per-frame peaks, not a delay line — no latency is added.
//! Per frame `i`, the required gain is `min(1, ceiling / max(peaks[i ..=
//! i+L]))`; the applied gain is the running release-smoothed gain clamped
//! from above by that requirement. Consequences, stated plainly:
//!
//! - **The sample-peak ceiling holds exactly.** `g[i] <= ceiling /
//!   peaks[i]` for every frame by construction — never a clipped sample,
//!   never clipping's harmonic distortion. (Inter-sample "true" peaks can
//!   still read fractionally higher; leave ~1 dB headroom under a
//!   `TruePeakBelow` target.)
//! - **Attack is a step, not a ramp**: gain drops to its reduced value the
//!   moment a peak enters the lookahead window (L frames early), in one
//!   step. A step in gain is a small discontinuity in the output; for
//!   limiting duty (a few dB on transients) this sits well below the
//!   artifact a ceiling clip would produce. A shaped attack ramp would
//!   trade exactness of the ceiling for smoothness — this implementation
//!   chooses the guarantee.
//! - **Release is a one-pole exponential toward unity** with the
//!   configured time constant, so gain recovery is click-free.

use cochlea_score::{Limiter, Master, SampleRate};
use std::collections::VecDeque;

/// `10^(db/20)`.
fn db_to_lin(db: f64) -> f64 {
    libm::pow(10.0, db / 20.0)
}

/// Apply the master chain (gain, then limiter) to an interleaved-stereo
/// f64 mix in place. A default master returns immediately without touching
/// a sample — the `mix == Σ stems` byte contract of a master-less score is
/// preserved exactly.
pub(crate) fn process(mix: &mut [f64], sr: SampleRate, master: &Master) {
    if master.is_default() {
        return;
    }
    let gain_db = f64::from(master.gain_db_value());
    if gain_db != 0.0 {
        let gain = db_to_lin(gain_db);
        for s in mix.iter_mut() {
            *s *= gain;
        }
    }
    if let Some(limiter) = master.limiter_value() {
        limit(mix, sr, limiter);
    }
}

fn limit(mix: &mut [f64], sr: SampleRate, limiter: &Limiter) {
    let frames = mix.len() / 2;
    if frames == 0 {
        return;
    }
    let sr_f = f64::from(sr.0);
    let ceiling = db_to_lin(f64::from(limiter.ceiling_db_value()));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "lookahead_ms is validated to 0..=50, so this is a small sample count"
    )]
    let lookahead = (f64::from(limiter.lookahead_ms_value()) / 1000.0 * sr_f).round() as usize;
    let release_s = f64::from(limiter.release_ms_value()) / 1000.0;
    // One-pole release coefficient; release_ms is validated >= 1 so the
    // divisor is never zero.
    let release_coeff = libm::exp(-1.0 / (release_s * sr_f));

    let peaks: Vec<f64> = (0..frames)
        .map(|i| mix[2 * i].abs().max(mix[2 * i + 1].abs()))
        .collect();
    let window_peaks = sliding_forward_max(&peaks, lookahead);

    let mut gain = 1.0f64;
    for i in 0..frames {
        // Release toward unity...
        gain = (gain * release_coeff + (1.0 - release_coeff)).min(1.0);
        // ...but never above what the upcoming window allows (instant
        // attack — the brick wall).
        let peak = window_peaks[i];
        if peak > ceiling {
            gain = gain.min(ceiling / peak);
        }
        mix[2 * i] *= gain;
        mix[2 * i + 1] *= gain;
    }
}

/// `out[i] = max(values[i ..= min(i + window, len-1)])` via a monotonic
/// deque — O(n), no floating-point accumulation, fully deterministic.
fn sliding_forward_max(values: &[f64], window: usize) -> Vec<f64> {
    let n = values.len();
    let mut out = vec![0.0f64; n];
    // Deque of indices with monotonically decreasing values, scanned right
    // to left so each position sees its *forward* window.
    let mut deque: VecDeque<usize> = VecDeque::new();
    for i in (0..n).rev() {
        while let Some(&front) = deque.front() {
            if front > i + window {
                deque.pop_front();
            } else {
                break;
            }
        }
        while let Some(&back) = deque.back() {
            if values[back] <= values[i] {
                deque.pop_back();
            } else {
                break;
            }
        }
        deque.push_back(i);
        out[i] = values[*deque.front().expect("just pushed")];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliding_forward_max_matches_brute_force() {
        let values = [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0];
        for window in 0..values.len() + 2 {
            let fast = sliding_forward_max(&values, window);
            for i in 0..values.len() {
                let end = (i + window).min(values.len() - 1);
                let slow = values[i..=end].iter().copied().fold(f64::MIN, f64::max);
                assert_eq!(fast[i], slow, "i={i} window={window}");
            }
        }
    }
}
