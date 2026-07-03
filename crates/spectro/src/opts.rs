//! Analysis parameters for [`mel_spectrogram`](crate::mel_spectrogram).

/// STFT + mel-filterbank parameters.
///
/// Defaults (`SpectroOpts::new()`, mirrored by `Default`): 2048-sample FFT,
/// 512-sample hop (75% overlap), 128 mel bands, -80 dB floor, 20 Hz fmin.
/// `fmax` defaults to Nyquist (`sample_rate / 2`) and is *always* capped
/// there at analysis time (`mel_spectrogram` needs `sample_rate` to resolve
/// it, so it isn't known until then) even if set higher via [`Self::fmax`].
///
/// Chainable setters, mirroring the workspace's builder vocabulary
/// (`docs/plan.md`/`CLAUDE.md`): `SpectroOpts::new().fft(4096).mels(64)`.
#[derive(Debug, Clone, PartialEq)]
pub struct SpectroOpts {
    pub(crate) fft: usize,
    pub(crate) hop: usize,
    pub(crate) mels: usize,
    pub(crate) floor_db: f32,
    pub(crate) fmin: f32,
    pub(crate) fmax: Option<f32>,
}

impl Default for SpectroOpts {
    fn default() -> Self {
        Self::new()
    }
}

impl SpectroOpts {
    /// Defaults: `fft = 2048`, `hop = 512`, `mels = 128`, `floor_db =
    /// -80.0`, `fmin = 20.0`, `fmax` = Nyquist at analysis time.
    pub fn new() -> Self {
        Self {
            fft: 2048,
            hop: 512,
            mels: 128,
            floor_db: -80.0,
            fmin: 20.0,
            fmax: None,
        }
    }

    /// FFT / analysis window size in samples.
    ///
    /// # Panics
    /// Panics if `fft < 2`.
    pub fn fft(mut self, fft: usize) -> Self {
        assert!(fft >= 2, "fft size must be at least 2, got {fft}");
        self.fft = fft;
        self
    }

    /// Hop size in samples between successive STFT frames.
    ///
    /// # Panics
    /// Panics if `hop < 1`.
    pub fn hop(mut self, hop: usize) -> Self {
        assert!(hop >= 1, "hop must be at least 1, got {hop}");
        self.hop = hop;
        self
    }

    /// Number of mel filterbank bands.
    ///
    /// # Panics
    /// Panics if `mels < 1`.
    pub fn mels(mut self, mels: usize) -> Self {
        assert!(mels >= 1, "mels must be at least 1, got {mels}");
        self.mels = mels;
        self
    }

    /// dB floor: log-magnitude values are clamped at or above this value.
    ///
    /// # Panics
    /// Panics if `floor_db >= 0.0`.
    pub fn floor_db(mut self, floor_db: f32) -> Self {
        assert!(floor_db < 0.0, "floor_db must be negative, got {floor_db}");
        self.floor_db = floor_db;
        self
    }

    /// Lowest frequency (Hz) covered by the mel filterbank.
    ///
    /// # Panics
    /// Panics if `fmin < 0.0`.
    pub fn fmin(mut self, fmin: f32) -> Self {
        assert!(fmin >= 0.0, "fmin must be non-negative, got {fmin}");
        self.fmin = fmin;
        self
    }

    /// Highest frequency (Hz) covered by the mel filterbank. Always capped
    /// at Nyquist (`sample_rate / 2`) when the spectrogram is computed,
    /// regardless of what's set here.
    ///
    /// # Panics
    /// Panics if `fmax <= 0.0`.
    pub fn fmax(mut self, fmax: f32) -> Self {
        assert!(fmax > 0.0, "fmax must be positive, got {fmax}");
        self.fmax = Some(fmax);
        self
    }

    /// The `fmax` that will actually be used for a given `sample_rate`:
    /// the configured value (if any), capped at Nyquist, or Nyquist itself
    /// if unset.
    pub fn effective_fmax(&self, sample_rate: u32) -> f32 {
        let nyquist = sample_rate as f32 / 2.0;
        match self.fmax {
            Some(f) => f.min(nyquist),
            None => nyquist,
        }
    }
}
