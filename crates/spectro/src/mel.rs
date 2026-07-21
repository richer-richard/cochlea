//! STFT -> hand-rolled HTK mel filterbank -> log-magnitude dB matrix.
//!
//! ## Mel formula: HTK, not Slaney (documented choice)
//!
//! `mel = 2595 * log10(1 + f/700)`, inverse `f = 700 * (10^(mel/2595) - 1)`.
//! This is the HTK formula. The alternative, Slaney's (piecewise
//! linear-below-1kHz, log-above, with area-normalized filter weights so
//! each band integrates to the same energy for equal-loudness noise), was
//! considered and rejected for this crate: HTK is a single closed-form
//! curve (simpler to hand-roll and to reason about at the edges — no
//! `f == 1000.0` branch), and this crate consumes its own mel axis (there's
//! no cross-library mel-spectrogram compatibility requirement to satisfy).
//! Filters here are HTK-style peak-normalized triangles (weight `1.0`
//! exactly at each band's center bin) rather than Slaney's
//! area-normalized ones. Pick one, document it, never mix: this file is the
//! single source of truth for both the formula and the normalization.
//!
//! ## Framing (documented choice)
//!
//! Frame `i` covers samples `[i*hop, i*hop+fft)` of the mono downmix — no
//! centering, no reflection padding. Out-of-range indices (past the end of
//! the signal) read as `0.0`. Enough hop-sized frames are appended past the
//! last frame that touches real audio (`ceil(fft/hop)` of them) that the
//! *last* frames of every [`MelSpec`] are guaranteed to be pure zero-padding
//! — deterministically at `floor_db`, not an incidental edge effect.

use crate::opts::SpectroOpts;
use rustfft::FftPlannerScalar;
use rustfft::num_complex::Complex;
use std::f32::consts::PI;

/// A `[mels x frames]` log-magnitude (dB) mel spectrogram: mel band `0` is
/// the lowest frequency, frame `0` is earliest in time. Row-major:
/// `mel_db[mel * frames + frame]`.
#[derive(Debug, Clone, PartialEq)]
pub struct MelSpec {
    pub mel_db: Vec<f32>,
    pub mels: usize,
    pub frames: usize,
    pub sample_rate: u32,
    pub hop: usize,
    pub floor_db: f32,
    /// Lowest frequency covered by the filterbank, Hz (band 0's low edge).
    pub fmin: f32,
    /// Highest frequency covered, Hz (the resolved, Nyquist-capped value).
    pub fmax: f32,
}

impl MelSpec {
    /// Log-magnitude in dB at `(mel, frame)`.
    ///
    /// # Panics
    /// Panics if `mel >= self.mels` or `frame >= self.frames`.
    pub fn get(&self, mel: usize, frame: usize) -> f32 {
        assert!(
            mel < self.mels,
            "mel {mel} out of range (mels = {})",
            self.mels
        );
        assert!(
            frame < self.frames,
            "frame {frame} out of range (frames = {})",
            self.frames
        );
        self.mel_db[mel * self.frames + frame]
    }

    /// Sample offset of the start of `frame` (see the module doc on framing).
    pub fn frame_sample(&self, frame: usize) -> u64 {
        frame as u64 * self.hop as u64
    }

    /// The mel band whose center is nearest `hz` on this spectrogram's
    /// axis, `None` when `hz` falls outside `[fmin, fmax]` (or isn't
    /// finite). Backs the pitch-track overlay in
    /// [`render_annotated`](crate::render_annotated) — the same HTK mel
    /// mapping the filterbank uses, so an overlaid f0 lands on the band
    /// that actually holds its energy.
    pub fn hz_band(&self, hz: f64) -> Option<usize> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "audio-range Hz fits f32 exactly enough for a pixel-row lookup"
        )]
        let hz = hz as f32;
        if !hz.is_finite() || hz < self.fmin || hz > self.fmax || self.mels == 0 {
            return None;
        }
        let mel_min = hz_to_mel(self.fmin);
        let mel_max = hz_to_mel(self.fmax);
        if mel_max <= mel_min {
            return None;
        }
        let pos = (hz_to_mel(hz) - mel_min) / (mel_max - mel_min) * self.mels as f32;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "pos is in [0, mels] by the range checks above"
        )]
        Some((pos as usize).min(self.mels - 1))
    }

    /// Nearest frame index to a given sample offset, clamped to the last
    /// valid frame.
    pub fn sample_frame(&self, sample: u64) -> usize {
        let hop = self.hop.max(1) as u64;
        let f = (sample + hop / 2) / hop;
        (f as usize).min(self.frames.saturating_sub(1))
    }

    /// A new `MelSpec` covering only frames `[start, end)` of `self`,
    /// re-numbered starting at frame 0. Used by `contact_sheet` to slice
    /// tiles; kept crate-private since callers can always re-slice from the
    /// full analysis instead.
    pub(crate) fn sub(&self, start: usize, end: usize) -> MelSpec {
        let end = end.min(self.frames);
        let start = start.min(end);
        let frames = end - start;
        let mut mel_db = vec![0.0f32; self.mels * frames];
        for mel in 0..self.mels {
            for (j, frame) in (start..end).enumerate() {
                mel_db[mel * frames + j] = self.get(mel, frame);
            }
        }
        MelSpec {
            mel_db,
            mels: self.mels,
            frames,
            sample_rate: self.sample_rate,
            hop: self.hop,
            floor_db: self.floor_db,
            fmin: self.fmin,
            fmax: self.fmax,
        }
    }
}

/// Compute a mel spectrogram: downmix to mono, windowed STFT (rustfft's
/// `FftPlannerScalar` — never the runtime-dispatching `FftPlanner`, banned
/// via `clippy.toml`), magnitude spectrum, hand-rolled HTK-style mel
/// filterbank (see module docs), log magnitude in dB floored at
/// `opts.floor_db`.
///
/// `samples` is interleaved PCM with `channels` channels; downmix is the
/// fixed-order average `(c0 + c1 + ... ) / channels` (mono is a no-op copy).
///
/// # Panics
/// Panics if `channels == 0`, `sample_rate == 0`, `samples.len()` is not a
/// multiple of `channels`, or the resolved `fmax` (capped at Nyquist) does
/// not exceed `opts.fmin` (e.g. `fmin` set above the given `sample_rate`'s
/// Nyquist).
pub fn mel_spectrogram(
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    opts: &SpectroOpts,
) -> MelSpec {
    assert!(channels > 0, "channels must be nonzero");
    assert!(sample_rate > 0, "sample_rate must be nonzero");
    assert_eq!(
        samples.len() % channels as usize,
        0,
        "samples length ({}) must be a multiple of channels ({channels})",
        samples.len()
    );

    let mono = downmix(samples, channels);

    let fft_len = opts.fft;
    let hop = opts.hop;
    let window = hann_window(fft_len);

    // See module doc: enough trailing frames are appended that the last
    // ones are guaranteed pure zero-padding.
    let real_frames = mono.len().div_ceil(hop).max(1);
    let tail_frames = fft_len.div_ceil(hop);
    let frames = real_frames + tail_frames;

    let fmax = opts.effective_fmax(sample_rate);
    assert!(
        fmax > opts.fmin,
        "effective fmax ({fmax}) must exceed fmin ({}) at sample_rate {sample_rate}",
        opts.fmin
    );
    let filterbank = mel_filterbank(opts.mels, fft_len, sample_rate, opts.fmin, fmax);
    let n_bins = fft_len / 2 + 1;

    let mut planner = FftPlannerScalar::<f32>::new();
    let fft = planner.plan_fft_forward(fft_len);

    let mut mel_db = vec![0.0f32; opts.mels * frames];
    let mut buffer = vec![
        Complex {
            re: 0.0f32,
            im: 0.0f32
        };
        fft_len
    ];
    let mut magnitude = vec![0.0f32; n_bins];

    for frame_idx in 0..frames {
        let start = frame_idx * hop;
        for (n, w) in window.iter().enumerate() {
            let sample = mono.get(start + n).copied().unwrap_or(0.0);
            buffer[n] = Complex {
                re: sample * w,
                im: 0.0,
            };
        }
        fft.process(&mut buffer);
        for (k, m) in magnitude.iter_mut().enumerate() {
            let c = buffer[k];
            // std sqrt is exempt from the transcendental ban (IEEE-exact,
            // hardware instruction everywhere) — see clippy.toml.
            *m = (c.re * c.re + c.im * c.im).sqrt();
        }
        for (mel, row) in filterbank.iter().enumerate() {
            let mut energy = 0.0f32;
            for &(bin, weight) in row {
                energy += weight * magnitude[bin];
            }
            let db = if energy > 0.0 {
                (20.0 * libm::log10f(energy)).max(opts.floor_db)
            } else {
                opts.floor_db
            };
            mel_db[mel * frames + frame_idx] = db;
        }
    }

    MelSpec {
        mel_db,
        mels: opts.mels,
        frames,
        sample_rate,
        hop,
        floor_db: opts.floor_db,
        fmin: opts.fmin,
        fmax,
    }
}

/// Fixed-order downmix `(c0 + c1 + ... + c_{n-1}) / n` per frame — same
/// convention as the workspace-wide mono-downmix rule in `docs/plan.md`
/// (`(l + r) / 2` for stereo, generalized to N channels).
fn downmix(samples: &[f32], channels: u16) -> Vec<f32> {
    let channels = channels as usize;
    if channels == 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Periodic (DFT-even) Hann window: `0.5 - 0.5*cos(2*pi*n/fft)`. See module
/// doc for why periodic (not symmetric) was chosen.
fn hann_window(fft_len: usize) -> Vec<f32> {
    if fft_len <= 1 {
        return vec![1.0; fft_len];
    }
    let n = fft_len as f32;
    (0..fft_len)
        .map(|i| 0.5 - 0.5 * libm::cosf(2.0 * PI * i as f32 / n))
        .collect()
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * libm::log10f(1.0 + hz / 700.0)
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (libm::powf(10.0, mel / 2595.0) - 1.0)
}

/// Sparse HTK-style triangular mel filterbank: for each of `mels` bands,
/// the list of `(fft_bin, weight)` pairs with nonzero overlap. See the
/// module doc for the HTK-vs-Slaney choice.
fn mel_filterbank(
    mels: usize,
    fft_len: usize,
    sample_rate: u32,
    fmin: f32,
    fmax: f32,
) -> Vec<Vec<(usize, f32)>> {
    let n_bins = fft_len / 2 + 1;
    let mel_min = hz_to_mel(fmin);
    let mel_max = hz_to_mel(fmax);

    // mels + 2 boundary points -> mels triangular filters: filter m uses
    // (left = point[m], center = point[m+1], right = point[m+2]).
    let denom = (mels + 1) as f32;
    let mel_points: Vec<f32> = (0..mels + 2)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / denom)
        .collect();
    let bin_points: Vec<usize> = mel_points
        .iter()
        .map(|&mel| {
            let hz = mel_to_hz(mel);
            let bin = ((fft_len + 1) as f32 * hz / sample_rate as f32).floor();
            (bin.max(0.0) as usize).min(n_bins - 1)
        })
        .collect();

    (0..mels)
        .map(|m| {
            let left = bin_points[m];
            let center = bin_points[m + 1];
            let right = bin_points[m + 2];
            let mut row = Vec::new();
            if center > left {
                for bin in left..center {
                    let w = (bin - left) as f32 / (center - left) as f32;
                    if w > 0.0 {
                        row.push((bin, w));
                    }
                }
            }
            // Center bin always gets peak weight 1.0 (HTK convention).
            row.push((center, 1.0));
            if right > center {
                for bin in (center + 1)..=right {
                    let w = (right - bin) as f32 / (right - center) as f32;
                    if w > 0.0 {
                        row.push((bin, w));
                    }
                }
            }
            row
        })
        .collect()
}

/// The Hz at the center of mel band `m` for a filterbank built with
/// `mel_filterbank`'s equal-mel-spacing rule — used by tests to predict
/// which band a pure tone should land in without duplicating the filter
/// construction itself.
#[cfg(test)]
fn mel_band_center_hz(m: usize, mels: usize, fmin: f32, fmax: f32) -> f32 {
    let mel_min = hz_to_mel(fmin);
    let mel_max = hz_to_mel(fmax);
    let denom = (mels + 1) as f32;
    let center_mel = mel_min + (mel_max - mel_min) * (m + 1) as f32 / denom;
    mel_to_hz(center_mel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sample_rate: u32, num_samples: usize, amp: f32) -> Vec<f32> {
        (0..num_samples)
            .map(|n| amp * libm::sinf(2.0 * PI * freq * n as f32 / sample_rate as f32))
            .collect()
    }

    fn chirp(f0: f32, f1: f32, sample_rate: u32, num_samples: usize, amp: f32) -> Vec<f32> {
        let sr = sample_rate as f32;
        let t_total = num_samples as f32 / sr;
        (0..num_samples)
            .map(|n| {
                let t = n as f32 / sr;
                // Instantaneous phase for a linear chirp: 2*pi*(f0*t + (f1-f0)/(2*T)*t^2).
                let phase = 2.0 * PI * (f0 * t + (f1 - f0) / (2.0 * t_total) * t * t);
                amp * libm::sinf(phase)
            })
            .collect()
    }

    #[test]
    fn sine_energy_concentrates_near_its_frequency() {
        let sr = 48_000u32;
        let samples = sine(1000.0, sr, sr as usize, 0.8);
        let opts = SpectroOpts::new();
        let spec = mel_spectrogram(&samples, 1, sr, &opts);

        // Frame 10 (start sample 5120) is well inside the steady-state
        // signal for the default fft=2048/hop=512.
        let frame = 10;
        let (argmax_mel, _) = (0..spec.mels)
            .map(|m| (m, spec.get(m, frame)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).expect("no NaNs in a dB spectrum"))
            .expect("mels > 0");

        let fmax = opts.effective_fmax(sr);
        let center_hz = mel_band_center_hz(argmax_mel, spec.mels, opts.fmin, fmax);
        assert!(
            (center_hz - 1000.0).abs() < 150.0,
            "argmax mel band {argmax_mel} centers at {center_hz} Hz, expected close to 1000 Hz"
        );
    }

    #[test]
    fn frames_past_the_signal_are_at_floor() {
        let sr = 48_000u32;
        let samples = sine(1000.0, sr, sr as usize, 0.8);
        let opts = SpectroOpts::new();
        let spec = mel_spectrogram(&samples, 1, sr, &opts);

        // By construction (see module doc), the last frame is guaranteed
        // pure zero-padding.
        let last = spec.frames - 1;
        for mel in 0..spec.mels {
            assert_eq!(
                spec.get(mel, last),
                spec.floor_db,
                "mel {mel} at trailing frame {last} should be at floor"
            );
        }
    }

    #[test]
    fn chirp_argmax_rises_monotonically() {
        let sr = 48_000u32;
        let num_samples = sr as usize * 2; // 2 s sweep, 100 Hz -> 8 kHz
        let samples = chirp(100.0, 8000.0, sr, num_samples, 0.8);
        let opts = SpectroOpts::new();
        let spec = mel_spectrogram(&samples, 1, sr, &opts);

        let real_frames = num_samples.div_ceil(opts.hop).max(1);
        // Coarse checkpoints across the real (non-padding) region, spaced
        // out to be robust to local spectral-leakage jitter.
        let checkpoints: Vec<usize> = (0..real_frames).step_by(15).collect();

        let argmax_at = |frame: usize| -> usize {
            (0..spec.mels)
                .map(|m| (m, spec.get(m, frame)))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .unwrap()
                .0
        };

        let bins: Vec<usize> = checkpoints.iter().map(|&f| argmax_at(f)).collect();
        for w in bins.windows(2) {
            assert!(
                w[1] >= w[0],
                "argmax bin should not fall as the chirp rises: {bins:?}"
            );
        }
        assert!(
            *bins.last().unwrap() > *bins.first().unwrap(),
            "argmax bin should rise overall: {bins:?}"
        );
    }

    #[test]
    fn mel_spectrogram_is_deterministic() {
        let sr = 48_000u32;
        let samples = sine(1000.0, sr, 4096, 0.8);
        let opts = SpectroOpts::new().fft(512).hop(128).mels(32);
        let a = mel_spectrogram(&samples, 1, sr, &opts);
        let b = mel_spectrogram(&samples, 1, sr, &opts);

        assert_eq!(a.mel_db.len(), b.mel_db.len());
        for (x, y) in a.mel_db.iter().zip(b.mel_db.iter()) {
            assert_eq!(x.to_bits(), y.to_bits(), "bitwise mismatch: {x} vs {y}");
        }
    }

    #[test]
    fn stereo_downmix_matches_mono_average() {
        let sr = 8_000u32;
        let mono = sine(440.0, sr, 200, 0.5);
        let stereo: Vec<f32> = mono.iter().flat_map(|&s| [s, s]).collect();
        assert_eq!(downmix(&stereo, 2), mono);
    }
}
