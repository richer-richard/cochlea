//! YIN pitch tracking over the mono downmix: difference function,
//! cumulative mean normalized difference (CMNDF), absolute threshold,
//! parabolic interpolation. de Cheveigné & Kawahara, "YIN, a fundamental
//! frequency estimator for speech and music" (2002).

use crate::report::{PitchReport, PitchSegment};

/// Analysis window, samples.
const WINDOW: usize = 2048;
/// Hop between windows, samples.
const HOP: usize = 512;
/// YIN's absolute CMNDF threshold: the first period candidate whose CMNDF
/// dips below this is accepted (de Cheveigné & Kawahara recommend ~0.1).
const THRESHOLD: f64 = 0.1;

/// Summarize an already-computed f0 track (see [`f0_track`]) into the
/// probe report's pitch section: voiced ratio, whole-buffer median f0,
/// contiguous voiced runs, and the quantized melody note events
/// (`crate::melody`) — one YIN pass feeds all of it.
pub(crate) fn analyze_track(track: &[(usize, Option<f64>)], sample_rate: u32) -> PitchReport {
    if track.is_empty() || sample_rate == 0 {
        return PitchReport {
            voiced_ratio: 0.0,
            median_f0_hz: None,
            segments: Vec::new(),
            melody: Vec::new(),
        };
    }

    let f0s: Vec<Option<f64>> = track.iter().map(|(_, f0)| *f0).collect();

    let voiced_count = f0s.iter().filter(|v| v.is_some()).count();
    let voiced_ratio = voiced_count as f64 / f0s.len() as f64;

    let mut all_voiced: Vec<f64> = f0s.iter().filter_map(|v| *v).collect();
    let median_f0_hz = median(&mut all_voiced);

    let segments = segment_runs(&f0s, sample_rate);
    let melody = crate::melody::notes_from_track(track, sample_rate);

    PitchReport {
        voiced_ratio,
        median_f0_hz,
        segments,
        melody,
    }
}

/// The raw per-hop f0 track `analyze` summarizes: one
/// `(window start sample, f0)` per `HOP`-spaced `WINDOW`-sample frame.
/// `pub(crate)`: `segments` buckets these per display window (a hop
/// contributes to a window only when its analysis frame lies fully inside
/// it) instead of re-running a full YIN pass per window — for a 3-minute
/// file at 1 s windows that was ~180 extra YIN passes over audio this
/// track already covers.
pub(crate) fn f0_track(mono: &[f32], sample_rate: u32) -> Vec<(usize, Option<f64>)> {
    if mono.len() < WINDOW || sample_rate == 0 {
        return Vec::new();
    }
    let frame_count = (mono.len() - WINDOW) / HOP + 1;
    (0..frame_count)
        .map(|f| {
            let start = f * HOP;
            (
                start,
                yin_f0(&mono[start..start + WINDOW], f64::from(sample_rate)),
            )
        })
        .collect()
}

/// Analysis window length in samples — `pub(crate)` so `segments` can test
/// whether a hop's frame lies fully inside a display window.
pub(crate) const fn window_len() -> usize {
    WINDOW
}

/// Hop length in samples — `pub(crate)` so `melody` can convert frame
/// indices to the same times this module's segments use.
pub(crate) const fn hop_len() -> usize {
    HOP
}

/// One frame of YIN: `Some(f0_hz)` if some period passed the absolute
/// threshold, `None` if the frame looks unvoiced (silence, noise).
fn yin_f0(frame: &[f32], sample_rate: f64) -> Option<f64> {
    let tau_max = frame.len() / 2;
    if tau_max < 3 {
        return None;
    }

    // Difference function: d(tau) = sum_j (x[j] - x[j+tau])^2. Zipping the
    // frame against its tau-shifted self sums exactly j in
    // 0..(len - tau), no manual index bookkeeping.
    let mut diff = vec![0.0f64; tau_max + 1];
    for (tau, slot) in diff.iter_mut().enumerate().skip(1) {
        *slot = frame
            .iter()
            .zip(frame[tau..].iter())
            .map(|(&a, &b)| {
                let d = f64::from(a) - f64::from(b);
                d * d
            })
            .sum();
    }

    // Cumulative mean normalized difference function.
    let mut cmndf = vec![1.0f64; tau_max + 1];
    let mut running_sum = 0.0f64;
    for tau in 1..=tau_max {
        running_sum += diff[tau];
        cmndf[tau] = if running_sum > 0.0 {
            diff[tau] * tau as f64 / running_sum
        } else {
            1.0
        };
    }

    // Absolute threshold: the first local minimum whose CMNDF dips below
    // `THRESHOLD`. Start at tau=2 to skip the always-near-zero-lag region.
    let mut tau = 2;
    let mut candidate = None;
    while tau < tau_max {
        if cmndf[tau] < THRESHOLD {
            let mut t = tau;
            while t + 1 < tau_max && cmndf[t + 1] < cmndf[t] {
                t += 1;
            }
            candidate = Some(t);
            break;
        }
        tau += 1;
    }
    let tau0 = candidate?;

    let refined_tau = parabolic_interpolate(&cmndf, tau0);
    if refined_tau <= 0.0 {
        return None;
    }
    Some(sample_rate / refined_tau)
}

/// Parabolic interpolation of the CMNDF minimum around `tau0` — pure
/// arithmetic, no transcendentals, standard YIN refinement step.
fn parabolic_interpolate(cmndf: &[f64], tau0: usize) -> f64 {
    if tau0 == 0 || tau0 + 1 >= cmndf.len() {
        return tau0 as f64;
    }
    let (s0, s1, s2) = (cmndf[tau0 - 1], cmndf[tau0], cmndf[tau0 + 1]);
    let denom = 2.0 * (2.0 * s1 - s2 - s0);
    if denom == 0.0 {
        return tau0 as f64;
    }
    tau0 as f64 + (s2 - s0) / denom
}

/// `midi_hz(n) = 440 * 2^((n - 69) / 12)`, the equal-tempered A440 scale.
fn midi_hz(n: i32) -> f64 {
    440.0 * libm::exp2((f64::from(n) - 69.0) / 12.0)
}

/// Nearest equal-tempered MIDI note number to `f0_hz`. Exposed crate-wide so
/// `segments`/`digest` can classify a per-window or per-buffer median f0 the
/// same way this module classifies a per-run one.
pub(crate) fn nearest_midi(f0_hz: f64) -> i32 {
    (69.0 + 12.0 * libm::log2(f0_hz / 440.0)).round() as i32
}

/// Deviation of `f0_hz` from `midi`'s pitch, in cents. See [`nearest_midi`]
/// on visibility.
pub(crate) fn cents_off(f0_hz: f64, midi: i32) -> f64 {
    1200.0 * libm::log2(f0_hz / midi_hz(midi))
}

/// Median of `values`. Exposed crate-wide for `digest`'s display-bucket
/// aggregation (median f0 of the voiced segments in a merged row).
pub(crate) fn median(values: &mut [f64]) -> Option<f64> {
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

/// Group contiguous `Some` runs of `f0s` into [`PitchSegment`]s, each with
/// its own median f0. Segment boundaries are frame-start times, so
/// consecutive segments tile without gaps or overlap.
fn segment_runs(f0s: &[Option<f64>], sample_rate: u32) -> Vec<PitchSegment> {
    let hop_ms = HOP as f64 / f64::from(sample_rate) * 1000.0;
    let mut segments = Vec::new();
    let mut i = 0;
    while i < f0s.len() {
        if f0s[i].is_none() {
            i += 1;
            continue;
        }
        let run_start = i;
        let mut run = Vec::new();
        while let Some(v) = f0s.get(i).copied().flatten() {
            run.push(v);
            i += 1;
        }
        let f0_hz = median(&mut run).expect("run is non-empty by construction");
        let midi_nearest = nearest_midi(f0_hz);
        segments.push(PitchSegment {
            start_ms: run_start as f64 * hop_ms,
            end_ms: i as f64 * hop_ms,
            f0_hz,
            midi_nearest,
            cents_off: cents_off(f0_hz, midi_nearest),
        });
    }
    segments
}
