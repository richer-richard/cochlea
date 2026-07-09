//! `section_count`: whole-mix structural section check via
//! `cochlea_features::detect_structure`.

use cochlea_features::{StructureOpts, detect_structure};
use cochlea_render::Rendered;

use crate::checks::util::stereo_audio;
use crate::report::CheckResult;

/// The mix's detected structural section count
/// (`cochlea_features::StructureReport::section_count`) falls within
/// `[min, max]`. Always defined (never `None`) — `detect_structure`
/// reports `section_count: 0` only for genuinely empty/invalid input and
/// `1` for audio too short to segment, never an undefined value.
pub(crate) fn section_count(rendered: &Rendered, min: usize, max: usize) -> CheckResult {
    let kind = "section_count";
    let assertion = format!("mix has between {min} and {max} structural sections");
    let expected = format!("{min}..={max}");

    let audio = stereo_audio(rendered.mix(), rendered.sample_rate().0);
    let report = detect_structure(&audio, &StructureOpts::default());

    CheckResult {
        kind,
        assertion,
        passed: report.section_count >= min && report.section_count <= max,
        expected,
        actual: format!(
            "{} section(s) (confidence {:.2})",
            report.section_count, report.confidence
        ),
        detail: None,
    }
}
