//! Onset detection: half-wave-rectified spectral flux over a 1024/256 STFT
//! (short window, for time resolution), an adaptive (rolling-median)
//! threshold, and minimum-gap peak picking.
//!
//! **Frame-time convention**: an onset detected at STFT frame `t` is
//! reported at frame `t`'s *center* in milliseconds — `(t * HOP +
//! FFT_SIZE / 2) / sample_rate` seconds, times `1000` — not its start.
//!
//! This convention was chosen empirically, not derived analytically: flux
//! at frame `t` is a first difference between frame `t` and frame `t - 1`'s
//! magnitude, and because the analysis window is Hann-tapered, a transient
//! contributes almost no energy while it sits near a window's edge — flux
//! only rises once the transient has moved well into the window, closer to
//! its center. Reporting the frame center (rather than the start, which
//! measured ~10-15 ms early on this crate's click-track fixture) absorbs
//! most of that lag; see the click-track test for the measured residual (a
//! few ms, well inside the 6 ms — one analysis frame — tolerance it
//! asserts).

use crate::report::OnsetsReport;
use crate::stft::Stft;

/// FFT size in samples. At 48 kHz this is ~21 ms — good time resolution
/// for locating transients (`docs/plan.md`). `pub(crate)`: `tempo` reuses
/// this exact STFT so its autocorrelation lag-to-BPM math and this module's
/// frame-center convention stay in lockstep.
pub(crate) const FFT_SIZE: usize = 1024;
/// Hop size in samples (75% overlap at `FFT_SIZE = 1024`). See `FFT_SIZE`
/// on visibility.
pub(crate) const HOP: usize = 256;
/// Rolling-median window for the adaptive threshold, in STFT frames. Wide
/// enough to average over a full cycle of the "picket fence" flux ripple a
/// stationary, non-bin-aligned tone produces (see `DELTA_SCALE`).
const MEDIAN_WINDOW: usize = 21;
/// Threshold = local median + `DELTA_SCALE` * local MAD (median absolute
/// deviation), floored by `DELTA_MIN` so a silent/uniform flux signal
/// (float noise around zero) never produces spurious onsets.
///
/// Tuned empirically against two measurements on this crate's fixtures: a
/// sustained 440 Hz tone's own frame-to-frame flux ripples up to ~0.29 (its
/// magnitude isn't exactly bin-aligned to the 1024-point FFT, so the
/// windowed spectrum's exact peak shifts slightly as the analysis window
/// slides — expected DSP behavior, not noise), while the click-track
/// fixture's true onsets spike to ~560, over three orders of magnitude
/// higher. `DELTA_SCALE = 8.0` clears the sustained-tone ripple (see the
/// `sustained_tone_has_no_false_onsets` test) with enormous headroom still
/// left below a real transient.
const DELTA_SCALE: f64 = 8.0;
const DELTA_MIN: f64 = 1e-6;
/// Minimum time between accepted onsets.
const MIN_GAP_MS: f64 = 30.0;

pub(crate) fn analyze(mono: &[f32], sample_rate: u32) -> OnsetsReport {
    let stft = Stft::compute(mono, sample_rate, FFT_SIZE, HOP);
    let frame_count = stft.magnitudes.len();
    if frame_count < 3 {
        return OnsetsReport {
            count: 0,
            times_ms: Vec::new(),
        };
    }

    let flux = spectral_flux(&stft);
    let threshold = adaptive_threshold(&flux);

    let min_gap_frames =
        ((MIN_GAP_MS / 1000.0 * f64::from(stft.sample_rate) / HOP as f64).ceil() as usize).max(1);

    let mut peaks = Vec::new();
    for t in 1..frame_count - 1 {
        if flux[t] > threshold[t] && flux[t] >= flux[t - 1] && flux[t] > flux[t + 1] {
            peaks.push(t);
        }
    }

    let accepted = suppress_close_peaks(&peaks, &flux, min_gap_frames);
    let times_ms: Vec<f64> = accepted
        .into_iter()
        .map(|t| {
            (t as f64 * HOP as f64 + FFT_SIZE as f64 / 2.0) / f64::from(stft.sample_rate) * 1000.0
        })
        .collect();

    OnsetsReport {
        count: times_ms.len(),
        times_ms,
    }
}

/// Half-wave-rectified spectral flux: `flux[t] = sum(max(0, mag[t][b] -
/// mag[t-1][b]))` over bins `b`. `flux[0]` is always `0.0` (no predecessor).
/// `pub(crate)`: `tempo`'s autocorrelation runs over this same onset
/// strength envelope rather than recomputing it.
pub(crate) fn spectral_flux(stft: &Stft) -> Vec<f64> {
    let mut flux = vec![0.0];
    flux.extend(stft.magnitudes.windows(2).map(|pair| {
        let (prev, cur) = (&pair[0], &pair[1]);
        cur.iter()
            .zip(prev.iter())
            .map(|(&c, &p)| (f64::from(c) - f64::from(p)).max(0.0))
            .sum::<f64>()
    }));
    flux
}

/// Per-frame adaptive threshold: a local (rolling, edge-clamped) median
/// plus a scaled local median absolute deviation.
fn adaptive_threshold(flux: &[f64]) -> Vec<f64> {
    let half = MEDIAN_WINDOW / 2;
    let n = flux.len();
    (0..n)
        .map(|t| {
            let lo = t.saturating_sub(half);
            let hi = (t + half + 1).min(n);
            let window = &flux[lo..hi];
            let med = median_of(window);
            let mad = mad_of(window, med);
            med + DELTA_SCALE * mad + DELTA_MIN
        })
        .collect()
}

fn median_of(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

fn mad_of(values: &[f64], median: f64) -> f64 {
    let deviations: Vec<f64> = values.iter().map(|v| (v - median).abs()).collect();
    median_of(&deviations)
}

/// Non-maximum suppression: within any window of `min_gap` frames, keep
/// only the highest-flux candidate peak.
fn suppress_close_peaks(peaks: &[usize], flux: &[f64], min_gap: usize) -> Vec<usize> {
    let mut accepted = Vec::new();
    let mut i = 0;
    while i < peaks.len() {
        let mut best = peaks[i];
        let mut j = i + 1;
        while j < peaks.len() && peaks[j] - best <= min_gap {
            if flux[peaks[j]] > flux[best] {
                best = peaks[j];
            }
            j += 1;
        }
        accepted.push(best);
        i = j;
    }
    accepted
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sustained tone isn't silent and doesn't have a note-attack after
    /// its own start, but non-bin-aligned stationary content still
    /// produces a small frame-to-frame flux ripple (see `DELTA_SCALE`'s
    /// docs) — regression-test that the adaptive threshold clears it.
    #[test]
    fn sustained_tone_has_no_false_onsets() {
        let sr = 48_000u32;
        let n = sr as usize * 2;
        let mono: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(sr);
                (0.5 * libm::sin(2.0 * std::f64::consts::PI * 440.0 * t)) as f32
            })
            .collect();
        let report = analyze(&mono, sr);
        assert_eq!(
            report.count, 0,
            "sustained tone should have no onsets after its own start: {:?}",
            report.times_ms
        );
    }
}
