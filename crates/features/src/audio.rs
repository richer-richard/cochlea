//! [`Audio`]: interleaved PCM plus channel/rate metadata, and the only way
//! into this crate from a WAV file on disk.

use std::path::Path;

/// Interleaved PCM in `[-1.0, 1.0]`-normalized `f32`, plus the metadata
/// needed to interpret it. The only audio representation this crate knows
/// about — no score, no synth, arbitrary WAVs in (docs/plan.md).
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    /// Interleaved samples: `frame * channels + channel`.
    pub samples: Vec<f32>,
    /// Channel count (1 = mono, 2 = stereo, ...).
    pub channels: u16,
    /// Samples per second, per channel.
    pub sample_rate: u32,
}

/// Failure reading a WAV file into [`Audio`].
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// Underlying WAV parse/IO failure.
    #[error(transparent)]
    Wav(#[from] hound::Error),
    /// A PCM encoding this crate doesn't decode. Supported: 8/16/24/32-bit
    /// integer PCM and 32-bit float.
    #[error("unsupported WAV format: {bits}-bit {format:?}")]
    UnsupportedFormat {
        /// Bits per sample, as reported by the WAV header.
        bits: u16,
        /// Integer vs. float sample encoding.
        format: hound::SampleFormat,
    },
}

impl Audio {
    /// Read a WAV file, converting to interleaved `f32` in `[-1.0, 1.0]`.
    ///
    /// Supports 8/16/24/32-bit integer PCM, normalized by the format's
    /// full-scale magnitude (e.g. 16-bit by `/32768.0`), and 32-bit float
    /// WAV (used as-is, already in `[-1.0, 1.0]` by convention).
    pub fn from_wav(path: &Path) -> Result<Self, AudioError> {
        let mut reader = hound::WavReader::open(path)?;
        let spec = reader.spec();

        let samples = match spec.sample_format {
            hound::SampleFormat::Float if spec.bits_per_sample == 32 => {
                reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?
            }
            hound::SampleFormat::Int if matches!(spec.bits_per_sample, 8 | 16 | 24 | 32) => {
                let full_scale = full_scale(spec.bits_per_sample);
                reader
                    .samples::<i32>()
                    .map(|sample| sample.map(|v| v as f32 / full_scale))
                    .collect::<Result<Vec<_>, _>>()?
            }
            format => {
                return Err(AudioError::UnsupportedFormat {
                    bits: spec.bits_per_sample,
                    format,
                });
            }
        };

        Ok(Self {
            samples,
            channels: spec.channels,
            sample_rate: spec.sample_rate,
        })
    }

    /// Downmix to mono: `(sum of channels) / channels`, sample by sample,
    /// fixed index order. Mono input is returned unchanged (cloned).
    pub fn mono(&self) -> Vec<f32> {
        let channels = self.channels.max(1) as usize;
        if channels == 1 {
            return self.samples.clone();
        }
        self.samples
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    }

    /// Frame count (samples per channel) — the unit `source.samples` is
    /// reported in ([`crate::Report`]), not the interleaved sample count.
    pub fn frames(&self) -> usize {
        let channels = self.channels.max(1) as usize;
        self.samples.len() / channels
    }

    /// Duration in milliseconds, `frames() / sample_rate * 1000`. `0.0` if
    /// `sample_rate` is `0`.
    pub fn duration_ms(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frames() as f64 / f64::from(self.sample_rate) * 1000.0
    }
}

/// Full-scale magnitude for `bits`-deep signed integer PCM: the divisor
/// that maps the format's most negative representable value to `-1.0`.
fn full_scale(bits: u16) -> f32 {
    match bits {
        8 => 128.0,
        16 => 32_768.0,
        24 => 8_388_608.0,
        32 => 2_147_483_648.0,
        // `from_wav` only reaches this for the four depths matched above.
        _ => unreachable!("unsupported bit depth {bits} should have been rejected earlier"),
    }
}
