//! Timbre identity as a compact MFCC digest: mean and spread of the first
//! [`NUM_COEFFS`] mel-frequency cepstral coefficients over the whole
//! buffer. Two clips with the same notes at the same loudness but a
//! different *sound* (sine vs saw, pad vs pluck) separate here when every
//! other report section reads the same — the "did the re-render keep the
//! instrument's character" axis.
//!
//! Method (textbook MFCC, hand-rolled so `features` stays free of the
//! spectro crate and its `image` dependency): 2048/512 Hann STFT →
//! [`NUM_FILTERS`] triangular mel filters (HTK formula
//! `mel = 2595*log10(1 + f/700)`, matching `cochlea-spectro`'s documented
//! choice; peak-normalized triangles) over the *power* spectrum → natural
//! log with a floor → orthonormal DCT-II, keeping coefficients
//! `0..NUM_COEFFS`. Per-coefficient mean and population standard deviation
//! over all frames are the digest; the full per-frame matrix is
//! deliberately not reported (it's spectrogram-sized).
//!
//! `c0` is (log-)energy — a loudness proxy, kept because it is part of the
//! standard vector, but distance measures that want pure spectral shape
//! should skip it (the compare module does; see
//! [`crate::compare::TimbreDelta`]).

use serde::{Deserialize, Serialize};

use crate::stft::Stft;

/// Number of triangular mel filters.
const NUM_FILTERS: usize = 26;
/// Number of cepstral coefficients kept (`c0..c12`).
pub(crate) const NUM_COEFFS: usize = 13;
/// STFT analysis window, samples.
const FFT_SIZE: usize = 2048;
/// STFT hop, samples.
const HOP: usize = 512;
/// Lowest frequency covered by the filterbank, Hz.
const FMIN_HZ: f64 = 20.0;
/// The filterbank's upper edge is `min(FMAX_HZ, Nyquist)` — fixed rather
/// than rate-relative, so the same material at 44.1 kHz and 48 kHz lands
/// on (nearly) the same coefficients instead of stretching the filterbank
/// with the container's sample rate.
const FMAX_HZ: f64 = 16_000.0;
/// Absolute floor inside the log, on power-spectrum filter energies.
const LOG_FLOOR: f64 = 1e-10;
/// Per-frame dynamic-range floor: each frame's filter energies are floored
/// at this fraction of the frame's loudest filter (80 dB down) before the
/// log. Without it, filters holding no real signal fluctuate around the
/// absolute floor with spectral leakage, and their log-domain jitter
/// dominates `mfcc_std` for spectrally sparse input (a pure sine read a
/// *larger* timbre spread than a saw — measured, not hypothetical).
const FRAME_DYNAMIC_RANGE: f64 = 1e-8;

/// Compact timbre digest — see the module docs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimbreReport {
    /// Per-coefficient mean over all frames, `mfcc_mean[0] = c0` (log
    /// energy) through `c12`.
    pub mfcc_mean: Vec<f64>,
    /// Per-coefficient population standard deviation over all frames —
    /// how much the timbre *moves* (a static pad reads near zero, an
    /// evolving sweep doesn't).
    pub mfcc_std: Vec<f64>,
    /// Number of STFT frames the digest covers.
    pub frames: usize,
}

/// Compute the timbre digest of `mono`. `None` when the buffer is shorter
/// than one analysis window (nothing to measure — mirrors the other
/// analyzers' degenerate conventions).
pub(crate) fn analyze(mono: &[f32], sample_rate: u32) -> Option<TimbreReport> {
    if mono.len() < FFT_SIZE || sample_rate == 0 {
        return None;
    }
    let stft = Stft::compute(mono, sample_rate, FFT_SIZE, HOP);
    if stft.magnitudes.is_empty() {
        return None;
    }

    let filters = mel_filterbank(sample_rate);
    let frames = stft.magnitudes.len();

    let mut mean = [0.0f64; NUM_COEFFS];
    let mut sq_mean = [0.0f64; NUM_COEFFS];
    for mags in &stft.magnitudes {
        // Power-spectrum filter energies, then log with a per-frame
        // dynamic-range floor (see FRAME_DYNAMIC_RANGE).
        let energies: Vec<f64> = filters
            .iter()
            .map(|filter| {
                filter
                    .iter()
                    .map(|&(bin, w)| {
                        let m = f64::from(mags[bin]);
                        m * m * w
                    })
                    .sum()
            })
            .collect();
        let frame_max = energies.iter().copied().fold(0.0f64, f64::max);
        let floor = (frame_max * FRAME_DYNAMIC_RANGE).max(LOG_FLOOR);
        let log_energies: Vec<f64> = energies.iter().map(|&e| libm::log(e.max(floor))).collect();
        let coeffs = dct_ii(&log_energies);
        for (k, &c) in coeffs.iter().enumerate() {
            mean[k] += c;
            sq_mean[k] += c * c;
        }
    }

    let n = frames as f64;
    let mut std = vec![0.0f64; NUM_COEFFS];
    for k in 0..NUM_COEFFS {
        mean[k] /= n;
        // Population variance; clamp tiny negative rounding residue.
        let var = (sq_mean[k] / n - mean[k] * mean[k]).max(0.0);
        std[k] = libm::sqrt(var);
    }

    Some(TimbreReport {
        mfcc_mean: mean.to_vec(),
        mfcc_std: std,
        frames,
    })
}

/// HTK mel of `hz`.
fn hz_to_mel(hz: f64) -> f64 {
    2595.0 * libm::log10(1.0 + hz / 700.0)
}

/// Inverse of [`hz_to_mel`].
fn mel_to_hz(mel: f64) -> f64 {
    700.0 * (libm::pow(10.0, mel / 2595.0) - 1.0)
}

/// The filterbank as sparse `(bin, weight)` lists — one per filter,
/// peak-normalized triangles over `NUM_FILTERS + 2` equally-mel-spaced
/// edge points between [`FMIN_HZ`] and `min(FMAX_HZ, Nyquist)`.
fn mel_filterbank(sample_rate: u32) -> Vec<Vec<(usize, f64)>> {
    let nyquist = f64::from(sample_rate) / 2.0;
    let fmax = FMAX_HZ.min(nyquist);
    let bins = FFT_SIZE / 2 + 1;
    let hz_per_bin = f64::from(sample_rate) / FFT_SIZE as f64;

    let mel_lo = hz_to_mel(FMIN_HZ.min(fmax));
    let mel_hi = hz_to_mel(fmax);
    let edges: Vec<f64> = (0..NUM_FILTERS + 2)
        .map(|i| mel_to_hz(mel_lo + (mel_hi - mel_lo) * i as f64 / (NUM_FILTERS + 1) as f64))
        .collect();

    (0..NUM_FILTERS)
        .map(|f| {
            let (lo, center, hi) = (edges[f], edges[f + 1], edges[f + 2]);
            let mut filter = Vec::new();
            for bin in 0..bins {
                let hz = bin as f64 * hz_per_bin;
                let w = if hz <= lo || hz >= hi {
                    0.0
                } else if hz <= center {
                    (hz - lo) / (center - lo)
                } else {
                    (hi - hz) / (hi - center)
                };
                if w > 0.0 {
                    filter.push((bin, w));
                }
            }
            filter
        })
        .collect()
}

/// Orthonormal DCT-II of `input`, truncated to [`NUM_COEFFS`] outputs:
/// `c[k] = s(k) * sum_j input[j] * cos(pi/N * (j + 0.5) * k)` with
/// `s(0) = sqrt(1/N)`, `s(k>0) = sqrt(2/N)`.
fn dct_ii(input: &[f64]) -> Vec<f64> {
    let n = input.len();
    let scale0 = libm::sqrt(1.0 / n as f64);
    let scale = libm::sqrt(2.0 / n as f64);
    (0..NUM_COEFFS.min(n))
        .map(|k| {
            let sum: f64 = input
                .iter()
                .enumerate()
                .map(|(j, &x)| {
                    x * libm::cos(std::f64::consts::PI / n as f64 * (j as f64 + 0.5) * k as f64)
                })
                .sum();
            if k == 0 { scale0 * sum } else { scale * sum }
        })
        .collect()
}
