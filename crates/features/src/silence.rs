//! Leading/trailing silence and last-audible-sample detection via windowed
//! RMS over the mono downmix.

use crate::report::SilenceReport;

/// RMS window length, milliseconds.
const WINDOW_MS: f64 = 50.0;
/// Hop between windows, milliseconds.
const HOP_MS: f64 = 10.0;

pub(crate) fn analyze(mono: &[f32], sample_rate: u32, floor_dbfs: f64) -> SilenceReport {
    let total = mono.len();
    let duration_ms = if sample_rate == 0 {
        0.0
    } else {
        total as f64 / f64::from(sample_rate) * 1000.0
    };

    if total == 0 || sample_rate == 0 {
        return silent_report(duration_ms, floor_dbfs);
    }

    let window_len = ((WINDOW_MS / 1000.0 * f64::from(sample_rate)).round() as usize).max(1);
    let hop_len = ((HOP_MS / 1000.0 * f64::from(sample_rate)).round() as usize).max(1);

    let mut first_audible_start: Option<usize> = None;
    let mut last_audible_end: Option<usize> = None;

    let mut start = 0;
    loop {
        let end = (start + window_len).min(total);
        let window = &mono[start..end];
        let mean_sq = window
            .iter()
            .map(|&x| f64::from(x) * f64::from(x))
            .sum::<f64>()
            / window.len() as f64;
        let rms = mean_sq.sqrt();
        let dbfs = if rms > 0.0 {
            20.0 * libm::log10(rms)
        } else {
            f64::NEG_INFINITY
        };
        if dbfs > floor_dbfs {
            first_audible_start.get_or_insert(start);
            last_audible_end = Some(end);
        }
        if end >= total {
            break;
        }
        start += hop_len;
    }

    match (first_audible_start, last_audible_end) {
        (Some(first), Some(last_end)) => SilenceReport {
            leading_ms: first as f64 / f64::from(sample_rate) * 1000.0,
            trailing_ms: (total - last_end) as f64 / f64::from(sample_rate) * 1000.0,
            last_audible_sample: Some(last_end.saturating_sub(1)),
            floor_dbfs,
        },
        _ => silent_report(duration_ms, floor_dbfs),
    }
}

/// The report for a buffer with no window above the floor: entirely
/// leading (and trailing) silence, no audible sample.
fn silent_report(duration_ms: f64, floor_dbfs: f64) -> SilenceReport {
    SilenceReport {
        leading_ms: duration_ms,
        trailing_ms: duration_ms,
        last_audible_sample: None,
        floor_dbfs,
    }
}
