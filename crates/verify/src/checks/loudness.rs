//! `integrated_lufs`, `true_peak_below`, and `lra_below`: whole-mix
//! loudness checks via `cochlea_features`' ebur128-backed loudness
//! extractors.

use cochlea_features::{ProbeOpts, loudness_range, probe};
use cochlea_render::Rendered;

use crate::checks::util::stereo_audio;
use crate::report::CheckResult;

/// Integrated (program) loudness of the mix within `tol` LU of `target`
/// LUFS. A silent mix has no gated loudness (ebur128 reports `-inf`,
/// mapped to `None` upstream) and always fails, with a detail note.
pub(crate) fn integrated_lufs(rendered: &Rendered, target: f64, tol: f64) -> CheckResult {
    let kind = "integrated_lufs";
    let assertion = format!("mix integrated loudness is {target} LUFS (± {tol} LU)");
    let expected = format!("{target:.2} ± {tol:.2} LUFS");

    let audio = stereo_audio(rendered.mix(), rendered.sample_rate().0);
    let report = probe(&audio, &ProbeOpts::default());

    match report.loudness.integrated_lufs {
        Some(v) => {
            let diff = (v - target).abs();
            CheckResult {
                kind,
                assertion,
                passed: diff <= tol,
                expected,
                actual: format!("{v:.2} LUFS (Δ {diff:.2} LU)"),
                detail: None,
            }
        }
        None => CheckResult {
            kind,
            assertion,
            passed: false,
            expected,
            actual: "no gated loudness".to_string(),
            detail: Some("no gated loudness (silence?)".to_string()),
        },
    }
}

/// True peak of the mix at or below `dbtp` dBTP. A mix with no measurable
/// peak (silence) is treated as `-inf` dBTP, so it always passes (with a
/// detail note).
pub(crate) fn true_peak_below(rendered: &Rendered, dbtp: f64) -> CheckResult {
    let kind = "true_peak_below";
    let assertion = format!("mix true peak is at or below {dbtp} dBTP");
    let expected = format!("<= {dbtp:.2} dBTP");

    let audio = stereo_audio(rendered.mix(), rendered.sample_rate().0);
    let report = probe(&audio, &ProbeOpts::default());

    match report.loudness.true_peak_dbtp {
        Some(tp) => CheckResult {
            kind,
            assertion,
            passed: tp <= dbtp,
            expected,
            actual: format!("{tp:.2} dBTP"),
            detail: None,
        },
        None => CheckResult {
            kind,
            assertion,
            passed: true,
            expected,
            actual: "-inf dBTP".to_string(),
            detail: Some("no measurable true peak (silence?)".to_string()),
        },
    }
}

/// EBU R128 loudness range (LRA) of the mix at or below `lu` LU. A mix
/// with no measurable range (too short, or entirely below the -70 LUFS
/// absolute gate) has nothing to exceed the bound, so it always passes
/// (with a detail note) — mirrors [`true_peak_below`]'s convention for an
/// undefined metric.
pub(crate) fn lra_below(rendered: &Rendered, lu: f64) -> CheckResult {
    let kind = "lra_below";
    let assertion = format!("mix loudness range is at or below {lu} LU");
    let expected = format!("<= {lu:.2} LU");

    let audio = stereo_audio(rendered.mix(), rendered.sample_rate().0);

    match loudness_range(&audio) {
        Some(measured) => CheckResult {
            kind,
            assertion,
            passed: measured <= lu,
            expected,
            actual: format!("{measured:.2} LU"),
            detail: None,
        },
        None => CheckResult {
            kind,
            assertion,
            passed: true,
            expected,
            actual: "no measurable loudness range".to_string(),
            detail: Some("no measurable loudness range (too short or silent?)".to_string()),
        },
    }
}
