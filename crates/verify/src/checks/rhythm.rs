//! `has_clear_rhythm` and `grid_alignment_at_least`: whole-mix rhythm
//! (pattern) checks via `cochlea_features::estimate_tempo_and_rhythm` —
//! one shared analysis pass for both the tempo and rhythm halves.

use cochlea_features::{TempoOpts, estimate_tempo_and_rhythm};
use cochlea_render::Rendered;

use crate::checks::util::stereo_audio;
use crate::report::CheckResult;

/// The mix's rhythm trustworthiness
/// (`cochlea_features::RhythmReport::clear_rhythm` — the grid-based rule:
/// detected pulse, busy-enough surface, onsets on the subdivision grid)
/// equals `expected` — asserts a clear, articulated rhythm when `true`,
/// or asserts the *absence* of one when `false`.
pub(crate) fn has_clear_rhythm(rendered: &Rendered, expected: bool) -> CheckResult {
    let kind = "has_clear_rhythm";
    let assertion = format!("mix's clear_rhythm flag is {expected}");
    let expected_text = format!("clear_rhythm = {expected}");

    let audio = stereo_audio(rendered.mix(), rendered.sample_rate().0);
    let (tempo, rhythm) = estimate_tempo_and_rhythm(&audio, &TempoOpts::default());

    let bpm_text = tempo
        .bpm
        .map_or_else(|| "none".to_string(), |b| format!("{b:.1}"));
    let align_text = rhythm
        .grid_alignment
        .map_or_else(|| "-".to_string(), |a| format!("{a:.2}"));
    CheckResult {
        kind,
        assertion,
        passed: rhythm.clear_rhythm == expected,
        expected: expected_text,
        actual: format!(
            "clear_rhythm = {} (grid_align {align_text}, confidence {:.2}, bpm {bpm_text})",
            rhythm.clear_rhythm, tempo.confidence
        ),
        detail: None,
    }
}

/// The mix's rhythm grid alignment
/// (`cochlea_features::RhythmReport::grid_alignment`) is at least `min`.
/// Fails (with a detail note) when there is no usable beat grid at all —
/// per the undefined-metric policy (`checks` module docs), asserting a
/// minimum alignment presupposes a grid to align to.
pub(crate) fn grid_alignment_at_least(rendered: &Rendered, min: f64) -> CheckResult {
    let kind = "grid_alignment_at_least";
    let assertion = format!("mix's onset grid alignment is at least {min}");
    let expected_text = format!("grid_alignment >= {min:.2}");

    let audio = stereo_audio(rendered.mix(), rendered.sample_rate().0);
    let (tempo, rhythm) = estimate_tempo_and_rhythm(&audio, &TempoOpts::default());

    match rhythm.grid_alignment {
        Some(align) => CheckResult {
            kind,
            assertion,
            passed: align >= min,
            expected: expected_text,
            actual: format!(
                "grid_alignment = {align:.2} (bpm {}, {} onsets/s)",
                tempo
                    .bpm
                    .map_or_else(|| "none".to_string(), |b| format!("{b:.1}")),
                format_args!("{:.2}", rhythm.onset_rate_per_s),
            ),
            detail: None,
        },
        None => CheckResult {
            kind,
            assertion,
            passed: false,
            expected: expected_text,
            actual: "no usable beat grid".to_string(),
            detail: Some(
                "no beat grid to align to (no detected tempo, or no onsets to classify)"
                    .to_string(),
            ),
        },
    }
}
