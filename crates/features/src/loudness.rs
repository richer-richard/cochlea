//! EBU R128 loudness/true-peak via the `ebur128` crate. `-inf`/undefined
//! readings (silence, too little audio for the requested mode) map to
//! `None`, never a JSON `Infinity` (`docs/determinism.md`'s ebur128 audit).

use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};

use crate::audio::Audio;
use crate::report::LoudnessReport;

pub(crate) fn analyze(audio: &Audio) -> LoudnessReport {
    if audio.channels == 0 || audio.samples.is_empty() {
        return LoudnessReport::default();
    }

    let Ok(mut ebu) = EbuR128::new(
        u32::from(audio.channels),
        audio.sample_rate,
        Mode::I | Mode::S | Mode::TRUE_PEAK | Mode::LRA,
    ) else {
        // Only fails on channels == 0 (guarded above) or allocation
        // failure; either way, loudness is simply unavailable.
        return LoudnessReport::default();
    };

    let channels = audio.channels as usize;
    // Feed in 100 ms chunks so momentary loudness (a 400 ms rolling window)
    // and short-term loudness (a 3 s window) can be polled once enough audio
    // has been fed, and their running maxima tracked ourselves — ebur128 only
    // exposes the *current* reading (`docs/determinism.md`'s ebur128 audit).
    let chunk_frames = (audio.sample_rate as usize / 10).max(1);
    let chunk_len = chunk_frames * channels;
    let momentary_ready_frames = (audio.sample_rate as usize * 4 / 10).max(1);
    let short_term_ready_frames = (audio.sample_rate as usize * 3).max(1);

    let mut momentary_max: Option<f64> = None;
    let mut short_term_max: Option<f64> = None;
    let mut frames_fed = 0usize;
    for chunk in audio.samples.chunks(chunk_len) {
        if ebu.add_frames_f32(chunk).is_err() {
            break;
        }
        frames_fed += chunk.len() / channels;
        if frames_fed >= momentary_ready_frames
            && let Ok(m) = ebu.loudness_momentary()
            && m.is_finite()
        {
            momentary_max = Some(momentary_max.map_or(m, |cur: f64| cur.max(m)));
        }
        if frames_fed >= short_term_ready_frames
            && let Ok(s) = ebu.loudness_shortterm()
            && s.is_finite()
        {
            short_term_max = Some(short_term_max.map_or(s, |cur: f64| cur.max(s)));
        }
    }

    let integrated_lufs = ebu.loudness_global().ok().filter(|v| v.is_finite());

    let mut true_peak_lin = 0.0f64;
    let mut sample_peak_lin = 0.0f64;
    for ch in 0..u32::from(audio.channels) {
        if let Ok(tp) = ebu.true_peak(ch) {
            true_peak_lin = true_peak_lin.max(tp);
        }
        if let Ok(sp) = ebu.sample_peak(ch) {
            sample_peak_lin = sample_peak_lin.max(sp);
        }
    }

    let lra = ebu.loudness_range().ok().filter(|v| v.is_finite());

    LoudnessReport {
        integrated_lufs,
        momentary_max_lufs: momentary_max,
        short_term_max_lufs: short_term_max,
        true_peak_dbtp: lin_to_db(true_peak_lin),
        sample_peak_dbfs: lin_to_db(sample_peak_lin),
        lra,
    }
}

/// Linear amplitude to dB (`20*log10`, via `libm`). Non-positive input —
/// ebur128 reports peaks as `Ok(0.0)` for silence, never `-inf` — maps to
/// `None` rather than `-inf`.
fn lin_to_db(lin: f64) -> Option<f64> {
    if lin > 0.0 {
        Some(20.0 * libm::log10(lin))
    } else {
        None
    }
}

/// EBU R128 loudness range (LRA), LU, per EBU 3342, standalone (see also
/// [`crate::Report::loudness`]'s `lra` field, computed in the same pass as
/// the rest of [`analyze`] since `schema_version: 2` — this function
/// remains for callers who want just the LRA without a full probe).
///
/// `None` when ebur128 can't produce a measurement: no audio, a
/// construction failure, or too little audio for even one gated block
/// (EBU R128 gates on absolute -70 LUFS blocks, so a buffer shorter than
/// one 400 ms block — or one that's entirely below that floor — reports
/// nothing to range over).
pub fn loudness_range(audio: &Audio) -> Option<f64> {
    if audio.channels == 0 || audio.samples.is_empty() {
        return None;
    }

    let mut ebu = EbuR128::new(u32::from(audio.channels), audio.sample_rate, Mode::LRA).ok()?;

    let channels = audio.channels as usize;
    // Feed in 100 ms chunks, same convention as `analyze` — LRA itself is
    // only read once at the end, but ebur128 only accumulates its gated
    // block history incrementally as frames are fed.
    let chunk_frames = (audio.sample_rate as usize / 10).max(1);
    let chunk_len = chunk_frames * channels;
    for chunk in audio.samples.chunks(chunk_len) {
        if ebu.add_frames_f32(chunk).is_err() {
            break;
        }
    }

    ebu.loudness_range().ok().filter(|v| v.is_finite())
}

/// Tunables for [`loudness_timeline`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessTimelineOpts {
    /// Spacing between timeline samples, milliseconds. Default `100.0` — the
    /// EBU R128 momentary-loudness update rate; a dynamics curve fine enough
    /// to see a gate open or a chorus lift without flooding the output.
    pub hop_ms: f64,
}

impl Default for LoudnessTimelineOpts {
    fn default() -> Self {
        Self { hop_ms: 100.0 }
    }
}

impl LoudnessTimelineOpts {
    /// Override the sample spacing, milliseconds.
    #[must_use]
    pub fn with_hop_ms(mut self, hop_ms: f64) -> Self {
        self.hop_ms = hop_ms;
        self
    }
}

/// One point on the loudness-over-time curve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoudnessPoint {
    /// Time of this reading, milliseconds (the end of the window fed so far).
    pub t_ms: f64,
    /// Momentary (400 ms) loudness at `t_ms`, LUFS; `None` before enough
    /// audio has been fed, or where the reading is `-inf`/undefined.
    pub momentary_lufs: Option<f64>,
    /// Short-term (3 s) loudness at `t_ms`, LUFS; `None` likewise.
    pub short_term_lufs: Option<f64>,
}

/// A loudness-over-time curve: momentary and short-term LUFS sampled every
/// `hop_ms`. The dynamics view the single integrated/`lra` summary can't give
/// — where the loud parts are, how the level moves. Standalone (not embedded
/// in [`crate::Report`], which would bloat every probe JSON with hundreds of
/// points); callers who want the curve ask for it, exactly like
/// [`crate::segment_timeline`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoudnessTimeline {
    /// Sample spacing, milliseconds (echoes the requested [`LoudnessTimelineOpts::hop_ms`]).
    pub hop_ms: f64,
    /// The curve, ascending in time.
    pub points: Vec<LoudnessPoint>,
}

/// Momentary + short-term LUFS sampled every `opts.hop_ms` over `audio`. See
/// [`LoudnessTimeline`]. Degenerate input (no audio, zero sample rate, or a
/// non-finite/sub-millisecond hop) yields an empty timeline, never a panic.
pub fn loudness_timeline(audio: &Audio, opts: &LoudnessTimelineOpts) -> LoudnessTimeline {
    let hop_ms = opts.hop_ms;
    if audio.channels == 0
        || audio.samples.is_empty()
        || audio.sample_rate == 0
        || !hop_ms.is_finite()
        || hop_ms < 1.0
    {
        return LoudnessTimeline {
            hop_ms,
            points: Vec::new(),
        };
    }

    let Ok(mut ebu) = EbuR128::new(
        u32::from(audio.channels),
        audio.sample_rate,
        Mode::M | Mode::S,
    ) else {
        return LoudnessTimeline {
            hop_ms,
            points: Vec::new(),
        };
    };

    let channels = audio.channels as usize;
    let frames = audio.samples.len() / channels;
    // Clamp the hop to [1 frame, the whole clip]: a hop past the end yields a
    // single point, and the upper bound keeps `hop_len` from overflowing
    // `usize` on an absurd (but finite) `hop_ms` — `as usize` saturates the
    // float to `usize::MAX`, and `* channels` would then wrap.
    let hop_frames =
        ((hop_ms / 1000.0 * f64::from(audio.sample_rate)).round() as usize).clamp(1, frames.max(1));
    let hop_len = hop_frames * channels;

    let mut points = Vec::new();
    let mut frames_fed = 0usize;
    for chunk in audio.samples.chunks(hop_len) {
        if ebu.add_frames_f32(chunk).is_err() {
            break;
        }
        frames_fed += chunk.len() / channels;
        let t_ms = frames_fed as f64 / f64::from(audio.sample_rate) * 1000.0;
        let momentary_lufs = ebu.loudness_momentary().ok().filter(|v| v.is_finite());
        let short_term_lufs = ebu.loudness_shortterm().ok().filter(|v| v.is_finite());
        points.push(LoudnessPoint {
            t_ms,
            momentary_lufs,
            short_term_lufs,
        });
    }

    LoudnessTimeline { hop_ms, points }
}
