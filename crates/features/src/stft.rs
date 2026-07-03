//! STFT machinery shared by the `onsets` and `key` extractors: a
//! Hann-windowed magnitude spectrogram via `FftPlannerScalar` — never the
//! runtime-dispatching `FftPlanner`, which is clippy-banned because it
//! CPU-feature-dispatches per machine (`docs/determinism.md`'s rustfft
//! audit).
//!
//! Onsets and chroma want different tradeoffs from the same transform:
//! onsets need good *time* resolution (a short window), chroma needs good
//! *frequency* resolution at low pitches (a long window — see the `key`
//! module docs for why a short window smears a full octave's worth of
//! chroma at the bottom of the piano). So each extractor calls
//! [`Stft::compute`] with its own `fft_size`/`hop`, rather than sharing one
//! transform.

use rustfft::FftPlannerScalar;
use rustfft::num_complex::Complex;

/// A Hann-windowed magnitude spectrogram: one bin-magnitude vector of
/// length `fft_size / 2 + 1` per frame.
pub(crate) struct Stft {
    pub magnitudes: Vec<Vec<f32>>,
    pub sample_rate: u32,
    pub fft_size: usize,
}

impl Stft {
    /// Compute the spectrogram of `mono` at `sample_rate` with the given
    /// `fft_size`/`hop`. Empty (no frames) if `mono` is shorter than one
    /// FFT window — callers must handle that, not this constructor.
    pub fn compute(mono: &[f32], sample_rate: u32, fft_size: usize, hop: usize) -> Self {
        let window = hann_window(fft_size);
        let mut planner = FftPlannerScalar::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);
        let bins = fft_size / 2 + 1;

        let mut magnitudes = Vec::new();
        if mono.len() >= fft_size {
            let frame_count = (mono.len() - fft_size) / hop + 1;
            let mut buf = vec![Complex::new(0.0f32, 0.0f32); fft_size];
            for frame in 0..frame_count {
                let start = frame * hop;
                let src = &mono[start..start + fft_size];
                for ((slot, &sample), &w) in buf.iter_mut().zip(src.iter()).zip(window.iter()) {
                    *slot = Complex::new(sample * w, 0.0);
                }
                fft.process(&mut buf);
                magnitudes.push(
                    buf[..bins]
                        .iter()
                        .map(|c| (c.re * c.re + c.im * c.im).sqrt())
                        .collect(),
                );
            }
        }

        Self {
            magnitudes,
            sample_rate,
            fft_size,
        }
    }

    /// Center frequency of `bin`, in Hz.
    pub fn bin_hz(&self, bin: usize) -> f64 {
        bin as f64 * f64::from(self.sample_rate) / self.fft_size as f64
    }
}

/// Periodic Hann window, `w[n] = 0.5 - 0.5 * cos(2*pi*n / (N-1))`, computed
/// with `libm::cosf` per `docs/determinism.md` (std `f32::cos` is
/// clippy-banned).
fn hann_window(n: usize) -> Vec<f32> {
    if n <= 1 {
        return vec![1.0; n];
    }
    let denom = (n - 1) as f32;
    (0..n)
        .map(|i| {
            let phase = 2.0 * std::f32::consts::PI * i as f32 / denom;
            0.5 - 0.5 * libm::cosf(phase)
        })
        .collect()
}
