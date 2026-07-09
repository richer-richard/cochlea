//! EBU R128 loudness/true-peak via the `ebur128` crate. `-inf`/undefined
//! readings (silence, too little audio for the requested mode) map to
//! `None`, never a JSON `Infinity` (`docs/determinism.md`'s ebur128 audit).

use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};

use crate::audio::Audio;
use crate::report::LoudnessReport;

/// Short-term loudness (last 3 s, EBU R128) polled once per second of
/// programme.
const SHORT_TERM_WINDOW_S: usize = 3;
/// Hop between [`LoudnessDynamicsReport::short_term`] samples, seconds.
const SHORT_TERM_HOP_S: usize = 1;

pub(crate) fn analyze(audio: &Audio) -> LoudnessReport {
    if audio.channels == 0 || audio.samples.is_empty() {
        return LoudnessReport::default();
    }

    let Ok(mut ebu) = EbuR128::new(
        u32::from(audio.channels),
        audio.sample_rate,
        Mode::I | Mode::TRUE_PEAK,
    ) else {
        // Only fails on channels == 0 (guarded above) or allocation
        // failure; either way, loudness is simply unavailable.
        return LoudnessReport::default();
    };

    let channels = audio.channels as usize;
    // Feed in 100 ms chunks so momentary loudness (a 400 ms rolling window)
    // can be polled once enough audio has been fed, and its running max
    // tracked ourselves — ebur128 only exposes the *current* momentary
    // reading (`docs/determinism.md`'s ebur128 audit).
    let chunk_frames = (audio.sample_rate as usize / 10).max(1);
    let chunk_len = chunk_frames * channels;
    let momentary_ready_frames = (audio.sample_rate as usize * 4 / 10).max(1);

    let mut momentary_max: Option<f64> = None;
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

    LoudnessReport {
        integrated_lufs,
        momentary_max_lufs: momentary_max,
        true_peak_dbtp: lin_to_db(true_peak_lin),
        sample_peak_dbfs: lin_to_db(sample_peak_lin),
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

/// One point of [`LoudnessDynamicsReport::short_term`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ShortTermPoint {
    /// Sample time, milliseconds — the point at which this 3 s window's
    /// worth of history had just been fed.
    pub time_ms: f64,
    /// Short-term (3 s) loudness at this point, LUFS. `None` where ebur128
    /// reports `-inf` (no energy in the trailing 3 s window yet) — the same
    /// convention as [`crate::LoudnessReport`]'s fields.
    pub lufs: Option<f64>,
}

/// Loudness *dynamics*: EBU R128 loudness range (LRA) and a short-term
/// loudness curve. Plain struct, no own schema version — parallel API
/// alongside [`crate::LoudnessReport`], meant to be embedded into a future
/// `Report` schema bump rather than stand alone (mirrors
/// [`crate::TempoReport`]'s status).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoudnessDynamicsReport {
    /// Loudness range per EBU 3342, LU. `None` only if ebur128 reports a
    /// non-finite value (in practice: never observed, but guarded — see
    /// [`loudness_dynamics`]). `Some(0.0)` is a legitimate reading (silence,
    /// or perfectly constant loudness both genuinely have zero range), not
    /// a sentinel for "undefined."
    pub lra: Option<f64>,
    /// Short-term loudness curve, one point per second of programme,
    /// starting once 3 s of audio have been fed. Empty if the buffer is
    /// shorter than 3 s.
    pub short_term: Vec<ShortTermPoint>,
}

fn empty_dynamics() -> LoudnessDynamicsReport {
    LoudnessDynamicsReport {
        lra: None,
        short_term: Vec::new(),
    }
}

/// EBU R128 loudness range (LRA) and a short-term loudness curve — a
/// second, independent `EbuR128` pass over `audio` alongside
/// [`analyze`]'s (`Mode::LRA` implies short-term tracking internally;
/// there's no cheap way to add it to the same instance after the fact
/// without also computing integrated/true-peak state it doesn't need).
///
/// Feeds the same 100 ms chunks as [`analyze`], polling
/// [`ebur128::EbuR128::loudness_shortterm`] once per elapsed second (once
/// the first 3 s of trailing history exists — EBU R128's short-term window
/// — see [`SHORT_TERM_WINDOW_S`]/[`SHORT_TERM_HOP_S`]) and reading LRA once
/// at the end from the full block-energy history ebur128 accumulated along
/// the way.
pub fn loudness_dynamics(audio: &Audio) -> LoudnessDynamicsReport {
    if audio.channels == 0 || audio.samples.is_empty() {
        return empty_dynamics();
    }

    let Ok(mut ebu) = EbuR128::new(u32::from(audio.channels), audio.sample_rate, Mode::LRA) else {
        return empty_dynamics();
    };

    let channels = audio.channels as usize;
    let chunk_frames = (audio.sample_rate as usize / 10).max(1);
    let chunk_len = chunk_frames * channels;
    let ready_frames = (audio.sample_rate as usize * SHORT_TERM_WINDOW_S).max(1);
    let hop_frames = (audio.sample_rate as usize * SHORT_TERM_HOP_S).max(1);

    let mut short_term = Vec::new();
    let mut frames_fed = 0usize;
    let mut next_poll_at = ready_frames;

    for chunk in audio.samples.chunks(chunk_len) {
        if ebu.add_frames_f32(chunk).is_err() {
            break;
        }
        frames_fed += chunk.len() / channels;
        while frames_fed >= next_poll_at {
            let lufs = ebu.loudness_shortterm().ok().filter(|v| v.is_finite());
            short_term.push(ShortTermPoint {
                time_ms: next_poll_at as f64 / f64::from(audio.sample_rate) * 1000.0,
                lufs,
            });
            next_poll_at += hop_frames;
        }
    }

    let lra = ebu.loudness_range().ok().filter(|v| v.is_finite());

    LoudnessDynamicsReport { lra, short_term }
}
