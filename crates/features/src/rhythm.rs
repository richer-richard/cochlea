//! Rhythm (pattern) analysis: how the detected onsets relate to the
//! detected pulse. The [`crate::tempo`] module answers "how fast, and how
//! periodic is the envelope"; this module answers "do the hits actually
//! land on that pulse's grid, and where on it" — the two are deliberately
//! separate reports because they change independently: a drum solo holds a
//! steady tempo while its rhythm turns confusing, and a tempo change can
//! arrive while the figure stays a plain backbeat.
//!
//! Everything here is grid geometry over already-computed parts (onset
//! times + beat grid) — no new signal pass, so it's effectively free once
//! `probe()` has run tempo and onsets.
//!
//! Method: each consecutive beat-grid interval is split into
//! [`SUBDIVISIONS_PER_BEAT`] equal subdivisions (16ths at the beat level —
//! fine enough for hats/ghost notes, coarse enough that random onsets don't
//! accidentally align; a triplet feel will read a little lower, honestly,
//! rather than being force-fit). An onset is *aligned* when it falls within
//! [`ALIGN_TOL_FRACTION`] of a subdivision interval from the nearest grid
//! point (floored at [`ALIGN_TOL_MIN_MS`] — the onset detector itself is
//! only accurate to a few ms, and human timing jitter under ~10 ms reads as
//! tight, not off-grid). `grid_alignment` is the aligned fraction;
//! `offbeat_ratio` is, among aligned onsets, the fraction whose nearest
//! grid point is *not* an integer beat — a syncopation signal, not a
//! judgment.

use serde::{Deserialize, Serialize};

use crate::report::OnsetsReport;
use crate::tempo::TempoReport;

/// Subdivisions per beat for the alignment grid (4 = sixteenth notes at
/// the detected-beat level).
const SUBDIVISIONS_PER_BEAT: usize = 4;
/// An onset counts as on-grid within this fraction of one subdivision
/// interval of the nearest grid point. With 4 subdivisions per beat, a
/// uniformly random onset would align ~30% of the time at this tolerance —
/// the [`CLEAR_RHYTHM_MIN_ALIGNMENT`] threshold sits far above that floor.
const ALIGN_TOL_FRACTION: f64 = 0.15;
/// Alignment tolerance floor, ms — onset detection is frame-quantized
/// (~5.3 ms at 48 kHz) and human "tight" playing jitters several ms, so
/// tolerances below this would measure the detector, not the rhythm.
const ALIGN_TOL_MIN_MS: f64 = 10.0;

/// `clear_rhythm` thresholds — see [`analyze_rhythm`] for the exact rule.
/// Calibrated on this crate's fixtures (values printed by the
/// `calibration_readings` test in `tests/rhythm.rs`): click tracks measure
/// `grid_alignment` = 1.0 even under ±30 ms humanized jitter (the DP grid
/// follows human timing); uniformly random onset times measure ≈ 0.57 —
/// the DP grid's bounded elasticity lifts them above the ~0.3 rigid-grid
/// geometric floor, but no further. `0.7` splits those populations.
const CLEAR_RHYTHM_MIN_ALIGNMENT: f64 = 0.7;
/// Pulse-clarity floor for `clear_rhythm`, in [`TempoReport::confidence`]'s
/// normalized-autocovariance units: a clean click track measures ≈ 0.96,
/// one with ±10 ms human jitter ≈ 0.51, ±30 ms ≈ 0.33; random onset times
/// ≈ 0.10. This floor exists to reject the no-pulse case, not to grade
/// groove tightness — that's `grid_alignment`'s job.
const CLEAR_RHYTHM_MIN_CONFIDENCE: f64 = 0.1;
/// Minimum onset rate for `clear_rhythm`: a handful of widely-spaced hits
/// can align perfectly without being a rhythm an agent should trust.
const CLEAR_RHYTHM_MIN_ONSET_RATE_PER_S: f64 = 0.5;

/// Rhythm (pattern) analysis result — the onsets' relationship to the
/// detected beat grid. See the module docs for the tempo/rhythm split.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RhythmReport {
    /// Fraction of detected onsets that land on the beat-subdivision grid,
    /// `0.0..=1.0`. `None` when there is no usable grid (no detected
    /// tempo, or fewer than two placed beats) or no onsets to classify.
    pub grid_alignment: Option<f64>,
    /// Among *aligned* onsets, the fraction whose nearest grid point is not
    /// an integer beat — high values mean the hits live on the off-beats
    /// and subdivisions (syncopation, hat-driven grooves), low values mean
    /// four-square on-the-beat playing. Descriptive, not a judgment.
    /// `None` whenever `grid_alignment` is `None` or zero onsets aligned.
    pub offbeat_ratio: Option<f64>,
    /// Onsets per second over the whole buffer.
    pub onset_rate_per_s: f64,
    /// Whether an agent can trust the tempo/beat grid as a real,
    /// articulated rhythm: a detected pulse (confidence above a floor), a
    /// busy-enough surface (onset rate), and hits that actually sit on the
    /// grid (`grid_alignment`). See [`analyze_rhythm`] for the exact rule.
    pub clear_rhythm: bool,
}

fn degenerate_report(onset_rate_per_s: f64) -> RhythmReport {
    RhythmReport {
        grid_alignment: None,
        offbeat_ratio: None,
        onset_rate_per_s,
        clear_rhythm: false,
    }
}

/// Analyze the onsets' relationship to the beat grid (see the module docs
/// for the method).
///
/// **`clear_rhythm`**: `true` iff a tempo was detected with
/// `tempo.confidence >= 0.1`, the onset rate is at least `0.5`/s, and
/// `grid_alignment >= 0.7`. This replaces the pre-0.2.0 rule (which
/// thresholded a mass-fraction confidence that structurally punished
/// multi-level periodicity — real grooves always carry several metrical
/// levels, so it flagged everything but bare click tracks as unclear).
/// Alignment asks the right question: not "is the envelope's energy
/// concentrated at one lag" but "do the hits land where the pulse says".
///
/// Degenerate input (no tempo, a beat grid with fewer than two beats, no
/// onsets, or a zero/negative duration) never panics — grid fields come
/// back `None` and `clear_rhythm` is `false`.
pub fn analyze_rhythm(
    onsets: &OnsetsReport,
    tempo: &TempoReport,
    duration_s: f64,
) -> RhythmReport {
    let onset_rate_per_s = if duration_s > 0.0 {
        onsets.count as f64 / duration_s
    } else {
        0.0
    };

    if tempo.bpm.is_none() || tempo.beats_ms.len() < 2 || onsets.times_ms.is_empty() {
        return degenerate_report(onset_rate_per_s);
    }

    let (aligned, offbeat) = classify_onsets(&onsets.times_ms, &tempo.beats_ms);
    let grid_alignment = aligned as f64 / onsets.times_ms.len() as f64;
    let offbeat_ratio = (aligned > 0).then(|| offbeat as f64 / aligned as f64);

    let clear_rhythm = tempo.confidence >= CLEAR_RHYTHM_MIN_CONFIDENCE
        && onset_rate_per_s >= CLEAR_RHYTHM_MIN_ONSET_RATE_PER_S
        && grid_alignment >= CLEAR_RHYTHM_MIN_ALIGNMENT;

    RhythmReport {
        grid_alignment: Some(grid_alignment),
        offbeat_ratio,
        onset_rate_per_s,
        clear_rhythm,
    }
}

/// Count `(aligned, aligned_and_offbeat)` onsets against the subdivision
/// grid built over `beats_ms`. Onsets before the first beat or after the
/// last are classified against the nearest edge interval's grid, extended
/// by up to one beat on each side (the DP grid starts at the first strong
/// onset, so pickup hits just before it shouldn't all read as off-grid).
fn classify_onsets(onset_times_ms: &[f64], beats_ms: &[f64]) -> (usize, usize) {
    let mut aligned = 0usize;
    let mut offbeat = 0usize;

    for &t in onset_times_ms {
        if let Some(sub_index) = nearest_grid_point(t, beats_ms) {
            aligned += 1;
            if sub_index % SUBDIVISIONS_PER_BEAT != 0 {
                offbeat += 1;
            }
        }
    }
    (aligned, offbeat)
}

/// The subdivision index (0 = an integer beat) of the grid point nearest
/// `t_ms`, if `t_ms` is within tolerance of it; `None` otherwise. The grid
/// covers `beats_ms[0] - one beat .. beats_ms[last] + one beat`, with each
/// real beat-to-beat interval subdivided individually (so a DP grid that
/// flexes with the music keeps its subdivisions honest), and the two
/// extension intervals reusing their adjacent interval's length.
fn nearest_grid_point(t_ms: f64, beats_ms: &[f64]) -> Option<usize> {
    let first = beats_ms[0];
    let last = beats_ms[beats_ms.len() - 1];
    let first_interval = beats_ms[1] - first;
    let last_interval = last - beats_ms[beats_ms.len() - 2];

    // Locate the interval containing t (with one-beat extensions).
    let (interval_start, interval_len) = if t_ms < first {
        if first_interval <= 0.0 || t_ms < first - first_interval {
            return None;
        }
        (first - first_interval, first_interval)
    } else if t_ms >= last {
        if last_interval <= 0.0 || t_ms > last + last_interval {
            return None;
        }
        (last, last_interval)
    } else {
        // Binary search for the beat at or before t.
        let i = match beats_ms.binary_search_by(|b| b.total_cmp(&t_ms)) {
            Ok(i) => i,
            Err(i) => i - 1, // t >= first, so i >= 1 here
        };
        let i = i.min(beats_ms.len() - 2);
        (beats_ms[i], beats_ms[i + 1] - beats_ms[i])
    };
    if interval_len <= 0.0 {
        return None;
    }

    let sub_len = interval_len / SUBDIVISIONS_PER_BEAT as f64;
    let pos = (t_ms - interval_start) / sub_len;
    let nearest = libm::round(pos);
    let dist_ms = (pos - nearest).abs() * sub_len;
    let tol_ms = (ALIGN_TOL_FRACTION * sub_len).max(ALIGN_TOL_MIN_MS);
    if dist_ms > tol_ms {
        return None;
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "nearest is a small non-negative subdivision index by construction"
    )]
    Some(nearest as usize % SUBDIVISIONS_PER_BEAT)
}
