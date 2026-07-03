//! Sample-clamp clipping count and the true-peak-over-0dBTP flag.

use crate::audio::Audio;
use crate::report::ClippingReport;

pub(crate) fn analyze(audio: &Audio, true_peak_dbtp: Option<f64>) -> ClippingReport {
    let clipped_samples = audio.samples.iter().filter(|&&x| x.abs() >= 1.0).count();
    let true_peak_over_0dbtp = true_peak_dbtp.is_some_and(|db| db > 0.0);
    ClippingReport {
        clipped_samples,
        true_peak_over_0dbtp,
    }
}
