//! The input-overwrite guards on the read subcommands. `export` has its own
//! aliased-path test (`export.rs`); this covers `probe`, whose `--json` write
//! used to be able to destroy the very file it was probing when the output
//! was spelled as a different path to the same file — it loaded the audio,
//! ran the probe, then clobbered the input with JSON and exited 0. The guard
//! now canonicalizes (via the shared `same_file` helper), so the aliased
//! spelling is caught. Mirrors the other cli tests' `CARGO_BIN_EXE_cochlea`
//! pattern.

use std::path::PathBuf;
use std::process::Command;

const SCORE: &str = r#"Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: "lead", instrument: Preset("sine"),
        notes: [ Note(at: (1, 1), dur: "1/4", pitch: "A4", vel: 96) ]) ],
)"#;

fn case_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("guards_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cochlea() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cochlea"))
}

/// Render a one-note score to a real WAV in `dir` and return its path.
fn render_wav(dir: &std::path::Path) -> PathBuf {
    let score = dir.join("score.ron");
    let wav = dir.join("in.wav");
    std::fs::write(&score, SCORE).unwrap();
    let status = cochlea()
        .args([
            "render",
            score.to_str().unwrap(),
            "--out",
            wav.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "setup render should succeed");
    wav
}

#[test]
fn render_refuses_to_write_the_report_over_the_mix() {
    // `render --out mix.wav --report mix.wav` writes the WAV, then the verify
    // report is written afterward and would silently clobber it (exit 0). The
    // pairwise output guard must refuse this, like probe/diff already do.
    let dir = case_dir("render_collision");
    let score = dir.join("score.ron");
    let out = dir.join("mix.wav");
    std::fs::write(&score, SCORE).unwrap();

    let code = cochlea()
        .args([
            "render",
            score.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--verify",
            "--report",
            out.to_str().unwrap(),
        ])
        .status()
        .unwrap()
        .code();
    assert_ne!(code, Some(0), "--report onto --out must be refused");
}

#[test]
fn probe_refuses_to_overwrite_its_input_via_an_aliased_path() {
    let dir = case_dir("probe_alias");
    let wav = render_wav(&dir);
    let before = std::fs::read(&wav).unwrap();

    // `<dir>/./in.wav` is a different spelling of `<dir>/in.wav`: a raw path
    // compare misses it, but both canonicalize to the same file, so the
    // `--json` write onto the probed audio must be refused.
    let aliased = dir.join(".").join("in.wav");
    let code = cochlea()
        .args([
            "probe",
            wav.to_str().unwrap(),
            "--json",
            aliased.to_str().unwrap(),
        ])
        .status()
        .unwrap()
        .code();
    assert_ne!(
        code,
        Some(0),
        "an aliased --json onto the probed input must be refused"
    );

    // ...and the input audio is byte-for-byte intact, not a JSON report.
    let after = std::fs::read(&wav).unwrap();
    assert_eq!(after, before, "the probed input file must be preserved");
    assert_eq!(&after[..4], b"RIFF", "the input is still a WAV, not JSON");
}

#[test]
fn probe_still_writes_json_to_a_distinct_path() {
    // The guard must not over-fire: a genuinely different output path works.
    let dir = case_dir("probe_ok");
    let wav = render_wav(&dir);
    let json = dir.join("report.json");

    let status = cochlea()
        .args([
            "probe",
            wav.to_str().unwrap(),
            "--json",
            json.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "probe to a distinct --json path should work"
    );
    let text = std::fs::read_to_string(&json).unwrap();
    assert!(
        text.contains("\"schema_version\""),
        "the report JSON should have been written"
    );
}
