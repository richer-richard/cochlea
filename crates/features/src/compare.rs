//! Feature-space diff between two [`Report`]/[`SegmentTimeline`] pairs: "did
//! my change do what I meant" without a human ear. Byte-identity of raw
//! samples is a decision for the *caller* to make on [`Audio`] directly —
//! see [`samples_identical`] — this module only compares derived features.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::audio::Audio;
use crate::report::{Mode, PitchClass, Report};
use crate::segments::SegmentTimeline;
use crate::util::{max_of, mode_name};

/// Schema version of [`CompareReport`]'s JSON form.
///
/// - `1`: initial shape.
/// - `2`: `clear_rhythm_changed` moved from `tempo` to the new `rhythm`
///   delta (tracking the probe report's own v3 tempo/rhythm split), which
///   also carries `grid_alignment_delta`; `tempo` gained `stability_delta`.
/// - `3`: `rhythm` gained `grid_changed` (straight-vs-triplet feel change);
///   new `timbre` delta (MFCC spectral-shape distance) — tracking the
///   probe report's v4 additions.
pub const COMPARE_SCHEMA_VERSION: u32 = 3;

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
    /// Tempo/beat-tracking deltas. Informational — not part of `verdict`
    /// (the workspace's Tier-2 contract, `docs/determinism.md`, doesn't
    /// define a tempo tolerance).
    pub tempo: TempoDelta,
    /// Rhythm (grid-alignment) deltas. Informational, same reasoning as
    /// `tempo`.
    pub rhythm: RhythmDelta,
    /// Timbre (MFCC) distance. Informational, same reasoning as `tempo`.
    pub timbre: TimbreDelta,
    /// Stereo-image deltas; `None` unless both sides are stereo.
    /// Informational, same reasoning as `tempo`.
    pub stereo: Option<StereoDelta>,
    /// Structure-detection deltas. Informational, same reasoning as
    /// `tempo`.
    pub structure: StructureDelta,
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
    /// EBU R128 loudness range (LRA) delta, LU.
    pub lra_delta: Option<f64>,
}

/// Tempo/beat-tracking deltas, `b` relative to `a`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TempoDelta {
    /// `b.bpm - a.bpm`. `None` unless both sides have a detected tempo.
    pub bpm_delta: Option<f64>,
    /// `b.stability - a.stability`. `None` unless both sides measured one.
    pub stability_delta: Option<f64>,
}

/// Rhythm (grid-alignment) deltas, `b` relative to `a`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RhythmDelta {
    /// Whether `clear_rhythm` differs between `a` and `b`.
    pub clear_rhythm_changed: bool,
    /// `b.grid_alignment - a.grid_alignment`. `None` unless both sides
    /// have a usable beat grid.
    pub grid_alignment_delta: Option<f64>,
    /// Whether the winning subdivision hypothesis (straight vs triplet)
    /// differs between `a` and `b` — a feel change (straightened or swung),
    /// distinct from a tightness change. `false` when either side has no
    /// grid at all (that case already reads as `grid_alignment_delta:
    /// null`).
    pub grid_changed: bool,
}

/// Timbre distance, `b` relative to `a`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TimbreDelta {
    /// Euclidean distance between the two sides' mean MFCC vectors,
    /// *excluding `c0`* (`c0` is log-energy, i.e. loudness — already
    /// covered by the loudness delta — so this number is spectral shape
    /// only). Same instrument re-rendered measures near `0`; a sine-vs-saw
    /// swap at matched loudness measures well above it. Unitless, in log
    /// mel-energy space; useful as an ordering, not an absolute scale.
    /// `None` unless both sides produced a timbre digest.
    pub mfcc_distance: Option<f64>,
}

/// Stereo-image deltas, `b - a`. Only produced when both sides are stereo
/// (see [`CompareReport::stereo`]).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StereoDelta {
    /// `b.width - a.width`.
    pub width_delta: f64,
    /// `b.correlation - a.correlation`. `None` unless both sides have a
    /// defined correlation.
    pub correlation_delta: Option<f64>,
    /// `b.balance - a.balance`. `None` unless both sides have a defined
    /// balance.
    pub balance_delta: Option<f64>,
}

/// Structure-detection deltas, `b` relative to `a`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StructureDelta {
    /// `b.section_count as i64 - a.section_count as i64`.
    pub section_count_delta: i64,
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
    /// tolerance: duration within 1 ms, integrated LUFS within 0.1 LU,
    /// every onset matched within 2 ms with none unmatched, pitch within 5
    /// cents with matching voicing (both sides voiced, or both unvoiced),
    /// and the same key.
    Tier2Equivalent,
    /// At least one dimension is outside tolerance; `dimensions` names each
    /// one (`"duration"`, `"loudness"`, `"onsets"`, `"pitch"`, `"key"`), in
    /// check order.
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
        lra_delta: delta_opt(a.report.loudness.lra, b.report.loudness.lra),
    };

    let onset_match = match_onsets(
        &a.report.onsets.times_ms,
        &b.report.onsets.times_ms,
        ONSET_MATCH_WINDOW_MS,
    );
    let pitch = pitch_delta(a.report, b.report);
    let key = key_delta(a.report, b.report);
    let segments = segment_rms_delta(a.timeline, b.timeline);
    let tempo = tempo_delta(a.report, b.report);
    let rhythm = rhythm_delta(a.report, b.report);
    let timbre = timbre_delta(a.report, b.report);
    let stereo = stereo_delta(a.report, b.report);
    let structure = StructureDelta {
        section_count_delta: b.report.structure.section_count as i64
            - a.report.structure.section_count as i64,
    };

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
        tempo,
        rhythm,
        timbre,
        stereo,
        structure,
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

fn tempo_delta(a: &Report, b: &Report) -> TempoDelta {
    TempoDelta {
        bpm_delta: delta_opt(a.tempo.bpm, b.tempo.bpm),
        stability_delta: delta_opt(a.tempo.stability, b.tempo.stability),
    }
}

fn rhythm_delta(a: &Report, b: &Report) -> RhythmDelta {
    let grid_changed = match (a.rhythm.grid, b.rhythm.grid) {
        (Some(ga), Some(gb)) => ga != gb,
        _ => false,
    };
    RhythmDelta {
        clear_rhythm_changed: a.rhythm.clear_rhythm != b.rhythm.clear_rhythm,
        grid_alignment_delta: delta_opt(a.rhythm.grid_alignment, b.rhythm.grid_alignment),
        grid_changed,
    }
}

/// Euclidean distance between mean MFCC vectors, skipping `c0` (see
/// [`TimbreDelta::mfcc_distance`]).
fn timbre_delta(a: &Report, b: &Report) -> TimbreDelta {
    let mfcc_distance = match (&a.timbre, &b.timbre) {
        (Some(ta), Some(tb)) => {
            let sum_sq: f64 = ta
                .mfcc_mean
                .iter()
                .zip(tb.mfcc_mean.iter())
                .skip(1)
                .map(|(&x, &y)| (x - y) * (x - y))
                .sum();
            Some(libm::sqrt(sum_sq))
        }
        _ => None,
    };
    TimbreDelta { mfcc_distance }
}

fn stereo_delta(a: &Report, b: &Report) -> Option<StereoDelta> {
    let (a_stereo, b_stereo) = (a.stereo.as_ref()?, b.stereo.as_ref()?);
    Some(StereoDelta {
        width_delta: b_stereo.width - a_stereo.width,
        correlation_delta: delta_opt(a_stereo.correlation, b_stereo.correlation),
        balance_delta: delta_opt(a_stereo.balance, b_stereo.balance),
    })
}

fn segment_rms_delta(a: &SegmentTimeline, b: &SegmentTimeline) -> SegmentDelta {
    // Index-aligned comparison only means anything when both timelines
    // were built on the same window width; mismatched timelines produce an
    // empty (not wrong) segment delta.
    if a.window_ms != b.window_ms {
        return SegmentDelta {
            max_abs_rms_delta_db: None,
            max_abs_rms_delta_index: None,
            rms_delta_db: Vec::new(),
        };
    }
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

    // Two same-input cross-platform renders have identical sample counts;
    // any real duration difference means different material, however
    // feature-similar (two silent files of different lengths must not read
    // as equivalent).
    if (b.source.duration_ms - a.source.duration_ms).abs() > 1.0 {
        dimensions.push("duration".to_string());
    }

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

    // A voicing mismatch (one side has a voiced median f0, the other has
    // none at all) is a pitch difference even though no cents delta can be
    // computed — without this, a tone and a noise bed at matching loudness
    // could read as equivalent.
    let voicing_matches = a.pitch.median_f0_hz.is_some() == b.pitch.median_f0_hz.is_some();
    let pitch_ok = voicing_matches
        && pitch
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
        "loudness     integrated {}  true_peak {}  lra {}",
        fmt_delta(r.loudness.integrated_lufs_delta, "LU"),
        fmt_delta(r.loudness.true_peak_dbtp_delta, "dB"),
        fmt_delta(r.loudness.lra_delta, "LU"),
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
        mode_name(r.key.a.mode),
        r.key.a.confidence,
        r.key.b.tonic.name(),
        mode_name(r.key.b.mode),
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
    writeln!(
        out,
        "tempo        bpm {}  stability {}",
        fmt_delta(r.tempo.bpm_delta, "bpm"),
        fmt_delta(r.tempo.stability_delta, ""),
    )
    .expect("String write is infallible");
    writeln!(
        out,
        "rhythm       clear_rhythm_changed={}  grid_align {}  grid_changed={}",
        r.rhythm.clear_rhythm_changed,
        fmt_delta(r.rhythm.grid_alignment_delta, ""),
        r.rhythm.grid_changed,
    )
    .expect("String write is infallible");
    writeln!(
        out,
        "timbre       mfcc_distance {}",
        fmt_dash_plain(r.timbre.mfcc_distance),
    )
    .expect("String write is infallible");
    match &r.stereo {
        Some(s) => writeln!(
            out,
            "stereo       width {:+.2}  correlation {}  balance {}",
            s.width_delta,
            fmt_delta(s.correlation_delta, ""),
            fmt_delta(s.balance_delta, ""),
        )
        .expect("String write is infallible"),
        None => {
            writeln!(out, "stereo       -  (not both stereo)").expect("String write is infallible")
        }
    }
    writeln!(
        out,
        "structure    section_count {:+}",
        r.structure.section_count_delta,
    )
    .expect("String write is infallible");

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
        // Unitless callers (correlation/balance) pass "" — appending the
        // separator anyway would bake a stray trailing space into a text
        // format that promises to be tidy and deterministic.
        Some(x) if unit.is_empty() => format!("{x:+.2}"),
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

/// Unsigned undefined-capable formatting — for magnitudes (distances)
/// where a forced `+` sign would misread as a direction.
fn fmt_dash_plain(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.2}"),
        None => "-".to_string(),
    }
}
