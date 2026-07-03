//! `onset_at`: a detected onset on a track's stem within tolerance of a
//! scored tick, via `cochlea_features`' spectral-flux onset detector.

use cochlea_features::{ProbeOpts, probe};
use cochlea_render::Rendered;
use cochlea_score::{Score, Ticks};

use crate::checks::util::stereo_audio;
use crate::report::CheckResult;

/// `track`'s stem has a detected onset within `tol_ms` of `at`. `actual`
/// always reports the nearest detected onset and its delta, even on a
/// pass, for context.
pub(crate) fn onset_at(
    rendered: &Rendered,
    score: &Score,
    track: &str,
    at: Ticks,
    tol_ms: f64,
) -> CheckResult {
    let kind = "onset_at";
    let expected_ms = score.tempo_map().ms_at(at);
    let assertion =
        format!("'{track}' has an onset within {tol_ms} ms of tick {at} ({expected_ms:.1} ms)");
    let expected = format!("onset within {tol_ms:.1} ms of {expected_ms:.1} ms");

    let Some(stem) = rendered.stem(track) else {
        return CheckResult::unknown_track(kind, assertion, track);
    };

    let audio = stereo_audio(stem, rendered.sample_rate().0);
    let report = probe(&audio, &ProbeOpts::default());

    let Some(nearest) = report
        .onsets
        .times_ms
        .iter()
        .copied()
        .min_by(|a, b| (a - expected_ms).abs().total_cmp(&(b - expected_ms).abs()))
    else {
        return CheckResult {
            kind,
            assertion,
            passed: false,
            expected,
            actual: "no onsets detected".to_string(),
            detail: Some(format!("'{track}' stem has zero detected onsets")),
        };
    };

    let delta = (nearest - expected_ms).abs();
    CheckResult {
        kind,
        assertion,
        passed: delta <= tol_ms,
        expected,
        actual: format!("nearest onset {nearest:.1} ms (Δ {delta:.1} ms)"),
        detail: None,
    }
}
