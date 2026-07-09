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

/// Band split, Hz: low is `< LOW_HIGH_HZ`, mid is `[LOW_HIGH_HZ,
/// MID_HIGH_HZ]`, high is `> MID_HIGH_HZ`.
const LOW_HIGH_HZ: f64 = 250.0;
const MID_HIGH_HZ: f64 = 4000.0;

/// Default silence floor, dBFS — also the fallback when a caller hands
/// [`segment_timeline`] a non-finite floor (a NaN floor would make every
/// `<=` comparison false and silently disable silence detection).
const DEFAULT_SILENCE_FLOOR_DBFS: f64 = -60.0;

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
            silence_floor_dbfs: DEFAULT_SILENCE_FLOOR_DBFS,
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
    /// The RMS floor, dBFS, that each segment's `silent` flag was computed
    /// against — serialized so a JSON consumer holding only the timeline
    /// can interpret (or re-derive) the classification instead of trusting
    /// an unstated threshold.
    pub silence_floor_dbfs: f64,
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
/// Empty audio, a zero sample rate, or a `window_ms` that is non-finite or
/// under 1 ms all produce an empty `segments` vec. (Non-finite matters:
/// `NaN` would otherwise slip past a `<= 0.0` guard — IEEE 754 comparisons
/// with NaN are all false — then saturate to a 1-sample window and explode
/// the segment count. The 1 ms floor closes the same resource hole for
/// tiny-but-positive values like `0.001`, whose rounded window length is
/// forced up to a single sample; sub-millisecond windows are also below
/// anything the per-window analyzers can measure.) A buffer shorter than
/// one window produces a single partial segment.
/// Validates a caller-supplied window length: finite and at least 1 ms —
/// the single source of truth the CLI flag parser and the MCP argument
/// validator delegate to, so the rule (and its rationale: NaN defeats
/// `<= 0.0` range checks, and sub-millisecond windows round to one-sample
/// segments and explode the timeline) lives in exactly one place.
/// [`segment_timeline`] itself stays infallible and degrades the same
/// class of input to an empty timeline.
pub fn validate_window_ms(window_ms: f64) -> Result<f64, String> {
    if window_ms.is_finite() && window_ms >= 1.0 {
        Ok(window_ms)
    } else {
        Err(format!(
            "must be a finite number of at least 1 (ms), got {window_ms}"
        ))
    }
}

pub fn segment_timeline(audio: &Audio, opts: &SegmentOpts) -> SegmentTimeline {
    let mono = audio.mono();
    let sample_rate = audio.sample_rate;

    // A NaN floor would make every `<=` comparison false and silently
    // disable silence detection; fall back to the documented default and
    // report the floor actually used (the timeline serializes it).
    let floor_dbfs = if opts.silence_floor_dbfs.is_finite() {
        opts.silence_floor_dbfs
    } else {
        DEFAULT_SILENCE_FLOOR_DBFS
    };

    if mono.is_empty() || sample_rate == 0 || !opts.window_ms.is_finite() || opts.window_ms < 1.0 {
        return SegmentTimeline {
            schema_version: SEGMENTS_SCHEMA_VERSION,
            window_ms: opts.window_ms,
            silence_floor_dbfs: floor_dbfs,
            segments: Vec::new(),
        };
    }

    let window_len = ((opts.window_ms / 1000.0 * f64::from(sample_rate)).round() as usize).max(1);
    let segment_count = mono.len().div_ceil(window_len);

    // One onsets-grade STFT serves both the onset detector and band-energy
    // bucketing (identical 1024/256 parameters — previously two separate,
    // byte-identical transforms), and one whole-file YIN track replaces a
    // fresh per-window pitch pass.
    let stft = Stft::compute(&mono, sample_rate, onsets::FFT_SIZE, onsets::HOP);
    let onsets_report = onsets::analyze_stft(&stft);
    let mut onset_counts = vec![0u32; segment_count];
    for &t_ms in &onsets_report.times_ms {
        onset_counts[bucket_index(t_ms, sample_rate, window_len, segment_count)] += 1;
    }

    // A hop contributes its f0 to a display window only when its analysis
    // frame lies fully inside that window — the same containment the old
    // per-slice YIN had, without re-analyzing audio the whole-file track
    // already covers.
    let mut window_f0s: Vec<Vec<f64>> = vec![Vec::new(); segment_count];
    for (start, f0) in pitch::f0_track(&mono, sample_rate) {
        if let Some(f0) = f0 {
            let first = start / window_len;
            let last = (start + pitch::window_len() - 1) / window_len;
            if first == last && first < segment_count {
                window_f0s[first].push(f0);
            }
        }
    }

    let mut band_energy_acc = vec![[0.0f64; 3]; segment_count];
    for (t, frame) in stft.magnitudes.iter().enumerate() {
        let center_ms = (t as f64 * onsets::HOP as f64 + onsets::FFT_SIZE as f64 / 2.0)
            / f64::from(sample_rate)
            * 1000.0;
        let idx = bucket_index(center_ms, sample_rate, window_len, segment_count);
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

        let silent = rms_dbfs.unwrap_or(f64::NEG_INFINITY) <= floor_dbfs;

        let f0_hz = pitch::median(&mut window_f0s[i]);
        let (midi_nearest, cents_off) = match f0_hz {
            Some(f0) => {
                let midi = pitch::nearest_midi(f0);
                (
                    // 127, not u8::MAX: the field documents standard MIDI
                    // range, and a YIN misfire above ~12.5 kHz must not
                    // leak 128..=255 to consumers trusting that.
                    Some(midi.clamp(0, 127) as u8),
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
        silence_floor_dbfs: floor_dbfs,
        segments,
    }
}

/// Map a time in milliseconds to a segment index, clamped into
/// `0..segment_count`.
///
/// Buckets on the same **sample-quantized** boundaries the segments are
/// actually built on (`i * window_len` samples), not on nominal
/// `floor(t_ms / window_ms)` — when `window_ms` doesn't map to a whole
/// number of samples (e.g. 3.0 ms at 44.1 kHz → 132 samples ≈ 2.993 ms),
/// nominal-ms bucketing drifts a full window off the real boundaries after
/// `window_ms / (window_ms - real_ms)` segments, misattributing onsets and
/// band energy to a neighboring segment.
fn bucket_index(t_ms: f64, sample_rate: u32, window_len: usize, segment_count: usize) -> usize {
    if segment_count == 0 || window_len == 0 {
        return 0;
    }
    let sample = t_ms / 1000.0 * f64::from(sample_rate);
    let idx = (sample / window_len as f64).floor();
    if idx <= 0.0 {
        0
    } else {
        (idx as usize).min(segment_count - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;
    /// 1000 ms at 48 kHz.
    const WIN: usize = 48_000;

    #[test]
    fn bucket_index_clamps_to_last_segment() {
        assert_eq!(bucket_index(-5.0, SR, WIN, 5), 0);
        assert_eq!(bucket_index(0.0, SR, WIN, 5), 0);
        assert_eq!(bucket_index(999.9, SR, WIN, 5), 0);
        assert_eq!(bucket_index(1000.0, SR, WIN, 5), 1);
        assert_eq!(bucket_index(4999.0, SR, WIN, 5), 4);
        assert_eq!(bucket_index(5001.0, SR, WIN, 5), 4); // clamp past the end
    }

    /// The drift case nominal-ms bucketing gets wrong: 3.0 ms at 44.1 kHz
    /// rounds to a 132-sample window (~2.993 ms), so by t = 1300 ms the
    /// real boundary grid is a full window behind the nominal one —
    /// sample 57330 sits in window 434, while `floor(1300 / 3) = 433`.
    #[test]
    fn bucket_index_follows_sample_boundaries_not_nominal_ms() {
        let sr = 44_100;
        let window_len = 132; // (3.0 ms / 1000 * 44100).round()
        assert_eq!(bucket_index(1300.0, sr, window_len, 1000), 434);
    }
}
