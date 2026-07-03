//! `monotone`: the authored automation curve for one track/param, sampled
//! at block-start ticks over a tick range, must be monotone (non-strict)
//! in the inferred (or given) direction. This checks what was *authored*,
//! not the render — spectral verification of the resulting audio would
//! conflate the instrument's response with the sweep itself.

use cochlea_score::{MonotoneDir, Param, Score, Ticks};

use crate::report::CheckResult;

/// Render engine's control rate: automation is sampled once per this many
/// samples, at the block-start tick (`crates/render/src/engine.rs`,
/// `MAX_BLOCK`). This check samples the authored curve at the same block
/// starts so it validates what the renderer actually saw.
const BLOCK: u64 = 64;

fn dir_str(d: MonotoneDir) -> &'static str {
    match d {
        MonotoneDir::Rising => "rising",
        MonotoneDir::Falling => "falling",
    }
}

/// `track`'s authored automation curve for `param` is monotone over
/// `from..to`. `explicit_dir` overrides direction inference (the
/// `VerifySpec::Monotone` data-form path always supplies one); the typed
/// builder path passes `None` and infers direction by comparing the
/// curve's value at the two endpoints.
pub(crate) fn monotone(
    score: &Score,
    track: &str,
    param: &Param,
    from: Ticks,
    to: Ticks,
    explicit_dir: Option<MonotoneDir>,
) -> CheckResult {
    let kind = "monotone";
    let assertion = format!("'{track}' {param} automation is monotone over ticks {from}..{to}");

    let Some(score_track) = score.tracks().iter().find(|t| t.name == track) else {
        return CheckResult::unknown_track(kind, assertion, track);
    };
    let Some(automation) = score_track.automation.iter().find(|a| a.param == *param) else {
        return CheckResult {
            kind,
            assertion,
            passed: false,
            expected: "authored automation present for this param".to_string(),
            actual: "no automation".to_string(),
            detail: Some(format!("'{track}' has no automation for {param}")),
        };
    };

    let direction = explicit_dir.unwrap_or_else(|| {
        let (a, b) = (automation.value_at(from), automation.value_at(to));
        if b >= a {
            MonotoneDir::Rising
        } else {
            MonotoneDir::Falling
        }
    });

    let tempo_map = score.tempo_map();
    let start_sample = tempo_map.sample_at(from);
    let end_sample = tempo_map.sample_at(to);

    let mut samples: Vec<(u64, f32)> = Vec::new();
    if end_sample >= start_sample {
        let mut block = start_sample.div_ceil(BLOCK) * BLOCK;
        while block <= end_sample {
            let tick = tempo_map.tick_at(block);
            samples.push((block, automation.value_at(tick)));
            block += BLOCK;
        }
    }

    let expected = format!(
        "{} over {} sampled block(s)",
        dir_str(direction),
        samples.len()
    );

    if samples.len() < 2 {
        return CheckResult {
            kind,
            assertion,
            passed: true,
            expected,
            actual: format!(
                "only {} block start(s) between tick {from} and {to}; trivially monotone",
                samples.len()
            ),
            detail: Some("range spans fewer than two 64-sample blocks".to_string()),
        };
    }

    let mut worst: Option<(u64, f32, f32)> = None;
    for pair in samples.windows(2) {
        let ((_, v0), (s1, v1)) = (pair[0], pair[1]);
        let bad = match direction {
            MonotoneDir::Rising => v1 < v0,
            MonotoneDir::Falling => v1 > v0,
        };
        if bad {
            let delta = (v1 - v0).abs();
            if worst.is_none_or(|(_, wa, wb)| (wb - wa).abs() < delta) {
                worst = Some((s1, v0, v1));
            }
        }
    }

    match worst {
        Some((sample, v0, v1)) => CheckResult {
            kind,
            assertion,
            passed: false,
            expected,
            actual: format!(
                "{v0} -> {v1} at sample {sample} breaks {} order",
                dir_str(direction)
            ),
            detail: None,
        },
        None => {
            let first = samples.first().expect("len >= 2, checked above").1;
            let last = samples.last().expect("len >= 2, checked above").1;
            CheckResult {
                kind,
                assertion,
                passed: true,
                expected,
                actual: format!("{first} -> {last}, monotone {}", dir_str(direction)),
                detail: None,
            }
        }
    }
}
