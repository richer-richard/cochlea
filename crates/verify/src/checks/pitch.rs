//! `pitch_matches_score`: per-note YIN pitch check against the scored MIDI
//! pitch.
//!
//! **Monophonic tracks only.** This check YINs whatever's in each note's
//! time window on the stem; if another note (on the same track — the
//! stem's own polyphony, from a poly-voiced patch) is sounding at the same
//! time, the window's energy is a mixture and a single-f0 YIN estimate is
//! meaningless. This check doesn't detect or warn about that; it's on the
//! caller to only use it on tracks that are actually monophonic.

use cochlea_features::{ProbeOpts, probe};
use cochlea_render::Rendered;
use cochlea_score::{Note, Score};

use crate::checks::util::stereo_audio;
use crate::report::CheckResult;

/// Notes shorter than this (score duration, note-on to note-off) are
/// skipped: once the 15 ms attack guard is subtracted there isn't enough
/// signal left for a stable YIN estimate (YIN's analysis window is 2048
/// samples, ~42.7 ms at 48 kHz — see `cochlea_features`' `pitch` module).
const MIN_NOTE_MS: f64 = 60.0;
/// Skipped off the front of each note's window, to dodge the attack
/// transient (onset click, filter/amp envelope attack) before YIN-ing.
const ATTACK_GUARD_MS: f64 = 15.0;

/// One note's pitch-check outcome. `cents` is `None` when YIN found no
/// pitch at all in the note's window (treated as a failing note: silence
/// or noise where a pitched tone was scored is itself a mismatch).
struct NoteEval {
    note: Note,
    cents: Option<f64>,
}

/// Every note on `track` YINs (per-note window, `cochlea_features::probe`)
/// within `tol_cents` of its scored pitch.
pub(crate) fn pitch_matches_score(
    rendered: &Rendered,
    score: &Score,
    track: &str,
    tol_cents: f64,
) -> CheckResult {
    let kind = "pitch_matches_score";
    let assertion =
        format!("every note on '{track}' YINs within {tol_cents} cents of its scored pitch");
    let expected = format!("<= {tol_cents:.1} cents per note");

    let Some(stem) = rendered.stem(track) else {
        return CheckResult::unknown_track(kind, assertion, track);
    };
    let Some(score_track) = score.tracks().iter().find(|t| t.name == track) else {
        return CheckResult::unknown_track(kind, assertion, track);
    };

    let sample_rate = rendered.sample_rate().0;
    let tempo_map = score.tempo_map();
    let frames = (stem.len() / 2) as u64;

    let mut evals: Vec<NoteEval> = Vec::new();
    let mut skips: Vec<String> = Vec::new();

    for &note in &score_track.notes {
        let onset_ms = tempo_map.ms_at(note.at);
        let end_ms = tempo_map.ms_at(note.end());
        let dur_ms = end_ms - onset_ms;
        if dur_ms < MIN_NOTE_MS {
            skips.push(format!(
                "note at tick {} ({}) skipped: {dur_ms:.1} ms < {MIN_NOTE_MS} ms",
                note.at, note.pitch
            ));
            continue;
        }

        let start_ms = onset_ms + ATTACK_GUARD_MS;
        let start_sample =
            ((start_ms / 1000.0 * f64::from(sample_rate)).round() as u64).min(frames);
        let end_sample = ((end_ms / 1000.0 * f64::from(sample_rate)).round() as u64).min(frames);
        if start_sample >= end_sample {
            skips.push(format!(
                "note at tick {} ({}) skipped: empty window after clamping to render length",
                note.at, note.pitch
            ));
            continue;
        }

        let window = &stem[(start_sample * 2) as usize..(end_sample * 2) as usize];
        let audio = stereo_audio(window, sample_rate);
        let report = probe(&audio, &ProbeOpts::default());

        // The reported median f0 across the window, falling back to the
        // longest voiced segment's f0 if the window had no globally-median
        // hop (i.e. no voiced hops at all — `median_f0_hz` is `None` iff
        // `segments` is empty, so this fallback only ever fires alongside
        // "no pitch found").
        let f0 = report.pitch.median_f0_hz.or_else(|| {
            report
                .pitch
                .segments
                .iter()
                .max_by(|a, b| (a.end_ms - a.start_ms).total_cmp(&(b.end_ms - b.start_ms)))
                .map(|seg| seg.f0_hz)
        });

        let cents = f0.map(|hz| 1200.0 * libm::log2(hz / note.pitch.hz()));
        evals.push(NoteEval { note, cents });
    }

    if evals.is_empty() {
        let detail = if skips.is_empty() {
            format!("'{track}' has no notes to check pitch against")
        } else {
            skips.join("; ")
        };
        return CheckResult {
            kind,
            assertion,
            passed: false,
            expected,
            actual: "no notes checked".to_string(),
            detail: Some(detail),
        };
    }

    let worst = evals
        .iter()
        .max_by(|a, b| {
            let ka = a.cents.map_or(f64::INFINITY, f64::abs);
            let kb = b.cents.map_or(f64::INFINITY, f64::abs);
            ka.total_cmp(&kb)
        })
        .expect("evals is non-empty, checked above");

    let passed = evals
        .iter()
        .all(|e| e.cents.is_some_and(|c| c.abs() <= tol_cents));

    let actual = match worst.cents {
        Some(c) => format!(
            "worst {c:+.1} cents (note at tick {}, {})",
            worst.note.at, worst.note.pitch
        ),
        None => format!(
            "no pitch detected (note at tick {}, {})",
            worst.note.at, worst.note.pitch
        ),
    };

    CheckResult {
        kind,
        assertion,
        passed,
        expected,
        actual,
        detail: (!skips.is_empty()).then(|| skips.join("; ")),
    }
}
