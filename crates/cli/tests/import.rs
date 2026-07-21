//! `cochlea import` end to end: a real SMF byte stream in, a RON score
//! out, and — the point of the pipeline — that score renders and probes.

use std::path::PathBuf;
use std::process::Command;

fn tmp_path(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn cochlea() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cochlea"))
}

/// A minimal format-1 file: 120 BPM, one melodic track (two quarters),
/// one channel-10 kick pattern. Chunk/VLQ encoding written out longhand.
fn demo_midi() -> Vec<u8> {
    let chunk = |id: &[u8; 4], data: &[u8]| {
        let mut out = id.to_vec();
        out.extend((data.len() as u32).to_be_bytes());
        out.extend(data);
        out
    };
    let mut bytes = chunk(b"MThd", &[0, 1, 0, 3, 0x01, 0xE0]); // format 1, 3 trks, 480 PPQ
    bytes.extend(chunk(
        b"MTrk",
        &[
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // tempo 500000 us = 120 BPM
            0x00, 0xFF, 0x2F, 0x00,
        ],
    ));
    bytes.extend(chunk(
        b"MTrk",
        &[
            0x00, 0x90, 60, 100, // C4 on
            0x83, 0x60, 0x80, 60, 64, // delta 480: C4 off
            0x00, 0x90, 64, 100, // E4 on
            0x83, 0x60, 0x80, 64, 64, // delta 480: E4 off
            0x00, 0xFF, 0x2F, 0x00,
        ],
    ));
    bytes.extend(chunk(
        b"MTrk",
        &[
            0x00, 0x99, 36, 110, // kick (ch 10)
            0x60, 0x89, 36, 64, // delta 96: off
            0x83, 0x00, 0x99, 36, 110, // delta 384: next kick
            0x60, 0x89, 36, 64, //
            0x00, 0xFF, 0x2F, 0x00,
        ],
    ));
    bytes
}

#[test]
fn import_produces_a_score_that_lints_renders_and_probes() {
    let mid = tmp_path("import_demo.mid");
    std::fs::write(&mid, demo_midi()).unwrap();
    let ron = tmp_path("import_demo.ron");
    let output = cochlea()
        .args([
            "import",
            mid.to_str().unwrap(),
            "--out",
            ron.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("imported 2 tracks"), "{stderr}");
    assert!(
        stderr.contains("kick"),
        "percussion mapping noted: {stderr}"
    );

    // The written RON is a valid score with the expected shape.
    let text = std::fs::read_to_string(&ron).unwrap();
    assert!(text.contains("Preset(\"kick\")"), "{text}");

    // And it renders + verifies end to end through the normal pipeline.
    let wav = tmp_path("import_demo.wav");
    let status = cochlea()
        .args([
            "render",
            ron.to_str().unwrap(),
            "--out",
            wav.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "imported score must render");
    let output = cochlea()
        .args(["probe", wav.to_str().unwrap(), "--digest"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let digest = String::from_utf8_lossy(&output.stdout);
    assert!(digest.starts_with("cochlea digest:"), "{digest}");
}

#[test]
fn import_rejects_non_midi_and_self_overwrite() {
    let not_midi = tmp_path("not_midi.mid");
    std::fs::write(&not_midi, b"definitely not midi").unwrap();
    let output = cochlea()
        .args([
            "import",
            not_midi.to_str().unwrap(),
            "--out",
            tmp_path("x.ron").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");

    let mid = tmp_path("self_overwrite.mid");
    std::fs::write(&mid, demo_midi()).unwrap();
    let output = cochlea()
        .args([
            "import",
            mid.to_str().unwrap(),
            "--out",
            mid.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}
