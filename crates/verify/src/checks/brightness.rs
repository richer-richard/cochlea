//! `brightness_rises` / `brightness_falls`: did an authored filter sweep
//! audibly happen in the *rendered output*? The `monotone` check
//! deliberately validates only the authored automation curve (spectral
//! detail would conflate the instrument's response with the sweep); this
//! check is the output-side companion — it listens to the stem's spectral
//! centroid and asserts the range's end is brighter (or darker) than its
//! start by a margin. Together they close the loop: `monotone` proves the
//! score asked for a sweep, this proves the render delivered one.
//!
//! Robustness over strictness: a raw per-frame centroid track is noisy
//! (note attacks, envelope decay), so the check compares *medians* over
//! the range's first and last quarters rather than demanding per-frame
//! monotonicity — an agent asserting "the sweep audibly opened" wants
//! that question answered, not a referendum on frame-level jitter.

use cochlea_features::spectral_centroid_curve;
use cochlea_render::Rendered;
use cochlea_score::{Score, Ticks};

use crate::report::CheckResult;

/// Direction for the brightness comparison.
#[derive(Clone, Copy)]
pub(crate) enum BrightnessDir {
    Rising,
    Falling,
}

/// `track`'s stem median spectral centroid over the last quarter of
/// `from..to` is at least `min_ratio` times the first quarter's
/// (`Rising`), or vice versa (`Falling`). Fails with a detail note when
/// either quarter has no measurable centroid (silence) — asserting an
/// audible sweep presupposes audible material on both ends.
pub(crate) fn brightness(
    rendered: &Rendered,
    score: &Score,
    track: &str,
    from: Ticks,
    to: Ticks,
    min_ratio: f64,
    dir: BrightnessDir,
) -> CheckResult {
    let dir_word = match dir {
        BrightnessDir::Rising => "rises",
        BrightnessDir::Falling => "falls",
    };
    let kind = match dir {
        BrightnessDir::Rising => "brightness_rises",
        BrightnessDir::Falling => "brightness_falls",
    };
    let assertion = format!(
        "'{track}' spectral centroid {dir_word} by >= {min_ratio}x over ticks {from}..{to}"
    );
    let expected = format!("last-quarter/first-quarter centroid ratio {dir_word} >= {min_ratio}");

    let Some(stem) = rendered.stem(track) else {
        return CheckResult::unknown_track(kind, assertion, track);
    };

    let map = score.tempo_map();
    let (start_ms, end_ms) = (map.ms_at(from), map.ms_at(to));
    if end_ms <= start_ms {
        return CheckResult {
            kind,
            assertion,
            passed: false,
            expected,
            actual: format!("empty range ({start_ms:.0}..{end_ms:.0} ms)"),
            detail: Some("the tick range resolves to zero audio".to_string()),
        };
    }

    // Mono downmix of the interleaved stereo stem, then one centroid pass.
    let mono: Vec<f32> = stem
        .chunks_exact(2)
        .map(|frame| (frame[0] + frame[1]) * 0.5)
        .collect();
    let curve = spectral_centroid_curve(&mono, rendered.sample_rate().0);

    let quarter = (end_ms - start_ms) / 4.0;
    let first = median_centroid_in(&curve, start_ms, start_ms + quarter);
    let last = median_centroid_in(&curve, end_ms - quarter, end_ms);

    let (Some(first_hz), Some(last_hz)) = (first, last) else {
        return CheckResult {
            kind,
            assertion,
            passed: false,
            expected,
            actual: format!(
                "first quarter {} Hz, last quarter {} Hz",
                fmt_opt(first),
                fmt_opt(last)
            ),
            detail: Some(
                "no measurable centroid in one of the endpoint quarters (silence?)".to_string(),
            ),
        };
    };

    let passed = match dir {
        BrightnessDir::Rising => last_hz >= first_hz * min_ratio,
        BrightnessDir::Falling => first_hz >= last_hz * min_ratio,
    };
    CheckResult {
        kind,
        assertion,
        passed,
        expected,
        actual: format!(
            "centroid {first_hz:.0} Hz -> {last_hz:.0} Hz (ratio {:.2})",
            last_hz / first_hz
        ),
        detail: None,
    }
}

/// Median of the defined centroid values whose frame centers fall in
/// `[lo_ms, hi_ms)`. `None` if no frame in range has a defined centroid.
fn median_centroid_in(
    curve: &[cochlea_features::CentroidPoint],
    lo_ms: f64,
    hi_ms: f64,
) -> Option<f64> {
    let mut values: Vec<f64> = curve
        .iter()
        .filter(|p| p.t_ms >= lo_ms && p.t_ms < hi_ms)
        .filter_map(|p| p.hz)
        .collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    })
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_string(), |x| format!("{x:.0}"))
}
