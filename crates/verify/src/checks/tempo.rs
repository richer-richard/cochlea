//! `tempo_is`: the whole-mix tempo (speed) check via
//! `cochlea_features::estimate_tempo`. Rhythm (pattern) checks —
//! `has_clear_rhythm`, `grid_alignment_at_least` — live in
//! `checks::rhythm`, mirroring the features crate's tempo/rhythm split.

use cochlea_features::{TempoOpts, estimate_tempo};
use cochlea_render::Rendered;

use crate::checks::util::stereo_audio;
use crate::report::CheckResult;

/// The mix's estimated tempo is within `tol_bpm` of `bpm`. Fails (with a
/// detail note) if the mix has no measurable tempo at all — too short,
/// silent, or an onset-strength envelope with no periodic energy (see
/// `cochlea_features::estimate_tempo`'s docs for exactly when that is);
/// per the undefined-metric policy (`checks` module docs), asserting a
/// specific tempo presupposes a measurable one.
///
/// `min_bpm`/`max_bpm` override the detector's search range (default
/// 30–300 BPM) — the escape hatch for fast material: above ~170 BPM the
/// octave-error prior otherwise favors the half-time subharmonic.
pub(crate) fn tempo_is(
    rendered: &Rendered,
    bpm: f64,
    tol_bpm: f64,
    min_bpm: Option<f64>,
    max_bpm: Option<f64>,
) -> CheckResult {
    let kind = "tempo_is";
    let assertion = format!("mix tempo is {bpm} BPM (± {tol_bpm} BPM)");
    let expected = format!("{bpm:.2} ± {tol_bpm:.2} BPM");

    let mut opts = TempoOpts::default();
    if let Some(min) = min_bpm {
        opts = opts.with_min_bpm(min);
    }
    if let Some(max) = max_bpm {
        opts = opts.with_max_bpm(max);
    }
    let audio = stereo_audio(rendered.mix(), rendered.sample_rate().0);
    let report = estimate_tempo(&audio, &opts);

    match report.bpm {
        Some(measured) => {
            let diff = (measured - bpm).abs();
            CheckResult {
                kind,
                assertion,
                passed: diff <= tol_bpm,
                expected,
                actual: format!(
                    "{measured:.2} BPM (Δ {diff:.2}, confidence {:.2})",
                    report.confidence
                ),
                detail: None,
            }
        }
        None => CheckResult {
            kind,
            assertion,
            passed: false,
            expected,
            actual: "no measurable tempo".to_string(),
            detail: Some(
                "no measurable tempo (too short, silent, or no periodic onset energy?)".to_string(),
            ),
        },
    }
}

