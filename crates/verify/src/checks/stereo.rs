//! `stereo_width_within`: whole-mix stereo-image check via
//! `cochlea_features::analyze_stereo`.

use cochlea_features::analyze_stereo;
use cochlea_render::Rendered;

use crate::checks::util::stereo_audio;
use crate::report::CheckResult;

/// The mix's stereo width (`cochlea_features::StereoReport::width`,
/// `0.0..=1.0`) falls within `[min, max]`. Fails (with a detail note) if
/// the render isn't stereo — `cochlea_render::Rendered` is always
/// 2-channel in v1, so this only fires on a malformed buffer.
pub(crate) fn stereo_width_within(rendered: &Rendered, min: f64, max: f64) -> CheckResult {
    let kind = "stereo_width_within";
    let assertion = format!("mix stereo width is within [{min}, {max}]");
    let expected = format!("{min:.2}..={max:.2}");

    // `stereo_audio` always builds a 2-channel Audio (v1 renders are
    // stereo by construction), so analyze_stereo's None arm below is
    // defensive-only through this path — it goes live if a future engine
    // ever renders non-stereo mixes.
    let audio = stereo_audio(rendered.mix(), rendered.sample_rate().0);
    match analyze_stereo(&audio) {
        Some(report) => CheckResult {
            kind,
            assertion,
            passed: report.width >= min && report.width <= max,
            expected,
            actual: format!("{:.3}", report.width),
            detail: None,
        },
        None => CheckResult {
            kind,
            assertion,
            passed: false,
            expected,
            actual: "not stereo audio".to_string(),
            detail: Some("stereo width is undefined for non-stereo audio".to_string()),
        },
    }
}
