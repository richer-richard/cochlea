//! A compact, deterministic plain-text digest of a [`Report`] +
//! [`SegmentTimeline`], sized for LLM context windows: reading a WAV should
//! cost tens of lines of text, not megabytes of PCM or JSON. Fixed decimal
//! places, no locale, no timestamps — byte-identical for identical input.

use std::fmt::Write as _;

use crate::report::{PitchClass, Report};
use crate::segments::{Segment, SegmentTimeline};
use crate::util::{max_of, mode_name};

/// Cap on rendered timeline rows: longer timelines are merged into coarser
/// display buckets (see [`write_timeline`]) so a multi-minute file still
/// digests to well under 100 lines.
const MAX_TIMELINE_ROWS: usize = 40;
/// Minimum run length of consecutive silent display rows collapsed into one
/// summary line.
const MIN_SILENT_RUN: usize = 3;

/// Render `report` + `timeline` as a compact deterministic text digest —
/// header stats, then a fixed-width timeline table (row-capped and with
/// silent runs collapsed; see [`write_timeline`]). See the module docs for
/// the size/determinism goals.
pub fn digest_text(report: &Report, timeline: &SegmentTimeline) -> String {
    let mut out = String::new();

    writeln!(
        out,
        "cochlea digest: {:.3}s  {}ch  {}Hz",
        report.source.duration_ms / 1000.0,
        report.source.channels,
        report.source.sample_rate,
    )
    .expect("String write is infallible");

    writeln!(
        out,
        "loudness: integrated={}  momentary_max={}  true_peak={}  lra={}",
        fmt_lufs(report.loudness.integrated_lufs),
        fmt_lufs(report.loudness.momentary_max_lufs),
        fmt_dash(report.loudness.true_peak_dbtp, 2),
        fmt_dash(report.loudness.lra, 2),
    )
    .expect("String write is infallible");

    writeln!(
        out,
        "key: {} {} (conf {:.2})  pitch: voiced={:.0}%  median={}",
        report.key.tonic.name(),
        mode_name(report.key.mode),
        report.key.confidence,
        report.pitch.voiced_ratio * 100.0,
        fmt_median_pitch(report.pitch.median_f0_hz),
    )
    .expect("String write is infallible");

    writeln!(out, "{}", fmt_tempo_line(&report.tempo)).expect("String write is infallible");

    // Omitted entirely for mono input — a "stereo: mono" line would carry
    // no information a reader doesn't already have from the header.
    if let Some(stereo) = &report.stereo {
        writeln!(
            out,
            "stereo: width={:.2} corr={} bal={}",
            stereo.width,
            fmt_dash(stereo.correlation, 2),
            fmt_signed_dash(stereo.balance),
        )
        .expect("String write is infallible");
    }

    writeln!(out, "{}", fmt_structure_line(&report.structure)).expect("String write is infallible");

    let duration_s = report.source.duration_ms / 1000.0;
    let onset_rate = if duration_s > 0.0 {
        report.onsets.count as f64 / duration_s
    } else {
        0.0
    };
    writeln!(
        out,
        "onsets: count={}  rate={onset_rate:.2}/s",
        report.onsets.count,
    )
    .expect("String write is infallible");

    writeln!(
        out,
        "silence: leading={:.0}ms  trailing={:.0}ms",
        report.silence.leading_ms, report.silence.trailing_ms,
    )
    .expect("String write is infallible");

    writeln!(
        out,
        "clipping: clipped={}  over_0dbtp={}",
        report.clipping.clipped_samples, report.clipping.true_peak_over_0dbtp,
    )
    .expect("String write is infallible");

    write_timeline(&mut out, timeline);

    out
}

/// One display row of the timeline table: either one [`Segment`] as-is, or
/// several merged together (see [`write_timeline`]).
struct Row {
    first_idx: usize,
    last_idx: usize,
    start_ms: f64,
    end_ms: f64,
    rms_dbfs: Option<f64>,
    peak_dbfs: Option<f64>,
    onset_count: u32,
    silent: bool,
    f0_hz: Option<f64>,
}

impl Row {
    /// Merge a chunk of consecutive segments into one display row:
    /// `rms`/`peak` take the max (loudest wins, so a transient inside a
    /// merged bucket isn't averaged away), `onsets` sum, `silent` requires
    /// every segment in the chunk to be silent, and `f0` is the median of
    /// whichever segments were voiced.
    fn merge(chunk: &[Segment]) -> Self {
        let first = chunk.first().expect("chunks() never yields an empty slice");
        let last = chunk.last().expect("chunks() never yields an empty slice");

        let rms_dbfs = max_of(chunk.iter().filter_map(|s| s.rms_dbfs));
        let peak_dbfs = max_of(chunk.iter().filter_map(|s| s.peak_dbfs));
        let onset_count = chunk.iter().map(|s| s.onset_count).sum();
        let silent = chunk.iter().all(|s| s.silent);
        let mut voiced: Vec<f64> = chunk.iter().filter_map(|s| s.f0_hz).collect();
        let f0_hz = crate::pitch::median(&mut voiced);

        Self {
            first_idx: first.index,
            last_idx: last.index,
            start_ms: first.start_ms,
            end_ms: last.end_ms,
            rms_dbfs,
            peak_dbfs,
            onset_count,
            silent,
            f0_hz,
        }
    }
}

/// Append the timeline table: one row per window, or — if `timeline` has
/// more than [`MAX_TIMELINE_ROWS`] segments — one row per
/// `ceil(n / MAX_TIMELINE_ROWS)`-segment bucket (see [`Row::merge`]). Runs
/// of [`MIN_SILENT_RUN`]-or-more consecutive silent rows (post-bucketing)
/// collapse into a single `idx-idx  silent (N.N s)` line.
fn write_timeline(out: &mut String, timeline: &SegmentTimeline) {
    let n = timeline.segments.len();
    let bucket_width = if n > MAX_TIMELINE_ROWS {
        n.div_ceil(MAX_TIMELINE_ROWS)
    } else {
        1
    };

    let rows: Vec<Row> = timeline
        .segments
        .chunks(bucket_width.max(1))
        .map(Row::merge)
        .collect();

    // Render the table body first so the header's `rows=` counts the lines
    // actually printed — silent-run collapsing merges 3+ rows into one, and
    // a header counting pre-collapse buckets would overstate the table an
    // agent is budgeting context for.
    let mut lines: Vec<String> = Vec::with_capacity(rows.len());
    let mut i = 0;
    while i < rows.len() {
        if rows[i].silent {
            let run_start = i;
            let mut run_end = i;
            while run_end + 1 < rows.len() && rows[run_end + 1].silent {
                run_end += 1;
            }
            if run_end - run_start + 1 >= MIN_SILENT_RUN {
                let first = &rows[run_start];
                let last = &rows[run_end];
                let duration_s = (last.end_ms - first.start_ms) / 1000.0;
                // "silent" (RMS under floor) and "onset" (spectral flux)
                // are independent detectors — a transient can land in a
                // floor-silent window, and a digest that hides it would
                // silently disagree with its own header onset count.
                let onsets: u32 = rows[run_start..=run_end]
                    .iter()
                    .map(|r| r.onset_count)
                    .sum();
                if onsets > 0 {
                    lines.push(format!(
                        "  {}-{}  silent ({duration_s:.1} s, ons={onsets})",
                        first.first_idx, last.last_idx,
                    ));
                } else {
                    lines.push(format!(
                        "  {}-{}  silent ({duration_s:.1} s)",
                        first.first_idx, last.last_idx,
                    ));
                }
                i = run_end + 1;
                continue;
            }
        }
        lines.push(row_line(&rows[i]));
        i += 1;
    }

    writeln!(
        out,
        "timeline: window={:.0}ms  bucket={bucket_width}x  rows={}",
        timeline.window_ms,
        lines.len(),
    )
    .expect("String write is infallible");
    writeln!(out, "   idx        t(s)     rms   peak  ons     f0  flags",)
        .expect("String write is infallible");
    for line in lines {
        writeln!(out, "{line}").expect("String write is infallible");
    }
}

fn row_line(row: &Row) -> String {
    let idx = if row.first_idx == row.last_idx {
        format!("{}", row.first_idx)
    } else {
        format!("{}-{}", row.first_idx, row.last_idx)
    };
    let flags = if row.silent { "S" } else { "-" };
    format!(
        "{idx:>6}  {:>6.3}-{:<6.3}  {:>6}  {:>5}  {:>3}  {:>6}  {flags}",
        row.start_ms / 1000.0,
        row.end_ms / 1000.0,
        fmt_dash(row.rms_dbfs, 2),
        fmt_dash(row.peak_dbfs, 2),
        row.onset_count,
        fmt_dash(row.f0_hz, 1),
    )
}

/// LUFS-specific formatting: `None` (ebur128's `-inf` reading, silence or
/// too little audio) prints as `"-inf"`, since that's what the measurement
/// actually is — distinct from the generic `"-"` used for other undefined
/// fields (`docs/plan.md`'s ebur128 audit).
fn fmt_lufs(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.2}"),
        None => "-inf".to_string(),
    }
}

/// Generic "undefined" formatting for non-LUFS fields: `None` prints as
/// `"-"`.
fn fmt_dash(v: Option<f64>, decimals: usize) -> String {
    match v {
        Some(x) => format!("{x:.decimals$}"),
        None => "-".to_string(),
    }
}

/// Generic "undefined" formatting with a forced sign on defined values:
/// `None` prints as `"-"`, `Some(x)` always shows `+`/`-`.
fn fmt_signed_dash(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:+.2}"),
        None => "-".to_string(),
    }
}

/// `"tempo: 120.0bpm (conf 0.11) clear_rhythm=true"`, or `"tempo: -"` when
/// no tempo was detected at all.
fn fmt_tempo_line(tempo: &crate::TempoSummary) -> String {
    match tempo.bpm {
        Some(bpm) => format!(
            "tempo: {bpm:.1}bpm (conf {:.2}) clear_rhythm={}",
            tempo.confidence, tempo.clear_rhythm
        ),
        None => "tempo: -".to_string(),
    }
}

/// `"structure: 3 sections @ 8.0s, 16.0s"`, or `"structure: 1 section"`
/// when no boundaries were found (singular/plural on the count itself, so
/// a degenerate `0 sections` reads naturally too).
fn fmt_structure_line(structure: &crate::StructureReport) -> String {
    if structure.boundaries_ms.is_empty() {
        let noun = if structure.section_count == 1 {
            "section"
        } else {
            "sections"
        };
        format!("structure: {} {noun}", structure.section_count)
    } else {
        let times: Vec<String> = structure
            .boundaries_ms
            .iter()
            .map(|ms| format!("{:.1}s", ms / 1000.0))
            .collect();
        format!(
            "structure: {} sections @ {}",
            structure.section_count,
            times.join(", ")
        )
    }
}

fn fmt_median_pitch(v: Option<f64>) -> String {
    match v {
        Some(f0) => {
            let midi = crate::pitch::nearest_midi(f0);
            let cents = crate::pitch::cents_off(f0, midi);
            format!("{f0:.1}Hz ({} {cents:+.1}c)", note_name(midi))
        }
        None => "-".to_string(),
    }
}

/// Note name + octave for a MIDI note number (`60` -> `"C4"`, MIDI's
/// standard octave numbering where middle C is C4).
fn note_name(midi: i32) -> String {
    let pc = PitchClass::ALL[midi.rem_euclid(12) as usize];
    let octave = midi.div_euclid(12) - 1;
    format!("{}{octave}", pc.name())
}
