//! `silent_after`: windowed RMS over the mix, checked against a fixed
//! -60 dBFS floor from a tick to the end of the render.

use cochlea_render::Rendered;
use cochlea_score::{Score, Ticks};

use crate::checks::util::lin_to_db;
use crate::report::CheckResult;

/// RMS window length, milliseconds — matches `cochlea_features`' `silence`
/// module's windowed-RMS convention.
const WINDOW_MS: f64 = 50.0;
/// Hop between windows, milliseconds.
const HOP_MS: f64 = 10.0;
/// The fixed silence floor this check asserts against.
const FLOOR_DBFS: f64 = -60.0;

/// The mix's windowed RMS stays below `FLOOR_DBFS` from `at` to the end of
/// the render.
pub(crate) fn silent_after(rendered: &Rendered, score: &Score, at: Ticks) -> CheckResult {
    let kind = "silent_after";
    let at_ms = score.tempo_map().ms_at(at);
    let assertion =
        format!("the mix stays below {FLOOR_DBFS} dBFS from tick {at} ({at_ms:.1} ms) onward");
    let expected = format!("every {WINDOW_MS:.0} ms window <= {FLOOR_DBFS} dBFS");

    let mix = rendered.mix();
    let sample_rate = rendered.sample_rate().0;
    let frames = rendered.frames();
    let start = score.tempo_map().sample_at(at).min(frames);

    let window_len = ((WINDOW_MS / 1000.0 * f64::from(sample_rate)).round() as u64).max(1);
    let hop_len = ((HOP_MS / 1000.0 * f64::from(sample_rate)).round() as u64).max(1);

    let mut any_window = false;
    let mut worst_db = f64::NEG_INFINITY;
    let mut worst_ms = at_ms;
    let mut cursor = start;
    loop {
        if cursor >= frames {
            break;
        }
        let end = (cursor + window_len).min(frames);
        let mut sum_sq = 0.0f64;
        let mut count = 0u64;
        for f in cursor..end {
            let l = f64::from(mix[(f * 2) as usize]);
            let r = f64::from(mix[(f * 2 + 1) as usize]);
            let m = (l + r) / 2.0;
            sum_sq += m * m;
            count += 1;
        }
        if count > 0 {
            any_window = true;
            let rms = (sum_sq / count as f64).sqrt();
            let db = lin_to_db(rms);
            if db > worst_db {
                worst_db = db;
                worst_ms = cursor as f64 / f64::from(sample_rate) * 1000.0;
            }
        }
        if end >= frames {
            break;
        }
        cursor += hop_len;
    }

    let passed = worst_db < FLOOR_DBFS;
    let actual = if any_window {
        format!("worst window {worst_db:.1} dBFS at {worst_ms:.1} ms")
    } else {
        format!("no audio after {at_ms:.1} ms (render ends earlier)")
    };

    CheckResult {
        kind,
        assertion,
        passed,
        expected,
        actual,
        detail: None,
    }
}
