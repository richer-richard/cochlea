//! `cochlea export` end-to-end: it writes a real Standard MIDI File, and it
//! refuses to clobber the source score — including when `--out` is a
//! *differently-spelled* path to the same file (the canonicalized-alias
//! guard, not just a raw string compare). Mirrors the other cli tests'
//! `Command::new(CARGO_BIN_EXE_cochlea)` pattern.

use std::path::PathBuf;
use std::process::Command;

const SCORE: &str = r#"Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: "lead", instrument: Preset("sine"),
        notes: [ Note(at: (1, 1), dur: "1/4", pitch: "A4", vel: 96) ]) ],
)"#;

fn case_dir(name: &str) -> PathBuf {
    // A unique dir per case so parallel test threads never share filenames.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("export_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cochlea() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cochlea"))
}

#[test]
fn export_writes_a_valid_smf() {
    let dir = case_dir("valid");
    let score = dir.join("score.ron");
    let out = dir.join("out.mid");
    std::fs::write(&score, SCORE).unwrap();

    let status = cochlea()
        .args([
            "export",
            score.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "export should succeed");

    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(
        &bytes[..4],
        b"MThd",
        "a Standard MIDI File starts with MThd"
    );
}

#[test]
fn export_refuses_to_overwrite_the_source_via_an_aliased_path() {
    let dir = case_dir("alias");
    let score = dir.join("score.ron");
    std::fs::write(&score, SCORE).unwrap();

    // `<dir>/./score.ron` is a different spelling of `<dir>/score.ron`: a raw
    // `out == score` compare misses it, but both canonicalize to the same
    // file, so the guard must still fire.
    let aliased_out = dir.join(".").join("score.ron");
    let code = cochlea()
        .args([
            "export",
            score.to_str().unwrap(),
            "--out",
            aliased_out.to_str().unwrap(),
        ])
        .status()
        .unwrap()
        .code();
    // A refusal is a non-success exit (the CLI maps this usage error to 2 via
    // `main`'s anyhow handler; assert only "not success" so the test pins the
    // guarantee, not the exit-code convention).
    assert_ne!(
        code,
        Some(0),
        "aliased --out onto the source must be refused"
    );

    // ...and the source is untouched: still the exact RON we wrote, not MIDI.
    let after = std::fs::read_to_string(&score).unwrap();
    assert_eq!(after, SCORE, "the source score must be preserved verbatim");
}
