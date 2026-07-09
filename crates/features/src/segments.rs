//! Windowed feature timeline: bucket a whole-file [`Audio`] analysis into
//! fixed-width windows so a caller can "seek" inside audio by index instead
//! of reading raw PCM. Reuses the onset detector and YIN pitch tracker
//! per-window; band energy is bucketed from a single full-file STFT (see
//! [`segment_timeline`]'s docs for the exact scheme).

use serde::{Deserialize, Serialize};

use crate::audio::Audio;
use crate::stft::Stft;
use crate::{onsets, pitch};

/// Schema version of [`SegmentTimeline`]'s JSON form.
pub const SEGMENTS_SCHEMA_VERSION: u32 = 1;

/// FFT size for the band-energy STFT, samples. Reuses the onset detector's
/// short-window tradeoff (good time resolution) since band energy is
/// bucketed per display window, not read at FFT resolution — see
/// [`segment_timeline`]'s docs.
const BAND_FFT_SIZE: usize = 1024;
/// Hop size for the band-energy STFT, samples (75% overlap at
/// `BAND_FFT_SIZE`).
const BAND_HOP: usize = 256;

/// Band split, Hz: low is `< LOW_HIGH_HZ`, mid is `[LOW_HIGH_HZ,
/// MID_HIGH_HZ]`, high is `> MID_HIGH_HZ`.
const LOW_HIGH_HZ: f64 = 250.0;
const MID_HIGH_HZ: f64 = 4000.0;

/// Tunables for [`segment_timeline`]. Mirrors [`crate::ProbeOpts`]'s
/// chainable-setter style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentOpts {
    /// Nominal window length, milliseconds. Default `1000.0`. The final
    /// window may be shorter (partial) if the file length isn't an exact
    /// multiple.
    pub window_ms: f64,
    /// RMS level, in dBFS, below which a window counts as `silent`. Default
    /// `-60.0` — mirrors [`crate::ProbeOpts::silence_floor_dbfs`].
    pub silence_floor_dbfs: f64,
}

impl Default for SegmentOpts {
    fn default() -> Self {
        Self {
            window_ms: 1000.0,
            silence_floor_dbfs: -60.0,
        }
    }
}

impl SegmentOpts {
    /// Override the window length, milliseconds.
    #[must_use]
    pub fn with_window_ms(mut self, window_ms: f64) -> Self {
        self.window_ms = window_ms;
        self
    }

    /// Override the silence floor, dBFS.
    #[must_use]
    pub fn with_silence_floor_dbfs(mut self, floor_dbfs: f64) -> Self {
        self.silence_floor_dbfs = floor_dbfs;
        self
    }
}

/// A windowed feature timeline over one [`Audio`] buffer, `schema_version:
/// 1`. See [`segment_timeline`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentTimeline {
    /// Schema version; see [`SEGMENTS_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The window length this timeline was built with, milliseconds.
    pub window_ms: f64,
    /// One entry per window, in order, index 0 first.
    pub segments: Vec<Segment>,
}

/// Fractional spectral energy in three bands, computed from the STFT
/// magnitudes whose frame center falls inside this segment. Fractions of
/// total in-segment energy, summing to `1.0` — except when the segment has
/// no STFT frame energy at all (silence, or a segment shorter than one FFT
/// window), in which case all three are `0.0`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BandEnergy {
    /// Fraction of energy below 250 Hz.
    pub low: f64,
    /// Fraction of energy in `[250, 4000]` Hz.
    pub mid: f64,
    /// Fraction of energy above 4000 Hz.
    pub high: f64,
}

/// One fixed-width window of [`SegmentTimeline`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Segment {
    /// Position in [`SegmentTimeline::segments`], `0`-based.
    pub index: usize,
    /// Window start, milliseconds.
    pub start_ms: f64,
    /// Window end, milliseconds (exclusive).
    pub end_ms: f64,
    /// Windowed RMS level, dBFS. `None` for digital silence (all-zero
    /// window), never `-inf` (mirrors [`crate::LoudnessReport`]'s
    /// convention — JSON has no `Infinity`).
    pub rms_dbfs: Option<f64>,
    /// Windowed sample peak, dBFS. `None` for digital silence.
    pub peak_dbfs: Option<f64>,
    /// Count of whole-file onsets whose time falls in `[start_ms, end_ms)`.
    pub onset_count: u32,
    /// Whether the windowed RMS is at or below the opts' silence floor
    /// (`true` for digital silence too, since `-inf <= floor` for any
    /// finite floor).
    pub silent: bool,
    /// Median YIN f0 across this window's voiced hops, Hz. `None` if the
    /// window is unvoiced, or shorter than one YIN analysis frame.
    pub f0_hz: Option<f64>,
    /// Nearest equal-tempered MIDI note to `f0_hz`, clamped to `0..=127`.
    /// `None` iff `f0_hz` is `None`.
    pub midi_nearest: Option<u8>,
    /// Deviation of `f0_hz` from `midi_nearest`'s pitch, cents. `None` iff
    /// `f0_hz` is `None`.
    pub cents_off: Option<f64>,
    /// Fractional spectral energy by band.
    pub band_energy: BandEnergy,
}

/// Build a [`SegmentTimeline`] over `audio`: fixed `opts.window_ms` windows,
/// each with its own RMS/peak, bucketed onset count, per-window YIN pitch,
/// and band energy fractions.
///
/// Onsets run once over the whole buffer (the same whole-file detector
/// [`crate::probe`] uses) and are bucketed by time, so
/// `segments.iter().map(|s| s.onset_count).sum()` equals the buffer's
/// global onset count. Band energy likewise runs one full-file STFT
/// (`1024`-point, `256`-sample hop — the onset detector's short-window
/// tradeoff, good time resolution for bucketing into ~second-scale windows)
/// and accumulates each frame's per-bin energy into whichever segment the
/// frame's center time lands in, rather than re-running a windowed STFT per
/// segment.
///
/// Empty audio, a non-positive `window_ms`, or a zero sample rate all
/// produce an empty `segments` vec. A buffer shorter than one window
/// produces a single partial segment.
pub fn segment_timeline(audio: &Audio, opts: &SegmentOpts) -> SegmentTimeline {
    let mono = audio.mono();
    let sample_rate = audio.sample_rate;

    if mono.is_empty() || sample_rate == 0 || opts.window_ms <= 0.0 {
        return SegmentTimeline {
            schema_version: SEGMENTS_SCHEMA_VERSION,
            window_ms: opts.window_ms,
            segments: Vec::new(),
        };
    }

    let window_len = ((opts.window_ms / 1000.0 * f64::from(sample_rate)).round() as usize).max(1);
    let segment_count = mono.len().div_ceil(window_len);

    let onsets_report = onsets::analyze(&mono, sample_rate);
    let mut onset_counts = vec![0u32; segment_count];
    for &t_ms in &onsets_report.times_ms {
        onset_counts[bucket_index(t_ms, opts.window_ms, segment_count)] += 1;
    }

    let stft = Stft::compute(&mono, sample_rate, BAND_FFT_SIZE, BAND_HOP);
    let mut band_energy_acc = vec![[0.0f64; 3]; segment_count];
    for (t, frame) in stft.magnitudes.iter().enumerate() {
        let center_ms = (t as f64 * BAND_HOP as f64 + BAND_FFT_SIZE as f64 / 2.0)
            / f64::from(sample_rate)
            * 1000.0;
        let idx = bucket_index(center_ms, opts.window_ms, segment_count);
        for (bin, &mag) in frame.iter().enumerate() {
            let hz = stft.bin_hz(bin);
            let energy = f64::from(mag) * f64::from(mag);
            let band = if hz < LOW_HIGH_HZ {
                0
            } else if hz <= MID_HIGH_HZ {
                1
            } else {
                2
            };
            band_energy_acc[idx][band] += energy;
        }
    }

    let mut segments = Vec::with_capacity(segment_count);
    for i in 0..segment_count {
        let start = i * window_len;
        let end = (start + window_len).min(mono.len());
        let slice = &mono[start..end];

        let start_ms = start as f64 / f64::from(sample_rate) * 1000.0;
        let end_ms = end as f64 / f64::from(sample_rate) * 1000.0;

        let mean_sq = slice
            .iter()
            .map(|&x| f64::from(x) * f64::from(x))
            .sum::<f64>()
            / slice.len() as f64;
        let rms = mean_sq.sqrt();
        let rms_dbfs = (rms > 0.0).then(|| 20.0 * libm::log10(rms));

        let peak = slice
            .iter()
            .fold(0.0f64, |acc, &x| acc.max(f64::from(x).abs()));
        let peak_dbfs = (peak > 0.0).then(|| 20.0 * libm::log10(peak));

        let silent = rms_dbfs.unwrap_or(f64::NEG_INFINITY) <= opts.silence_floor_dbfs;

        let f0_hz = pitch::analyze(slice, sample_rate).median_f0_hz;
        let (midi_nearest, cents_off) = match f0_hz {
            Some(f0) => {
                let midi = pitch::nearest_midi(f0);
                (
                    Some(midi.clamp(0, i32::from(u8::MAX)) as u8),
                    Some(pitch::cents_off(f0, midi)),
                )
            }
            None => (None, None),
        };

        let [low, mid, high] = band_energy_acc[i];
        let total = low + mid + high;
        let band_energy = if total > 0.0 {
            BandEnergy {
                low: low / total,
                mid: mid / total,
                high: high / total,
            }
        } else {
            BandEnergy {
                low: 0.0,
                mid: 0.0,
                high: 0.0,
            }
        };

        segments.push(Segment {
            index: i,
            start_ms,
            end_ms,
            rms_dbfs,
            peak_dbfs,
            onset_count: onset_counts[i],
            silent,
            f0_hz,
            midi_nearest,
            cents_off,
            band_energy,
        });
    }

    SegmentTimeline {
        schema_version: SEGMENTS_SCHEMA_VERSION,
        window_ms: opts.window_ms,
        segments,
    }
}

/// Map a time in milliseconds to a segment index by fixed-width bucketing
/// (`floor(t_ms / window_ms)`), clamped into `0..segment_count`.
fn bucket_index(t_ms: f64, window_ms: f64, segment_count: usize) -> usize {
    if segment_count == 0 {
        return 0;
    }
    let idx = (t_ms / window_ms).floor();
    if idx <= 0.0 {
        0
    } else {
        (idx as usize).min(segment_count - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_index_clamps_to_last_segment() {
        assert_eq!(bucket_index(-5.0, 1000.0, 5), 0);
        assert_eq!(bucket_index(0.0, 1000.0, 5), 0);
        assert_eq!(bucket_index(999.9, 1000.0, 5), 0);
        assert_eq!(bucket_index(1000.0, 1000.0, 5), 1);
        assert_eq!(bucket_index(4999.0, 1000.0, 5), 4);
        assert_eq!(bucket_index(5001.0, 1000.0, 5), 4); // clamp past the end
    }
}
