//! `cochlea transcribe`: the audio→score arrow, tested where it matters —
//! against a *rendered* score whose notes are known exactly, so the
//! assertion is "the transcription recovers what was played", not "the
//! output parses".
//!
//! The round trip is score → render → transcribe → score. Pitch and
//! placement are the contract; velocity is explicitly not (it is estimated
//! from peak level, and a synth's output level is not its authored
//! velocity). Mirrors the other cli tests' `CARGO_BIN_EXE_cochlea` pattern.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A C-major scale, one quarter note per beat at 120 BPM — five unambiguous
/// pitches on the grid, the easiest possible thing for a tracker to hear.
const SCALE: &str = r#"Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: "lead", instrument: Preset("sine"), notes: [
        Note(at: (1, 1), dur: "1/4", pitch: "C4", vel: 100),
        Note(at: (1, 2), dur: "1/4", pitch: "D4", vel: 100),
        Note(at: (1, 3), dur: "1/4", pitch: "E4", vel: 100),
        Note(at: (1, 4), dur: "1/4", pitch: "F4", vel: 100),
        Note(at: (2, 1), dur: "1/4", pitch: "G4", vel: 100),
    ]) ],
)"#;

fn case_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("transcribe_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cochlea() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cochlea"))
}

/// Render `score_ron` to a WAV in `dir` and return its path.
fn render(dir: &Path, score_ron: &str) -> PathBuf {
    let score = dir.join("source.ron");
    let wav = dir.join("source.wav");
    std::fs::write(&score, score_ron).unwrap();
    let status = cochlea()
        .args([
            "render",
            score.to_str().unwrap(),
            "--out",
            wav.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "render failed");
    wav
}

/// The `(bar, beat)` position and pitch name of every note in a written
/// score, in file order — enough to assert placement and pitch without
/// re-parsing RON structurally.
fn notes_of(ron: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let (mut at, mut pitch) = (None, None);
    for line in ron.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("at: ") {
            at = Some(rest.trim_end_matches(',').to_owned());
        } else if let Some(rest) = line.strip_prefix("pitch: ") {
            pitch = Some(rest.trim_end_matches(',').trim_matches('"').to_owned());
        }
        if let (Some(a), Some(p)) = (at.as_ref(), pitch.as_ref()) {
            out.push((a.clone(), p.clone()));
            (at, pitch) = (None, None);
        }
    }
    out
}

#[test]
fn a_rendered_scale_transcribes_back_to_the_notes_that_were_played() {
    let dir = case_dir("roundtrip");
    let wav = render(&dir, SCALE);
    let out = dir.join("back.ron");

    let output = cochlea()
        .args([
            "transcribe",
            wav.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            // Pin the tempo: detection is within ~0.3% here, but this test
            // is about the transcription, not the tempo estimator.
            "--bpm",
            "120",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "transcribe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let ron = std::fs::read_to_string(&out).unwrap();
    let notes = notes_of(&ron);
    assert_eq!(
        notes,
        vec![
            ("(1, 1)".to_owned(), "C4".to_owned()),
            ("(1, 2)".to_owned(), "D4".to_owned()),
            ("(1, 3)".to_owned(), "E4".to_owned()),
            ("(1, 4)".to_owned(), "F4".to_owned()),
            ("(2, 1)".to_owned(), "G4".to_owned()),
        ],
        "the transcription should recover the played scale\n{ron}"
    );
}

#[test]
fn the_transcribed_score_renders_again() {
    let dir = case_dir("rerender");
    let wav = render(&dir, SCALE);
    let out = dir.join("back.ron");
    let remix = dir.join("remix.wav");

    assert!(
        cochlea()
            .args([
                "transcribe",
                wav.to_str().unwrap(),
                "--out",
                out.to_str().unwrap(),
                "--bpm",
                "120",
            ])
            .status()
            .unwrap()
            .success()
    );
    // The whole point of transcribing to *score* rather than to a report:
    // the result goes straight back through the renderer.
    assert!(
        cochlea()
            .args([
                "render",
                out.to_str().unwrap(),
                "--out",
                remix.to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success(),
        "the transcribed score should render"
    );
    assert!(
        remix.metadata().unwrap().len() > 1000,
        "remix should be audio"
    );
}

#[test]
fn every_guess_is_reported() {
    let dir = case_dir("warnings");
    let wav = render(&dir, SCALE);
    let out = dir.join("back.ron");

    let output = cochlea()
        .args([
            "transcribe",
            wav.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in ["tempo", "monophonic", "quantized", "re-voice"] {
        assert!(
            stderr.contains(expected),
            "stderr should mention {expected:?}, got:\n{stderr}"
        );
    }
}

#[test]
fn the_preset_and_track_name_are_honored() {
    let dir = case_dir("voicing");
    let wav = render(&dir, SCALE);
    let out = dir.join("back.ron");

    assert!(
        cochlea()
            .args([
                "transcribe",
                wav.to_str().unwrap(),
                "--out",
                out.to_str().unwrap(),
                "--preset",
                "marimba",
                "--track",
                "melody",
            ])
            .status()
            .unwrap()
            .success()
    );
    let ron = std::fs::read_to_string(&out).unwrap();
    assert!(ron.contains(r#"Preset("marimba")"#), "{ron}");
    assert!(ron.contains(r#"name: "melody""#), "{ron}");
}

#[test]
fn transcribe_refuses_to_write_over_its_own_input() {
    let dir = case_dir("guard");
    let wav = render(&dir, SCALE);
    let before = std::fs::metadata(&wav).unwrap().len();

    // An aliased spelling of the same file — the class of bug the shared
    // `same_file` helper exists to close.
    let aliased = dir.join(".").join("source.wav");
    let output = cochlea()
        .args([
            "transcribe",
            wav.to_str().unwrap(),
            "--out",
            aliased.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "the guard should fail the command"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("overwrite"),
        "the guard should say why"
    );
    assert_eq!(
        std::fs::metadata(&wav).unwrap().len(),
        before,
        "the input audio must be untouched"
    );
}

#[test]
fn an_unparseable_grid_fails_before_reading_the_audio() {
    let dir = case_dir("badgrid");
    let wav = render(&dir, SCALE);
    let out = dir.join("back.ron");

    let output = cochlea()
        .args([
            "transcribe",
            wav.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--grid",
            "banana",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!out.exists(), "nothing should be written on a bad grid");
}

#[test]
fn an_out_of_range_tempo_is_rejected_at_the_flag() {
    let dir = case_dir("badbpm");
    let wav = render(&dir, SCALE);
    let out = dir.join("back.ron");

    let output = cochlea()
        .args([
            "transcribe",
            wav.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--bpm",
            "99999",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!out.exists(), "nothing should be written on a bad tempo");
}

#[test]
fn raw_timing_skips_quantization() {
    let dir = case_dir("rawgrid");
    let wav = render(&dir, SCALE);
    let out = dir.join("back.ron");

    let output = cochlea()
        .args([
            "transcribe",
            wav.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--grid",
            "none",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("raw tick resolution"),
        "the no-grid path should say so"
    );
}

/// Everything knowable without the audio is rejected at the boundary, and
/// crucially *before* `--out` is written — an existing file must survive a
/// command that was never going to succeed.
#[test]
fn invalid_flags_fail_before_writing_anything() {
    let dir = case_dir("failfast");
    let wav = render(&dir, SCALE);
    let out = dir.join("precious.ron");
    const PRECIOUS: &str = "// hand-edited, must not be clobbered\n";

    for args in [
        vec!["--preset", "saw"],         // not in the catalog
        vec!["--ppq", "3"],              // outside the IR's PPQ range
        vec!["--ppq", "25"],             // grid can't land on a whole tick
        vec!["--grid", "banana"],        // unparseable
        vec!["--grid", "1/2147483648."], // would overflow the dotted multiplier
        vec!["--bpm", "99999"],          // outside the IR's tempo range
    ] {
        std::fs::write(&out, PRECIOUS).unwrap();
        let mut cmd = cochlea();
        cmd.args([
            "transcribe",
            wav.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]);
        cmd.args(&args);
        let output = cmd.output().unwrap();

        assert!(
            !output.status.success(),
            "{args:?} should fail: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            PRECIOUS,
            "{args:?} must not touch --out"
        );
    }
}

/// An overlapping or collapsed note is repaired rather than emitted as a
/// chord — the transcription is a monophonic line by construction.
#[test]
fn the_transcription_is_monophonic() {
    let dir = case_dir("monophonic");
    let wav = render(&dir, SCALE);
    let out = dir.join("back.ron");

    assert!(
        cochlea()
            .args([
                "transcribe",
                wav.to_str().unwrap(),
                "--out",
                out.to_str().unwrap(),
                "--bpm",
                "120",
            ])
            .status()
            .unwrap()
            .success()
    );
    let ron = std::fs::read_to_string(&out).unwrap();
    // Every `at:` must be distinct — two notes on one tick would mean the
    // repair pass let a stack through.
    let ats: Vec<String> = ron
        .lines()
        .map(str::trim)
        .filter_map(|l| {
            l.strip_prefix("at: ")
                .map(|r| r.trim_end_matches(',').to_owned())
        })
        .collect();
    let mut unique = ats.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(ats.len(), unique.len(), "notes share a tick:\n{ron}");
}
