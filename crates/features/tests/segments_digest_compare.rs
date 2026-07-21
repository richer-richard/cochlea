//! Integration tests for the segment timeline, text digest, and compare
//! APIs (`cochlea_features::{segment_timeline, digest_text, compare}`).
//! Fixtures are synthesized here with `libm`, never a synth dependency —
//! mirrors `tests/probe.rs`'s style.

use cochlea_features::{
    Analysis, ProbeOpts, SegmentOpts, Verdict, compare, compare_with_identity, digest_text, probe,
    samples_identical, segment_timeline,
};

mod common;
use common::*;

// ---------------------------------------------------------------- segments

#[test]
fn segment_timeline_classifies_tone_silence_tone() {
    let mut samples = sine_wave(440.0, 0.5, 1.0, SR);
    samples.extend(silence(1.0, SR));
    samples.extend(sine_wave(440.0, 0.5, 1.0, SR));
    let audio = mono_audio(samples, SR);

    let timeline = segment_timeline(&audio, &SegmentOpts::default());
    assert_eq!(
        timeline.segments.len(),
        3,
        "segments: {:?}",
        timeline.segments
    );

    let tone_a = &timeline.segments[0];
    assert!(!tone_a.silent, "segment 0 should be audible");
    assert!(tone_a.rms_dbfs.is_some());
    let f0 = tone_a.f0_hz.expect("segment 0 should be voiced");
    let cents = 1200.0 * libm::log2(f0 / 440.0);
    assert!(
        cents.abs() <= 5.0,
        "segment 0 f0 {f0} Hz is {cents} cents from A4"
    );
    assert_eq!(tone_a.midi_nearest, Some(69));

    let silent_seg = &timeline.segments[1];
    assert!(silent_seg.silent, "segment 1 should be silent");
    assert_eq!(
        silent_seg.rms_dbfs, None,
        "digital silence should have no rms_dbfs"
    );
    assert_eq!(silent_seg.peak_dbfs, None);
    assert_eq!(silent_seg.f0_hz, None);
    assert_eq!(silent_seg.midi_nearest, None);
    assert_eq!(silent_seg.cents_off, None);

    let tone_b = &timeline.segments[2];
    assert!(!tone_b.silent);
    assert!(tone_b.f0_hz.is_some());
}

#[test]
fn band_energy_low_tone_is_low_dominant() {
    let audio = mono_audio(sine_wave(100.0, 0.8, 1.0, SR), SR);
    let timeline = segment_timeline(&audio, &SegmentOpts::default());
    let band = timeline.segments[0].band_energy;
    assert!(
        band.low > band.mid && band.low > band.high,
        "band_energy: {band:?}"
    );
}

#[test]
fn band_energy_high_tone_is_high_dominant() {
    let audio = mono_audio(sine_wave(8000.0, 0.8, 1.0, SR), SR);
    let timeline = segment_timeline(&audio, &SegmentOpts::default());
    let band = timeline.segments[0].band_energy;
    assert!(
        band.high > band.mid && band.high > band.low,
        "band_energy: {band:?}"
    );
}

#[test]
fn segment_onset_counts_sum_to_report_total() {
    let onset_times_s: Vec<f64> = (1..=6).map(|i| f64::from(i) * 0.5).collect();
    let audio = mono_audio(click_track(&onset_times_s, 4.0, SR), SR);

    let report = probe(&audio, &ProbeOpts::default());
    let timeline = segment_timeline(&audio, &SegmentOpts::default());

    let summed: u32 = timeline.segments.iter().map(|s| s.onset_count).sum();
    assert_eq!(summed as usize, report.onsets.count);
}

#[test]
fn segment_timeline_empty_audio_is_empty() {
    let audio = mono_audio(Vec::new(), SR);
    let timeline = segment_timeline(&audio, &SegmentOpts::default());
    assert!(timeline.segments.is_empty());
}

#[test]
fn segment_timeline_sub_window_file_is_one_partial_segment() {
    let audio = mono_audio(sine_wave(440.0, 0.5, 0.25, SR), SR); // 250 ms < 1000 ms window
    let timeline = segment_timeline(&audio, &SegmentOpts::default());
    assert_eq!(
        timeline.segments.len(),
        1,
        "segments: {:?}",
        timeline.segments
    );
    let seg = &timeline.segments[0];
    assert!((seg.end_ms - 250.0).abs() < 1.0, "end_ms = {}", seg.end_ms);
}

// ------------------------------------------------------------------ digest

#[test]
fn digest_text_snapshot_with_silent_run_collapsing() {
    let mut samples = sine_wave(440.0, 0.5, 2.0, SR);
    samples.extend(silence(4.0, SR));
    samples.extend(sine_wave(440.0, 0.5, 2.0, SR));
    let audio = mono_audio(samples, SR);

    let report = probe(&audio, &ProbeOpts::default());
    let timeline = segment_timeline(&audio, &SegmentOpts::default());
    let digest = digest_text(&report, &timeline);

    // Locked snapshot: proves byte-determinism and pins the format. The
    // silent run at segments 2-5 (the 4 s of digital silence) collapses
    // into one row, per `MIN_SILENT_RUN`.
    let expected = [
        "cochlea digest: 8.000s  1ch  48000Hz",
        "loudness: integrated=-10.05  momentary_max=-9.71  true_peak=-6.01  lra=3.01",
        "key: A minor (conf 0.68)  pitch: voiced=50%  median=440.0Hz (A4 +0.1c)",
        // Two isolated events six seconds apart have no tempo — the v3
        // detector says so instead of the old "126.4bpm (conf 0.00)".
        "tempo: -",
        "rhythm: clear=false  grid_align=-  offbeat=-",
        "structure: 1 section",
        "onsets: count=2  rate=0.25/s",
        "silence: leading=0ms  trailing=0ms",
        "clipping: clipped=0  over_0dbtp=false",
        // rows=5 counts the table lines actually printed (post-collapse),
        // not the 8 pre-collapse buckets — the header is what an agent
        // budgets context against.
        "timeline: window=1000ms  bucket=1x  rows=5",
        "   idx        t(s)     rms   peak  ons     f0  flags",
        "     0   0.000-1.000    -9.03  -6.02    0   440.0  -",
        "     1   1.000-2.000    -9.03  -6.02    0   440.0  -",
        // ons=1: the tone that resumes at 6 s produces a spectral-flux
        // onset whose frame-center lands just inside floor-silent segment
        // 5 — the digest must surface it, not hide it in the collapsed
        // run (its absence would contradict the header's count=2).
        "  2-5  silent (4.0 s, ons=1)",
        "     6   6.000-7.000    -9.03  -6.02    1   440.0  -",
        "     7   7.000-8.000    -9.03  -6.02    0   440.0  -",
    ]
    .join("\n")
        + "\n";

    assert_eq!(digest, expected, "actual digest:\n{digest}");
}

#[test]
fn digest_text_caps_timeline_rows_for_long_timelines() {
    // 5 s at 100 ms windows = 50 segments, over MAX_TIMELINE_ROWS (40) — the
    // silent-run-collapsing test never exercises this path (only 8 rows).
    let audio = mono_audio(sine_wave(440.0, 0.5, 5.0, SR), SR);
    let opts = SegmentOpts::default().with_window_ms(100.0);
    let timeline = segment_timeline(&audio, &opts);
    assert_eq!(
        timeline.segments.len(),
        50,
        "segments: {}",
        timeline.segments.len()
    );

    let report = probe(&audio, &ProbeOpts::default());
    let digest = digest_text(&report, &timeline);

    let header_line = digest
        .lines()
        .find(|l| l.starts_with("timeline:"))
        .expect("digest should have a timeline header");
    assert!(
        header_line.contains("bucket=2x"),
        "expected ceil(50/40) = 2-wide buckets: {header_line}"
    );
    assert!(
        header_line.contains("rows=25"),
        "expected 50/2 = 25 rows: {header_line}"
    );
}

// ----------------------------------------------------------------- compare

#[test]
fn compare_identical_signal_is_tier2_equivalent_with_zero_deltas() {
    let audio = mono_audio(sine_wave(440.0, 0.5, 2.0, SR), SR);
    let report = probe(&audio, &ProbeOpts::default());
    let timeline = segment_timeline(&audio, &SegmentOpts::default());

    let a = Analysis {
        report: &report,
        timeline: &timeline,
    };
    let b = Analysis {
        report: &report,
        timeline: &timeline,
    };
    let result = compare(a, b);

    assert_eq!(
        result.verdict,
        Verdict::Tier2Equivalent,
        "{:?}",
        result.verdict
    );
    assert_eq!(result.duration_delta_ms, 0.0);
    assert_eq!(result.loudness.integrated_lufs_delta, Some(0.0));
    assert_eq!(result.pitch.map(|p| p.cents), Some(0.0));
    assert!(!result.key.changed);
    assert_eq!(result.onsets.unmatched_a, 0);
    assert_eq!(result.onsets.unmatched_b, 0);
}

#[test]
fn compare_byte_identical_flag_wins_over_computed_verdict() {
    let audio = mono_audio(sine_wave(440.0, 0.5, 1.0, SR), SR);
    let report = probe(&audio, &ProbeOpts::default());
    let timeline = segment_timeline(&audio, &SegmentOpts::default());
    let a = Analysis {
        report: &report,
        timeline: &timeline,
    };
    let b = Analysis {
        report: &report,
        timeline: &timeline,
    };

    assert!(samples_identical(&audio, &audio));
    let result = compare_with_identity(a, b, true);
    assert_eq!(result.verdict, Verdict::ByteIdentical);
}

#[test]
fn compare_level_shift_flags_loudness() {
    let a_audio = mono_audio(sine_wave(440.0, 0.3, 2.0, SR), SR);
    let boosted_amp = 0.3 * libm::pow(10.0, 1.0 / 20.0); // +1 dB
    let b_audio = mono_audio(sine_wave(440.0, boosted_amp, 2.0, SR), SR);

    let a_report = probe(&a_audio, &ProbeOpts::default());
    let b_report = probe(&b_audio, &ProbeOpts::default());
    let a_timeline = segment_timeline(&a_audio, &SegmentOpts::default());
    let b_timeline = segment_timeline(&b_audio, &SegmentOpts::default());

    let result = compare(
        Analysis {
            report: &a_report,
            timeline: &a_timeline,
        },
        Analysis {
            report: &b_report,
            timeline: &b_timeline,
        },
    );

    match &result.verdict {
        Verdict::Different { dimensions } => {
            assert!(
                dimensions.contains(&"loudness".to_string()),
                "{dimensions:?}"
            );
        }
        other => panic!("expected Different, got {other:?}"),
    }
}

#[test]
fn compare_detune_flags_pitch() {
    let a_audio = mono_audio(sine_wave(440.0, 0.5, 2.0, SR), SR);
    let detuned_hz = 440.0 * libm::exp2(20.0 / 1200.0); // +20 cents
    let b_audio = mono_audio(sine_wave(detuned_hz, 0.5, 2.0, SR), SR);

    let a_report = probe(&a_audio, &ProbeOpts::default());
    let b_report = probe(&b_audio, &ProbeOpts::default());
    let a_timeline = segment_timeline(&a_audio, &SegmentOpts::default());
    let b_timeline = segment_timeline(&b_audio, &SegmentOpts::default());

    let result = compare(
        Analysis {
            report: &a_report,
            timeline: &a_timeline,
        },
        Analysis {
            report: &b_report,
            timeline: &b_timeline,
        },
    );

    match &result.verdict {
        Verdict::Different { dimensions } => {
            assert!(dimensions.contains(&"pitch".to_string()), "{dimensions:?}");
        }
        other => panic!("expected Different, got {other:?}"),
    }
    let cents = result.pitch.expect("both sides voiced").cents;
    assert!((cents - 20.0).abs() <= 1.0, "cents = {cents}");
}

#[test]
fn compare_added_onset_flags_onsets() {
    let a_times: Vec<f64> = (1..=4).map(|i| f64::from(i) * 0.5).collect();
    let mut b_times = a_times.clone();
    b_times.push(3.0);

    let a_audio = mono_audio(click_track(&a_times, 3.5, SR), SR);
    let b_audio = mono_audio(click_track(&b_times, 3.5, SR), SR);

    let a_report = probe(&a_audio, &ProbeOpts::default());
    let b_report = probe(&b_audio, &ProbeOpts::default());
    let a_timeline = segment_timeline(&a_audio, &SegmentOpts::default());
    let b_timeline = segment_timeline(&b_audio, &SegmentOpts::default());

    let result = compare(
        Analysis {
            report: &a_report,
            timeline: &a_timeline,
        },
        Analysis {
            report: &b_report,
            timeline: &b_timeline,
        },
    );

    match &result.verdict {
        Verdict::Different { dimensions } => {
            assert!(dimensions.contains(&"onsets".to_string()), "{dimensions:?}");
        }
        other => panic!("expected Different, got {other:?}"),
    }
    assert_eq!(result.onsets.unmatched_b, 1, "{:?}", result.onsets);
}

/// NaN in particular must not survive the `window_ms` guard: every IEEE 754
/// comparison with NaN is false, so a plain `<= 0.0` check passes it, after
/// which it saturates into a 1-sample window and the timeline explodes to
/// one segment per sample (reproduced at 337k segments / 132 MB of JSON for
/// a 7 s file before the guard was fixed). Tiny-but-positive values reach
/// the same one-sample window through rounding, hence the 1 ms floor.
#[test]
fn segment_timeline_rejects_degenerate_window_ms() {
    let audio = mono_audio(sine_wave(440.0, 0.5, 1.0, SR), SR);
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.0,
        -1000.0,
        0.001,
        0.999,
    ] {
        let timeline = segment_timeline(&audio, &SegmentOpts::default().with_window_ms(bad));
        assert!(
            timeline.segments.is_empty(),
            "window_ms={bad} should yield an empty timeline, got {} segments",
            timeline.segments.len()
        );
    }
}

/// A NaN silence floor would make every `<=` comparison false and silently
/// disable silence detection; the timeline must fall back to the default
/// and serialize the floor it actually used.
#[test]
fn non_finite_silence_floor_falls_back_to_the_default() {
    let mut samples = sine_wave(440.0, 0.5, 1.0, SR);
    samples.extend(silence(1.0, SR));
    let audio = mono_audio(samples, SR);

    let timeline = segment_timeline(
        &audio,
        &SegmentOpts::default().with_silence_floor_dbfs(f64::NAN),
    );
    assert_eq!(timeline.silence_floor_dbfs, -60.0);
    assert!(
        timeline.segments[1].silent,
        "digital silence must still classify as silent under the fallback floor"
    );

    // And the floor actually used is always serialized on the timeline.
    let custom = segment_timeline(
        &audio,
        &SegmentOpts::default().with_silence_floor_dbfs(-40.0),
    );
    assert_eq!(custom.silence_floor_dbfs, -40.0);
}
