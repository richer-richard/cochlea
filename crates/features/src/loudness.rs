//! EBU R128 loudness/true-peak via the `ebur128` crate. `-inf`/undefined
//! readings (silence, too little audio for the requested mode) map to
//! `None`, never a JSON `Infinity` (`docs/determinism.md`'s ebur128 audit).

use ebur128::{EbuR128, Mode};

use crate::audio::Audio;
use crate::report::LoudnessReport;

pub(crate) fn analyze(audio: &Audio) -> LoudnessReport {
    if audio.channels == 0 || audio.samples.is_empty() {
        return LoudnessReport::default();
    }

    let Ok(mut ebu) = EbuR128::new(
        u32::from(audio.channels),
        audio.sample_rate,
        Mode::I | Mode::TRUE_PEAK | Mode::LRA,
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

    let lra = ebu.loudness_range().ok().filter(|v| v.is_finite());

    LoudnessReport {
        integrated_lufs,
        momentary_max_lufs: momentary_max,
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
