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
    /// A float WAV containing NaN or ±infinity. Non-finite samples aren't
    /// audio, and letting them in poisons every downstream analyzer in
    /// quiet, contradictory ways (a NaN-corrupted buffer can read as
    /// "silent" while its peak is real) — rejected at the door instead.
    #[error("non-finite sample (NaN or infinity) at sample index {index}")]
    NonFiniteSample {
        /// Index into the interleaved sample stream of the first offender.
        index: usize,
    },
    /// The header declares a degenerate shape: zero channels, zero sample
    /// rate, or a sample count not divisible by the channel count. None of
    /// these is analyzable audio, and admitting one risks divide-by-zero or
    /// assertion panics downstream (the mel spectrogram and several analyzers
    /// assume a well-formed shape) — rejected at the door.
    #[error(
        "degenerate WAV shape: {channels} channels, {sample_rate} Hz, {samples} interleaved samples"
    )]
    DegenerateShape {
        /// Declared channel count.
        channels: u16,
        /// Declared sample rate, Hz.
        sample_rate: u32,
        /// Interleaved sample count actually read.
        samples: usize,
    },
}

impl Audio {
    /// Read a WAV file, converting to interleaved `f32` in `[-1.0, 1.0]`.
    ///
    /// Supports 8/16/24/32-bit integer PCM, normalized by the format's
    /// full-scale magnitude (e.g. 16-bit by `/32768.0`), and 32-bit float
    /// WAV (used as-is, already in `[-1.0, 1.0]` by convention — except
    /// that non-finite samples, which a float WAV can legally encode, are
    /// rejected with [`AudioError::NonFiniteSample`] rather than let loose
    /// on analyzers whose math silently misreads NaN as silence).
    pub fn from_wav(path: &Path) -> Result<Self, AudioError> {
        let mut reader = hound::WavReader::open(path)?;
        let spec = reader.spec();

        // Reject a degenerate header before reading samples: zero channels
        // would make the interleave stride (and several downstream analyzers)
        // divide by zero, and a zero sample rate has no time base.
        if spec.channels == 0 || spec.sample_rate == 0 {
            return Err(AudioError::DegenerateShape {
                channels: spec.channels,
                sample_rate: spec.sample_rate,
                samples: 0,
            });
        }

        let samples = match spec.sample_format {
            hound::SampleFormat::Float if spec.bits_per_sample == 32 => {
                let samples = reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?;
                if let Some(index) = samples.iter().position(|s| !s.is_finite()) {
                    return Err(AudioError::NonFiniteSample { index });
                }
                samples
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

        // A ragged read (interleaved length not a whole number of frames)
        // means a truncated or malformed data chunk — refuse rather than let
        // a partial final frame skew every per-frame analyzer.
        if !samples.len().is_multiple_of(usize::from(spec.channels)) {
            return Err(AudioError::DegenerateShape {
                channels: spec.channels,
                sample_rate: spec.sample_rate,
                samples: samples.len(),
            });
        }

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

    /// Cut the `[from_s, to_s)` window as a new buffer — the read path's
    /// zoom lens (`probe --from/--to`). Frame-exact: each boundary rounds
    /// to the nearest frame, and the cut is on frame boundaries so channel
    /// interleaving is preserved. Bounds are clamped to the buffer
    /// (`from_s` below zero reads as zero, `to_s = None` or past the end
    /// reads as the end); an inverted or empty window yields an empty
    /// buffer, not an error. Returns the windowed audio plus the exact
    /// start offset actually used, milliseconds — callers thread it into
    /// [`crate::ProbeOpts::with_start_ms`] so the report says where its
    /// times are anchored.
    pub fn window(&self, from_s: f64, to_s: Option<f64>) -> (Audio, f64) {
        let frames = self.frames();
        let sr = f64::from(self.sample_rate);
        let to_frame = |s: f64| -> usize {
            if self.sample_rate == 0 || !s.is_finite() || s <= 0.0 {
                return 0;
            }
            let f = libm::round(s * sr);
            if f >= frames as f64 {
                frames
            } else {
                f as usize
            }
        };
        let start = to_frame(from_s);
        let end = to_s.map_or(frames, to_frame).max(start);
        let channels = self.channels.max(1) as usize;
        let audio = Audio {
            samples: self.samples[start * channels..end * channels].to_vec(),
            channels: self.channels,
            sample_rate: self.sample_rate,
        };
        let start_ms = if self.sample_rate == 0 {
            0.0
        } else {
            start as f64 / sr * 1000.0
        };
        (audio, start_ms)
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
