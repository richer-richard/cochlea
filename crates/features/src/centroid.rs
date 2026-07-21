//! Spectral centroid: the magnitude-weighted mean frequency per STFT
//! frame — a standard "brightness" proxy. Public so `cochlea-verify` can
//! check that a rendered filter sweep audibly brightened/darkened the
//! *output audio* (its `Monotone` check deliberately validates only the
//! authored automation curve; this is the other half of that story).

use crate::stft::Stft;

/// STFT size for the centroid track — `key`-grade frequency resolution
/// isn't needed; 2048/512 matches the pitch tracker's time base.
const FFT_SIZE: usize = 2048;
const HOP: usize = 512;

/// One frame of the centroid track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CentroidPoint {
    /// Frame-center time, milliseconds (the same convention as
    /// [`crate::OnsetsReport::times_ms`]).
    pub t_ms: f64,
    /// Magnitude-weighted mean frequency, Hz. `None` for an (effectively)
    /// silent frame — a centroid of silence is undefined, not zero.
    pub hz: Option<f64>,
}

/// Threshold below which a frame's total magnitude counts as silence and
/// its centroid is reported as `None` rather than as the ratio of two
/// float-noise sums.
const SILENCE_MAG_FLOOR: f64 = 1e-9;

/// The spectral-centroid track of `mono` — one [`CentroidPoint`] per
/// 2048/512 STFT frame. Empty for buffers shorter than one frame or a zero
/// sample rate. Deterministic: fixed ascending summation order, `libm`-free
/// (pure arithmetic — a centroid is a weighted mean).
pub fn spectral_centroid_curve(mono: &[f32], sample_rate: u32) -> Vec<CentroidPoint> {
    if sample_rate == 0 || mono.len() < FFT_SIZE {
        return Vec::new();
    }
    let stft = Stft::compute(mono, sample_rate, FFT_SIZE, HOP);

    stft.magnitudes
        .iter()
        .enumerate()
        .map(|(t, frame)| {
            let t_ms = (t as f64 * HOP as f64 + FFT_SIZE as f64 / 2.0) / f64::from(sample_rate)
                * 1000.0;
            let mut mag_sum = 0.0f64;
            let mut weighted = 0.0f64;
            for (bin, &mag) in frame.iter().enumerate() {
                let m = f64::from(mag);
                mag_sum += m;
                weighted += m * stft.bin_hz(bin);
            }
            let hz = (mag_sum > SILENCE_MAG_FLOOR).then(|| weighted / mag_sum);
            CentroidPoint { t_ms, hz }
        })
        .collect()
}
