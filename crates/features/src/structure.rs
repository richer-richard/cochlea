//! Structure detection: Foote-style self-similarity novelty segmentation
//! (Foote, "Automatic Audio Segmentation Using a Measure of Audio
//! Novelty", 2000 — implemented from the paper's publicly described
//! method, not from any GPL codebase). Finds unlabeled section boundaries
//! — verse/chorus-style structural changes — without any notion of what a
//! "verse" or "chorus" is, just where the music's character changes.
//!
//! Pipeline: per-frame (default 1 s) feature vectors — 12-bin chroma +
//! 3-band spectral energy + RMS, see [`feature_vectors`] — an all-pairs
//! cosine self-similarity matrix (see [`similarity_matrix`]) — a
//! Gaussian-tapered checkerboard kernel convolved along the matrix
//! diagonal (Foote's novelty curve: high where "before" and "after" a
//! point look different from each other but each side looks internally
//! consistent; see [`novelty_curve`]) — adaptive-threshold peak-picking
//! with a minimum inter-boundary gap, capped at `max_sections - 1`
//! boundaries keeping the strongest (see [`pick_boundaries`]).
//!
//! Self-contained: this module reimplements its own small STFT-bucketing
//! and adaptive-threshold-peak-picking helpers (mirroring `key`'s chroma
//! formula and `onsets`' median/MAD threshold in spirit) rather than
//! reaching into sibling modules, so it has no cross-module coupling
//! beyond the always-shared [`crate::stft::Stft`].

use serde::{Deserialize, Serialize};

use crate::audio::Audio;
// median/MAD and non-maximum suppression come from `onsets` — the novelty
// curve is thresholded and peak-picked with exactly the machinery the
// onset flux uses, one implementation instead of drift-prone copies.
use crate::onsets::{mad_of, median_of, suppress_close_peaks};
use crate::stft::Stft;

/// STFT size for the per-frame chroma/band-energy pass, samples. Smaller
/// than `key`'s 8192 (this module compares independent ~1 s frames
/// against each other, not one whole-file chroma vector, so it doesn't
/// need `key`'s low-octave frequency resolution as badly) but still
/// ample: ~85 ms / ~11.7 Hz per bin at 48 kHz.
const FFT_SIZE: usize = 4096;
/// Hop size, samples (75% overlap at `FFT_SIZE`).
const HOP: usize = 1024;

/// Chroma bin range, Hz — matches `key`'s own choice, for consistency:
/// below is mostly sub-bass/DC leakage, above is thin harmonic content
/// that mostly adds chroma noise.
const CHROMA_MIN_HZ: f64 = 55.0;
const CHROMA_MAX_HZ: f64 = 5000.0;

/// Band split, Hz — matches `segments`' own choice, for consistency.
const LOW_HIGH_HZ: f64 = 250.0;
const MID_HIGH_HZ: f64 = 4000.0;

/// Feature-vector dimensionality: 12-bin chroma + 3-band energy + 1 RMS.
const FEATURE_DIMS: usize = 16;

/// Checkerboard kernel half-width, frames — the full kernel is `2 *
/// KERNEL_HALF_WIDTH` frames wide. At the default 1 s `frame_ms` that's a
/// 16 s window, matched against roughly minute-scale song sections.
const KERNEL_HALF_WIDTH: usize = 8;
/// Gaussian taper width for the checkerboard kernel, frames — tuned so
/// the kernel's weight has meaningfully decayed by its edges without
/// being so tight it collapses to a near-instantaneous (noisy)
/// comparison.
const KERNEL_SIGMA: f64 = 4.0;

/// Minimum gap between accepted boundaries, seconds — avoids reporting a
/// cluster of adjacent peaks around one real transition as several
/// separate sections.
const MIN_BOUNDARY_GAP_S: f64 = 4.0;
/// Adaptive novelty threshold: `median + NOVELTY_THRESHOLD_SCALE * MAD`.
/// Tuned empirically against this module's A/B and A/B/A fixtures (see
/// `tests/structure.rs`).
const NOVELTY_THRESHOLD_SCALE: f64 = 1.5;

/// Tunables for [`detect_structure`]. Mirrors [`crate::SegmentOpts`]'s
/// chainable-setter style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StructureOpts {
    /// Frame length for the self-similarity analysis, milliseconds.
    /// Default `1000.0` — beat-agnostic, one frame per second of
    /// programme.
    pub frame_ms: f64,
    /// Maximum sections to report (`boundaries_ms.len() + 1` for
    /// non-empty audio); the strongest `max_sections - 1` novelty peaks
    /// win if more candidates clear the threshold. Default `12` —
    /// generous for a typical song's intro/verse/chorus/bridge/outro
    /// structure without being effectively unbounded.
    pub max_sections: usize,
}

impl Default for StructureOpts {
    fn default() -> Self {
        Self {
            frame_ms: 1000.0,
            max_sections: 12,
        }
    }
}

impl StructureOpts {
    /// Override the frame length, milliseconds.
    #[must_use]
    pub fn with_frame_ms(mut self, frame_ms: f64) -> Self {
        self.frame_ms = frame_ms;
        self
    }

    /// Override the maximum section count.
    #[must_use]
    pub fn with_max_sections(mut self, max_sections: usize) -> Self {
        self.max_sections = max_sections;
        self
    }
}

/// Structure-detection result. Plain struct, no own schema version —
/// parallel API, meant to be embedded into a future `Report` schema bump
/// rather than stand alone (mirrors [`crate::TempoReport`]'s status).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructureReport {
    /// Section boundary times, milliseconds, ascending. Excludes `0.0` —
    /// the first section always starts at the top of the file, so a
    /// boundary there would be redundant.
    pub boundaries_ms: Vec<f64>,
    /// `boundaries_ms.len() + 1` for non-empty, long-enough-to-analyze
    /// audio (every boundary splits one more section off); `0` only for
    /// genuinely empty/invalid input. See [`detect_structure`] for the
    /// empty-vs-too-short distinction.
    pub section_count: usize,
    /// Confidence in the detected boundaries, `0.0..=1.0`. `0.0` when
    /// `boundaries_ms` is empty (nothing to be confident about). See
    /// [`detect_structure`] for the exact definition.
    pub confidence: f64,
}

fn empty_report() -> StructureReport {
    StructureReport {
        boundaries_ms: Vec::new(),
        section_count: 0,
        confidence: 0.0,
    }
}

fn one_section_report() -> StructureReport {
    StructureReport {
        boundaries_ms: Vec::new(),
        section_count: 1,
        confidence: 0.0,
    }
}

/// Detect structural section boundaries in `audio` (see the module docs
/// for the pipeline).
///
/// **`confidence`**: `mean(picked_peak_novelty - median_novelty) /
/// (max_novelty - median_novelty)`, clamped to `0.0..=1.0` — how prominent
/// the accepted peaks are, relative to the single most novel point
/// anywhere in the curve. A peak as prominent as the curve's global
/// maximum scores confidence near `1.0`; a peak that barely cleared the
/// adaptive threshold scores near `0.0`. `0.0` whenever no boundaries were
/// picked (including all degenerate cases).
///
/// Degenerate input: empty audio (or a zero sample rate, or a `frame_ms`
/// that is non-finite or under 1 ms) returns `section_count: 0` — there's
/// no audio to have even one section, or no sane frame grid to analyze it
/// on. Non-empty audio too short for one checkerboard-kernel
/// window (fewer than `2 * KERNEL_HALF_WIDTH` frames) returns
/// `section_count: 1` with no boundaries — there's audio, just not enough
/// of it to say anything about its internal structure. A homogeneous
/// buffer (a steady tone, or silence) naturally resolves to the same
/// `section_count: 1` outcome through the ordinary pipeline: a flat
/// self-similarity matrix produces a flat (all-zero) novelty curve, so no
/// peak ever clears the adaptive threshold. Neither case panics.
pub fn detect_structure(audio: &Audio, opts: &StructureOpts) -> StructureReport {
    let mono = audio.mono();
    // Same guard shape as `segment_timeline`'s window_ms: NaN slips past a
    // `<= 0.0` check (IEEE 754 comparisons with NaN are all false), then
    // saturates to a 1-sample frame and the O(n²) similarity matrix
    // explodes toward vec![vec![_; len]; len] — terabytes for real audio.
    // The 1 ms floor closes the tiny-positive route to the same blowup.
    if mono.is_empty()
        || audio.sample_rate == 0
        || !opts.frame_ms.is_finite()
        || opts.frame_ms < 1.0
    {
        return empty_report();
    }

    let sample_rate = audio.sample_rate;
    let frame_len = ((opts.frame_ms / 1000.0 * f64::from(sample_rate)).round() as usize).max(1);
    let frame_count = mono.len().div_ceil(frame_len);

    if frame_count < 2 * KERNEL_HALF_WIDTH {
        return one_section_report();
    }

    let features = feature_vectors(&mono, sample_rate, frame_len, frame_count, opts.frame_ms);
    let similarity = similarity_matrix(&features);
    let novelty = novelty_curve(&similarity, KERNEL_HALF_WIDTH, KERNEL_SIGMA);

    let min_gap_frames = ((MIN_BOUNDARY_GAP_S * 1000.0 / opts.frame_ms).ceil() as usize).max(1);
    let max_boundaries = opts.max_sections.max(1) - 1;
    let picked = pick_boundaries(&novelty, min_gap_frames, max_boundaries);

    if picked.is_empty() {
        return one_section_report();
    }

    let boundaries_ms: Vec<f64> = picked.iter().map(|&i| i as f64 * opts.frame_ms).collect();
    let confidence = confidence_of(&novelty, &picked);

    StructureReport {
        section_count: boundaries_ms.len() + 1,
        boundaries_ms,
        confidence,
    }
}

/// Per-frame 16-dim feature vectors: `[chroma(12), band_energy(3),
/// rms(1)]`. Chroma and band energy come from one full-file STFT (cheaper
/// than one STFT per frame), each frame's magnitude bins bucketed by the
/// frame their center time falls into — the same "single-pass, bucket by
/// time" scheme [`crate::segment_timeline`] uses for its own band energy.
/// Chroma is max-normalized (matches `key`'s convention); band energy is
/// normalized to fractions of the frame's total (matches `segments`'
/// convention); RMS is raw linear amplitude, already `0.0..=1.0` like the
/// other two, so no extra scaling is needed before cosine similarity.
fn feature_vectors(
    mono: &[f32],
    sample_rate: u32,
    frame_len: usize,
    frame_count: usize,
    frame_ms: f64,
) -> Vec<[f64; FEATURE_DIMS]> {
    let stft = Stft::compute(mono, sample_rate, FFT_SIZE, HOP);

    let mut chroma_acc = vec![[0.0f64; 12]; frame_count];
    let mut band_acc = vec![[0.0f64; 3]; frame_count];

    for (t, frame) in stft.magnitudes.iter().enumerate() {
        let center_ms =
            (t as f64 * HOP as f64 + FFT_SIZE as f64 / 2.0) / f64::from(sample_rate) * 1000.0;
        let idx = bucket_index(center_ms, frame_ms, frame_count);

        for (bin, &mag) in frame.iter().enumerate() {
            let hz = stft.bin_hz(bin);
            let mag = f64::from(mag);

            if (CHROMA_MIN_HZ..=CHROMA_MAX_HZ).contains(&hz) {
                let midi_float = 69.0 + 12.0 * libm::log2(hz / 440.0);
                let pitch_class = midi_float.round().rem_euclid(12.0) as usize;
                chroma_acc[idx][pitch_class] += mag;
            }

            let energy = mag * mag;
            let band = if hz < LOW_HIGH_HZ {
                0
            } else if hz <= MID_HIGH_HZ {
                1
            } else {
                2
            };
            band_acc[idx][band] += energy;
        }
    }

    (0..frame_count)
        .map(|i| {
            let start = i * frame_len;
            let end = (start + frame_len).min(mono.len());
            let slice = &mono[start..end];
            let rms = if slice.is_empty() {
                0.0
            } else {
                (slice
                    .iter()
                    .map(|&x| f64::from(x) * f64::from(x))
                    .sum::<f64>()
                    / slice.len() as f64)
                    .sqrt()
            };

            let mut chroma = chroma_acc[i];
            let max_chroma = chroma.iter().copied().fold(0.0f64, f64::max);
            if max_chroma > 0.0 {
                for v in &mut chroma {
                    *v /= max_chroma;
                }
            }

            let band = band_acc[i];
            let total_band = band[0] + band[1] + band[2];
            let band_frac = if total_band > 0.0 {
                [
                    band[0] / total_band,
                    band[1] / total_band,
                    band[2] / total_band,
                ]
            } else {
                [0.0, 0.0, 0.0]
            };

            let mut v = [0.0f64; FEATURE_DIMS];
            v[0..12].copy_from_slice(&chroma);
            v[12..15].copy_from_slice(&band_frac);
            v[15] = rms;
            v
        })
        .collect()
}

/// Map a time in milliseconds to a frame index by fixed-width bucketing,
/// clamped into `0..count`. Mirrors [`crate::segment_timeline`]'s own
/// bucketing helper (reimplemented locally — this module has no
/// cross-module dependency beyond [`crate::stft::Stft`]).
fn bucket_index(t_ms: f64, frame_ms: f64, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let idx = (t_ms / frame_ms).floor();
    if idx <= 0.0 {
        0
    } else {
        (idx as usize).min(count - 1)
    }
}

/// All-pairs cosine similarity, `-1.0..=1.0`, clamped defensively (the
/// formula is bounded there by Cauchy-Schwarz, but summation order can
/// overshoot by a hair in floating point). `0.0` — a neutral "undefined"
/// value, not a real similarity — wherever either frame's vector has zero
/// norm (a fully silent frame has an all-zero feature vector, so cosine
/// similarity against anything, including itself, is undefined).
fn similarity_matrix(features: &[[f64; FEATURE_DIMS]]) -> Vec<Vec<f64>> {
    let n = features.len();
    let norms: Vec<f64> = features
        .iter()
        .map(|v| v.iter().map(|x| x * x).sum::<f64>().sqrt())
        .collect();

    let mut sim = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        for j in 0..n {
            if norms[i] > 0.0 && norms[j] > 0.0 {
                let dot: f64 = features[i]
                    .iter()
                    .zip(features[j].iter())
                    .map(|(a, b)| a * b)
                    .sum();
                sim[i][j] = (dot / (norms[i] * norms[j])).clamp(-1.0, 1.0);
            }
        }
    }
    sim
}

/// Foote's novelty curve: a Gaussian-tapered checkerboard kernel convolved
/// along `sim`'s diagonal. At each position `i`, the kernel weights
/// `sim[i+u][i+v]` by `sign(u, v) * gaussian(u, v)` for offsets `u, v` in
/// `-half_width..half_width` — positive where `u` and `v` have the same
/// sign (the two "same-side" quadrants, roughly "does each side look like
/// itself") and negative where they differ (the two "cross" quadrants,
/// roughly "does before look like after") — so the curve spikes exactly
/// where two internally-consistent regions meet. Offsets that would index
/// outside `0..sim.len()` are skipped rather than padded, which naturally
/// (and correctly) tapers the achievable novelty near the very start/end
/// of the buffer, where a full kernel window doesn't fit.
fn novelty_curve(sim: &[Vec<f64>], half_width: usize, sigma: f64) -> Vec<f64> {
    let n = sim.len();
    let half = half_width as i64;

    (0..n)
        .map(|i| {
            let mut score = 0.0;
            for u in -half..half {
                for v in -half..half {
                    let a = i as i64 + u;
                    let b = i as i64 + v;
                    if a < 0 || b < 0 || a >= n as i64 || b >= n as i64 {
                        continue;
                    }
                    let sign = if (u >= 0) == (v >= 0) { 1.0 } else { -1.0 };
                    let gaussian = libm::exp(-((u * u + v * v) as f64) / (2.0 * sigma * sigma));
                    score += sign * gaussian * sim[a as usize][b as usize];
                }
            }
            score
        })
        .collect()
}

/// Adaptive-threshold peak-picking over the novelty curve (median + scaled
/// MAD, mirroring `onsets`' approach in spirit — reimplemented locally,
/// see the module docs), minimum-gap suppression, then a strongest-first
/// cap at `max_boundaries`, re-sorted back into chronological order.
fn pick_boundaries(novelty: &[f64], min_gap_frames: usize, max_boundaries: usize) -> Vec<usize> {
    let n = novelty.len();
    if n < 3 || max_boundaries == 0 {
        return Vec::new();
    }

    let med = median_of(novelty);
    let mad = mad_of(novelty, med);
    let threshold = med + NOVELTY_THRESHOLD_SCALE * mad;

    let mut peaks = Vec::new();
    for t in 1..n - 1 {
        if novelty[t] > threshold && novelty[t] >= novelty[t - 1] && novelty[t] > novelty[t + 1] {
            peaks.push(t);
        }
    }

    let mut chosen = suppress_close_peaks(&peaks, novelty, min_gap_frames);
    if chosen.len() > max_boundaries {
        chosen.sort_by(|&a, &b| novelty[b].total_cmp(&novelty[a]));
        chosen.truncate(max_boundaries);
        chosen.sort_unstable();
    }
    chosen
}

fn confidence_of(novelty: &[f64], picked: &[usize]) -> f64 {
    if picked.is_empty() {
        return 0.0;
    }
    let med = median_of(novelty);
    let max_n = novelty.iter().copied().fold(f64::MIN, f64::max);
    let range = max_n - med;
    if range <= 0.0 {
        return 0.0;
    }
    let mean_prominence = picked
        .iter()
        .map(|&i| (novelty[i] - med).max(0.0))
        .sum::<f64>()
        / picked.len() as f64;
    (mean_prominence / range).clamp(0.0, 1.0)
}
