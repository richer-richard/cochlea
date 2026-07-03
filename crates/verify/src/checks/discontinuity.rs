//! `no_discontinuity`: a click detector — flags sample-to-sample jumps
//! louder than a threshold, ignoring guard windows around each scored
//! note's on/off boundaries (attacks and releases are allowed to be loud)
//! and ignoring pairs where both samples already sit near the noise floor.

use cochlea_render::Rendered;
use cochlea_score::Score;

use crate::checks::util::lin_to_db;
use crate::report::CheckResult;

/// Guard half-width around every note on/off boundary, milliseconds.
const GUARD_MS: f64 = 10.0;
/// Deltas where both samples sit below this (dBFS) are ignored outright —
/// quantization/dither-level noise near silence, not a click.
const QUIET_FLOOR_DBFS: f64 = -70.0;

/// `track`'s stem has no sample-to-sample jump louder than `-db` dBFS
/// (e.g. `db = 40.0` flags jumps louder than -40 dBFS), outside a
/// `±GUARD_MS` window around every note on/off boundary and outside pairs
/// already below `QUIET_FLOOR_DBFS`.
pub(crate) fn no_discontinuity(
    rendered: &Rendered,
    score: &Score,
    track: &str,
    db: f64,
) -> CheckResult {
    let kind = "no_discontinuity";
    let assertion = format!(
        "'{track}' has no sample-to-sample jump louder than -{db} dBFS outside note-boundary guards"
    );
    let expected =
        format!("all Δ <= -{db:.1} dBFS (or inside a ±{GUARD_MS:.0} ms note-boundary guard)");

    let Some(stem) = rendered.stem(track) else {
        return CheckResult::unknown_track(kind, assertion, track);
    };
    let Some(score_track) = score.tracks().iter().find(|t| t.name == track) else {
        return CheckResult::unknown_track(kind, assertion, track);
    };

    let sample_rate = rendered.sample_rate().0;
    let tempo_map = score.tempo_map();
    let guard_samples = (GUARD_MS / 1000.0 * f64::from(sample_rate)).round() as u64;

    let mut guards: Vec<(u64, u64)> = Vec::new();
    for note in &score_track.notes {
        for tick in [note.at, note.end()] {
            let s = tempo_map.sample_at(tick);
            guards.push((s.saturating_sub(guard_samples), s + guard_samples));
        }
    }

    let threshold_db = -db;
    let frames = (stem.len() / 2) as u64;

    let mut worst_violation: Option<(f64, u64, usize)> = None;
    let mut worst_overall: Option<(f64, u64, usize)> = None;

    for f in 1..frames {
        for ch in 0..2usize {
            let cur = f64::from(stem[(f * 2) as usize + ch]);
            let prev = f64::from(stem[((f - 1) * 2) as usize + ch]);
            let delta_db = lin_to_db((cur - prev).abs());

            if delta_db.is_finite() && worst_overall.is_none_or(|(w, _, _)| delta_db > w) {
                worst_overall = Some((delta_db, f, ch));
            }

            if delta_db <= threshold_db {
                continue;
            }
            if lin_to_db(cur.abs()) < QUIET_FLOOR_DBFS && lin_to_db(prev.abs()) < QUIET_FLOOR_DBFS {
                continue;
            }
            if in_any_guard(f, &guards) || in_any_guard(f - 1, &guards) {
                continue;
            }
            if worst_violation.is_none_or(|(w, _, _)| delta_db > w) {
                worst_violation = Some((delta_db, f, ch));
            }
        }
    }

    match worst_violation {
        Some((d, idx, ch)) => CheckResult {
            kind,
            assertion,
            passed: false,
            expected,
            actual: format!("{d:.2} dBFS jump at sample {idx} (channel {ch})"),
            detail: None,
        },
        None => {
            let actual = match worst_overall {
                Some((d, idx, ch)) => {
                    format!("max Δ {d:.2} dBFS at sample {idx} (channel {ch}); no violations")
                }
                None => "no sample-to-sample deltas (stem too short)".to_string(),
            };
            CheckResult {
                kind,
                assertion,
                passed: true,
                expected,
                actual,
                detail: None,
            }
        }
    }
}

fn in_any_guard(sample: u64, guards: &[(u64, u64)]) -> bool {
    guards.iter().any(|&(lo, hi)| sample >= lo && sample <= hi)
}
