//! Feature-space diff between two [`Report`]/[`SegmentTimeline`] pairs: "did
//! my change do what I meant" without a human ear. Byte-identity of raw
//! samples is a decision for the *caller* to make on [`Audio`] directly —
//! see [`samples_identical`] — this module only compares derived features.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::audio::Audio;
use crate::report::{Mode, PitchClass, Report};
use crate::segments::SegmentTimeline;

/// Schema version of [`CompareReport`]'s JSON form.
pub const COMPARE_SCHEMA_VERSION: u32 = 1;

/// Tier-2 equivalence tolerance for integrated LUFS, LU (the workspace's
/// three-tier determinism contract, `docs/determinism.md`).
const LUFS_TOL: f64 = 0.1;
/// Tier-2 equivalence tolerance for matched onset offsets, ms.
const ONSET_TOL_MS: f64 = 2.0;
/// Search window for greedy onset matching, ms — generous relative to
/// [`ONSET_TOL_MS`] so a shifted-but-corresponding onset still matches (and
/// therefore still fails the tolerance check as one *matched-but-late*
/// onset), rather than showing up as one spurious unmatched onset on each
/// side.
const ONSET_MATCH_WINDOW_MS: f64 = 50.0;
/// Tier-2 equivalence tolerance for median pitch, cents.
const PITCH_TOL_CENTS: f64 = 5.0;

/// One side of a [`compare`]: a probe report plus its segment timeline, both
/// computed over the same buffer.
#[derive(Debug, Clone, Copy)]
pub struct Analysis<'a> {
    /// The whole-file probe report.
    pub report: &'a Report,
    /// The windowed segment timeline over the same buffer.
    pub timeline: &'a SegmentTimeline,
}

/// Feature-space diff of two analyses, `schema_version: 1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareReport {
    /// Schema version; see [`COMPARE_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// `b.duration_ms - a.duration_ms`.
    pub duration_delta_ms: f64,
    /// Loudness deltas.
    pub loudness: LoudnessDelta,
    /// Onset matching.
    pub onsets: OnsetMatch,
    /// Median pitch delta. `None` unless both sides have a voiced median
    /// f0.
    pub pitch: Option<PitchDelta>,
    /// Key estimates on both sides and whether they differ.
    pub key: KeyDelta,
    /// Per-segment RMS deltas over the overlapping timeline range.
    pub segments: SegmentDelta,
    /// The overall verdict.
    pub verdict: Verdict,
}

/// Loudness deltas, `b - a`. `None` where either side is undefined
/// (silence).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LoudnessDelta {
    /// Integrated LUFS delta, LU.
    pub integrated_lufs_delta: Option<f64>,
    /// True peak delta, dB.
    pub true_peak_dbtp_delta: Option<f64>,
}

/// Greedy nearest-neighbor onset matching within [`ONSET_MATCH_WINDOW_MS`],
/// each onset used at most once.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OnsetMatch {
    /// Count of matched onset pairs.
    pub matched: usize,
    /// Mean absolute offset of matched pairs, ms. `None` if `matched == 0`.
    pub mean_abs_offset_ms: Option<f64>,
    /// Max absolute offset of matched pairs, ms. `None` if `matched == 0`.
    pub max_abs_offset_ms: Option<f64>,
    /// Count of `a` onsets with no match in `b`.
    pub unmatched_a: usize,
    /// Count of `b` onsets with no match in `a`.
    pub unmatched_b: usize,
}

/// Median-f0 delta, cents, `b` relative to `a`. Only produced when both
/// sides have a voiced median f0.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PitchDelta {
    /// `1200 * log2(b_f0 / a_f0)`.
    pub cents: f64,
}

/// Key estimates on both sides and whether they differ (tonic or mode).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyDelta {
    /// Whether tonic or mode differs between `a` and `b`.
    pub changed: bool,
    /// `a`'s key estimate.
    pub a: KeySummary,
    /// `b`'s key estimate.
    pub b: KeySummary,
}

/// One side's key estimate, restated for [`KeyDelta`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct KeySummary {
    /// Estimated tonic.
    pub tonic: PitchClass,
    /// Estimated mode.
    pub mode: Mode,
    /// Correlation confidence (see [`crate::KeyReport::confidence`]).
    pub confidence: f64,
}

/// Per-segment RMS delta over the overlapping index range
/// (`0..min(a.segments.len(), b.segments.len())`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentDelta {
    /// Largest absolute RMS delta, dB, over the overlapping range. `None`
    /// if the range is empty or every overlapping pair has an undefined
    /// RMS (digital silence) on at least one side.
    pub max_abs_rms_delta_db: Option<f64>,
    /// Index (into the overlapping range) where `max_abs_rms_delta_db`
    /// occurs.
    pub max_abs_rms_delta_index: Option<usize>,
    /// `b - a` RMS delta, dB, per overlapping segment index. `None` per
    /// entry where either side's RMS is undefined.
    pub rms_delta_db: Vec<Option<f64>>,
}

/// The compare verdict: how close `a` and `b` are, in the workspace's
/// determinism-contract vocabulary (`docs/determinism.md`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Verdict {
    /// `a` and `b` are the same PCM, sample rate, and channel count — the
    /// strongest possible verdict, set only via
    /// [`compare_with_identity`]/the caller's own [`samples_identical`]
    /// check on the source [`Audio`].
    ByteIdentical,
    /// Every dimension is inside the workspace's Tier-2 cross-platform
    /// tolerance: integrated LUFS within 0.1 LU, every onset matched within
    /// 2 ms with none unmatched, pitch within 5 cents, and the same key.
    Tier2Equivalent,
    /// At least one dimension is outside tolerance; `dimensions` names each
    /// one (`"loudness"`, `"onsets"`, `"pitch"`, `"key"`), in check order.
    Different {
        /// Names of the out-of-tolerance dimensions.
        dimensions: Vec<String>,
    },
}

/// `compare_with_identity(a, b, false)` — feature-space diff without a
/// prior byte-identity check.
pub fn compare(a: Analysis, b: Analysis) -> CompareReport {
    compare_with_identity(a, b, false)
}

/// Feature-space diff of `a` vs `b`. `byte_identical` should come from the
/// caller's own [`samples_identical`] check (or another source of truth,
/// e.g. a golden-hash comparison) on the underlying [`Audio`] buffers —
/// when `true`, `verdict` is [`Verdict::ByteIdentical`] regardless of the
/// computed deltas (which are still populated, for callers that want the
/// numbers anyway).
pub fn compare_with_identity(a: Analysis, b: Analysis, byte_identical: bool) -> CompareReport {
    let duration_delta_ms = b.report.source.duration_ms - a.report.source.duration_ms;

    let loudness = LoudnessDelta {
        integrated_lufs_delta: delta_opt(
            a.report.loudness.integrated_lufs,
            b.report.loudness.integrated_lufs,
        ),
        true_peak_dbtp_delta: delta_opt(
            a.report.loudness.true_peak_dbtp,
            b.report.loudness.true_peak_dbtp,
        ),
    };

    let onset_match = match_onsets(
        &a.report.onsets.times_ms,
        &b.report.onsets.times_ms,
        ONSET_MATCH_WINDOW_MS,
    );
    let pitch = pitch_delta(a.report, b.report);
    let key = key_delta(a.report, b.report);
    let segments = segment_rms_delta(a.timeline, b.timeline);

    let verdict = if byte_identical {
        Verdict::ByteIdentical
    } else {
        verdict_from(a.report, b.report, &onset_match, &pitch, &key)
    };

    CompareReport {
        schema_version: COMPARE_SCHEMA_VERSION,
        duration_delta_ms,
        loudness,
        onsets: onset_match,
        pitch,
        key,
        segments,
        verdict,
    }
}

/// Whether two [`Audio`] buffers hold identical PCM — samples, sample rate,
/// and channel count all equal. The caller-side check that feeds
/// [`compare_with_identity`]'s `byte_identical` flag.
pub fn samples_identical(a: &Audio, b: &Audio) -> bool {
    a.sample_rate == b.sample_rate && a.channels == b.channels && a.samples == b.samples
}

fn delta_opt(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(av), Some(bv)) => Some(bv - av),
        _ => None,
    }
}

fn pitch_delta(a: &Report, b: &Report) -> Option<PitchDelta> {
    match (a.pitch.median_f0_hz, b.pitch.median_f0_hz) {
        (Some(fa), Some(fb)) if fa > 0.0 && fb > 0.0 => Some(PitchDelta {
            cents: 1200.0 * libm::log2(fb / fa),
        }),
        _ => None,
    }
}

fn key_delta(a: &Report, b: &Report) -> KeyDelta {
    let changed = a.key.tonic != b.key.tonic || a.key.mode != b.key.mode;
    KeyDelta {
        changed,
        a: KeySummary {
            tonic: a.key.tonic,
            mode: a.key.mode,
            confidence: a.key.confidence,
        },
        b: KeySummary {
            tonic: b.key.tonic,
            mode: b.key.mode,
            confidence: b.key.confidence,
        },
    }
}

fn segment_rms_delta(a: &SegmentTimeline, b: &SegmentTimeline) -> SegmentDelta {
    let n = a.segments.len().min(b.segments.len());
    let mut deltas = Vec::with_capacity(n);
    let mut max_abs: Option<f64> = None;
    let mut max_idx: Option<usize> = None;
    for i in 0..n {
        let delta = delta_opt(a.segments[i].rms_dbfs, b.segments[i].rms_dbfs);
        if let Some(d) = delta {
            let ad = d.abs();
            if max_abs.is_none_or(|m| ad > m) {
                max_abs = Some(ad);
                max_idx = Some(i);
            }
        }
        deltas.push(delta);
    }
    SegmentDelta {
        max_abs_rms_delta_db: max_abs,
        max_abs_rms_delta_index: max_idx,
        rms_delta_db: deltas,
    }
}

/// Greedy nearest-neighbor onset matching: candidate pairs within
/// `window_ms` are matched shortest-offset-first (ties broken by
/// `a`-then-`b` index, for determinism), each onset used at most once.
fn match_onsets(a_times: &[f64], b_times: &[f64], window_ms: f64) -> OnsetMatch {
    let mut candidates = Vec::new();
    for (i, &ta) in a_times.iter().enumerate() {
        for (j, &tb) in b_times.iter().enumerate() {
            let offset = (tb - ta).abs();
            if offset <= window_ms {
                candidates.push((i, j, offset));
            }
        }
    }
    candidates.sort_by(|x, y| x.2.total_cmp(&y.2).then(x.0.cmp(&y.0)).then(x.1.cmp(&y.1)));

    let mut used_a = vec![false; a_times.len()];
    let mut used_b = vec![false; b_times.len()];
    let mut offsets = Vec::new();
    for (i, j, offset) in candidates {
        if !used_a[i] && !used_b[j] {
            used_a[i] = true;
            used_b[j] = true;
            offsets.push(offset);
        }
    }

    let matched = offsets.len();
    let mean_abs_offset_ms = (matched > 0).then(|| offsets.iter().sum::<f64>() / matched as f64);
    let max_abs_offset_ms = max_of(offsets.iter().copied());

    OnsetMatch {
        matched,
        mean_abs_offset_ms,
        max_abs_offset_ms,
        unmatched_a: used_a.iter().filter(|&&u| !u).count(),
        unmatched_b: used_b.iter().filter(|&&u| !u).count(),
    }
}

fn max_of(values: impl Iterator<Item = f64>) -> Option<f64> {
    values.fold(None, |acc: Option<f64>, v| {
        Some(acc.map_or(v, |m| m.max(v)))
    })
}

/// The [`Verdict`]: [`Verdict::Tier2Equivalent`] iff loudness, onsets,
/// pitch, and key are all within the workspace's Tier-2 tolerances (see
/// [`Verdict`]'s docs), else [`Verdict::Different`] naming every
/// out-of-tolerance dimension.
fn verdict_from(
    a: &Report,
    b: &Report,
    onsets: &OnsetMatch,
    pitch: &Option<PitchDelta>,
    key: &KeyDelta,
) -> Verdict {
    let mut dimensions = Vec::new();

    let loudness_ok = match (a.loudness.integrated_lufs, b.loudness.integrated_lufs) {
        (None, None) => true,
        (Some(av), Some(bv)) => (bv - av).abs() <= LUFS_TOL,
        _ => false,
    };
    if !loudness_ok {
        dimensions.push("loudness".to_string());
    }

    let onsets_ok = onsets.unmatched_a == 0
        && onsets.unmatched_b == 0
        && onsets.max_abs_offset_ms.is_none_or(|m| m <= ONSET_TOL_MS);
    if !onsets_ok {
        dimensions.push("onsets".to_string());
    }

    let pitch_ok = pitch
        .as_ref()
        .is_none_or(|p| p.cents.abs() <= PITCH_TOL_CENTS);
    if !pitch_ok {
        dimensions.push("pitch".to_string());
    }

    if key.changed {
        dimensions.push("key".to_string());
    }

    if dimensions.is_empty() {
        Verdict::Tier2Equivalent
    } else {
        Verdict::Different { dimensions }
    }
}

/// Compact deterministic text rendering of a [`CompareReport`]: verdict
/// first, then one line per dimension with `a`/`b`/delta.
pub fn compare_text(r: &CompareReport) -> String {
    let mut out = String::new();

    writeln!(out, "verdict: {}", verdict_text(&r.verdict)).expect("String write is infallible");
    writeln!(out, "duration     a->b {:+.1} ms", r.duration_delta_ms)
        .expect("String write is infallible");
    writeln!(
        out,
        "loudness     integrated {}  true_peak {}",
        fmt_delta(r.loudness.integrated_lufs_delta, "LU"),
        fmt_delta(r.loudness.true_peak_dbtp_delta, "dB"),
    )
    .expect("String write is infallible");
    writeln!(
        out,
        "onsets       matched={}  mean_offset={}  max_offset={}  unmatched_a={}  unmatched_b={}",
        r.onsets.matched,
        fmt_ms(r.onsets.mean_abs_offset_ms),
        fmt_ms(r.onsets.max_abs_offset_ms),
        r.onsets.unmatched_a,
        r.onsets.unmatched_b,
    )
    .expect("String write is infallible");
    match &r.pitch {
        Some(p) => writeln!(out, "pitch        delta {:+.1} cents", p.cents)
            .expect("String write is infallible"),
        None => writeln!(out, "pitch        delta -  (not both voiced)")
            .expect("String write is infallible"),
    }
    writeln!(
        out,
        "key          a={} {} (conf {:.2})  b={} {} (conf {:.2})  changed={}",
        r.key.a.tonic.name(),
        mode_text(r.key.a.mode),
        r.key.a.confidence,
        r.key.b.tonic.name(),
        mode_text(r.key.b.mode),
        r.key.b.confidence,
        r.key.changed,
    )
    .expect("String write is infallible");
    match (
        r.segments.max_abs_rms_delta_db,
        r.segments.max_abs_rms_delta_index,
    ) {
        (Some(delta), Some(idx)) => writeln!(
            out,
            "segments     max_abs_rms_delta {delta:.2} dB at idx={idx}"
        )
        .expect("String write is infallible"),
        _ => writeln!(out, "segments     max_abs_rms_delta -").expect("String write is infallible"),
    }

    out
}

fn verdict_text(v: &Verdict) -> String {
    match v {
        Verdict::ByteIdentical => "byte-identical".to_string(),
        Verdict::Tier2Equivalent => "tier2-equivalent".to_string(),
        Verdict::Different { dimensions } => format!("different ({})", dimensions.join(", ")),
    }
}

fn fmt_delta(v: Option<f64>, unit: &str) -> String {
    match v {
        Some(x) => format!("{x:+.2} {unit}"),
        None => "-".to_string(),
    }
}

fn fmt_ms(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.2} ms"),
        None => "-".to_string(),
    }
}

fn mode_text(mode: Mode) -> &'static str {
    match mode {
        Mode::Major => "major",
        Mode::Minor => "minor",
    }
}
