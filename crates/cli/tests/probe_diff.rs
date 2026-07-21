//! End-to-end tests for `cochlea probe`'s `--digest`/`--segments` flags and
//! the `cochlea diff` subcommand. Mirrors `demos.rs`'s
//! `Command::new(CARGO_BIN_EXE_cochlea)` pattern. Renders are kept to two
//! total (two different demo scores, rendered once via `OnceLock` so
//! parallel `#[test]` threads share the same WAVs instead of racing to
//! re-render them) — every assertion below reuses their output.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;

fn demo_path(name: &str, file: &str) -> String {
    format!("{}/../../demos/{name}/{file}", env!("CARGO_MANIFEST_DIR"))
}

fn tmp_path(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn cochlea() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cochlea"))
}

fn render(score: &str, out: &Path) {
    let status = cochlea()
        .args(["render", score, "--out", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "render {score} failed");
}

/// Two different demo renders, `(metronome, chord_pad)`. Rendered exactly
/// once per test binary run, regardless of how many tests call this
/// concurrently — `OnceLock::get_or_init` blocks concurrent callers until
/// the first render finishes, so nothing reads a half-written WAV.
fn fixtures() -> &'static (PathBuf, PathBuf) {
    static FIXTURES: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        let metronome = tmp_path("probe_diff_metronome.wav");
        let chord_pad = tmp_path("probe_diff_chord_pad.wav");
        render(&demo_path("metronome", "score.ron"), &metronome);
        render(&demo_path("chord_pad", "score.ron"), &chord_pad);
        (metronome, chord_pad)
    })
}

#[test]
fn probe_digest_prints_a_digest_not_json() {
    let (a, _b) = fixtures();
    let output = cochlea()
        .args(["probe", a.to_str().unwrap(), "--digest"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("cochlea digest:"), "{stdout}");
    assert!(
        serde_json::from_str::<Value>(&stdout).is_err(),
        "digest stdout should not parse as JSON: {stdout}"
    );
}

#[test]
fn probe_segments_writes_valid_segment_timeline_json() {
    let (a, _b) = fixtures();
    let segments_path = tmp_path("probe_diff_segments.json");
    let output = cochlea()
        .args([
            "probe",
            a.to_str().unwrap(),
            "--segments",
            segments_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = std::fs::read_to_string(&segments_path).unwrap();
    let timeline: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(timeline["schema_version"], 1);
    assert!(
        !timeline["segments"].as_array().unwrap().is_empty(),
        "{timeline}"
    );
}

#[test]
fn probe_digest_and_json_together_write_both() {
    let (a, _b) = fixtures();
    let json_path = tmp_path("probe_diff_report.json");
    let output = cochlea()
        .args([
            "probe",
            a.to_str().unwrap(),
            "--digest",
            "--json",
            json_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Digest to stdout...
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("cochlea digest:"), "{stdout}");

    // ...full JSON report to the file, both present, no conflict.
    let report: Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    assert_eq!(report["schema_version"], 4);
}

#[test]
fn diff_self_is_byte_identical_and_tier2_exits_zero() {
    let (a, _b) = fixtures();
    let output = cochlea()
        .args(["diff", a.to_str().unwrap(), a.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("byte-identical"), "{stdout}");

    let status = cochlea()
        .args(["diff", a.to_str().unwrap(), a.to_str().unwrap(), "--tier2"])
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "diff --tier2 of a file against itself should exit 0"
    );
}

#[test]
fn diff_different_renders_reports_different_and_tier2_exits_one() {
    let (a, b) = fixtures();
    let output = cochlea()
        .args(["diff", a.to_str().unwrap(), b.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "diff without --tier2 exits 0 even when the verdict is Different"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("different"), "{stdout}");

    let status = cochlea()
        .args(["diff", a.to_str().unwrap(), b.to_str().unwrap(), "--tier2"])
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(1),
        "diff --tier2 of two different renders should exit 1"
    );
}

/// FLAC input goes through the same `probe` path as WAV
/// (`cochlea_decode::load` dispatches on extension) — uses the decode
/// crate's committed fixture, no render needed.
#[test]
fn probe_digest_reads_flac() {
    let flac = format!(
        "{}/../decode/tests/fixtures/tone_mono_16.flac",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = cochlea()
        .args(["probe", &flac, "--digest"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("cochlea digest:"), "{stdout}");
    assert!(stdout.contains("8000Hz"), "{stdout}");
}

/// Output flags pointing back at an input are a usage error (exit 2) —
/// before this guard, `probe x.wav --json x.wav` silently destroyed the
/// input audio with exit 0.
#[test]
fn output_flags_never_overwrite_an_input() {
    let (a, b) = fixtures();
    let a_str = a.to_str().unwrap();
    let b_str = b.to_str().unwrap();
    let before = std::fs::read(a).unwrap();

    let output = cochlea()
        .args(["probe", a_str, "--json", a_str])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");

    let output = cochlea()
        .args(["probe", a_str, "--segments", a_str])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");

    let output = cochlea()
        .args(["diff", a_str, b_str, "--json", b_str])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");

    assert_eq!(
        std::fs::read(a).unwrap(),
        before,
        "the input WAV must be byte-identical after the rejected calls"
    );
}

/// The zoom lens: `--from/--to` cuts before analysis, the report anchors
/// itself with `source.start_ms`, and bad windows are usage errors.
#[test]
fn probe_from_to_windows_the_analysis() {
    let (a, _b) = fixtures();
    let a_str = a.to_str().unwrap();
    let json_path = tmp_path("probe_window_report.json");

    let output = cochlea()
        .args([
            "probe",
            a_str,
            "--from",
            "1.0",
            "--to",
            "3.0",
            "--json",
            json_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let report: Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    assert_eq!(report["source"]["start_ms"], 1000.0);
    assert_eq!(report["source"]["duration_ms"], 2000.0);

    // Inverted and past-the-end windows are usage errors (exit 2).
    let output = cochlea()
        .args(["probe", a_str, "--from", "3.0", "--to", "1.0"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let output = cochlea()
        .args(["probe", a_str, "--from", "9999"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

#[test]
fn spectro_annotate_and_window_write_a_png() {
    let (a, _b) = fixtures();
    let out = tmp_path("spectro_annotated.png");
    let output = cochlea()
        .args([
            "spectro",
            a.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--annotate",
            "--from",
            "0.5",
            "--to",
            "4.5",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let img = image::open(&out).unwrap().to_rgb8();
    // 4 s at 512-hop/48kHz ≈ 375 real frames (+ padding tail) — the window
    // was honored, not the full 16+ s render.
    assert!(
        img.width() < 450,
        "windowed spectrogram too wide: {}",
        img.width()
    );
    // The metronome has strong onsets: cyan onset ticks must appear.
    let onset_px = img.pixels().filter(|p| p.0 == [64, 224, 255]).count();
    assert!(onset_px > 0, "no onset overlay pixels drawn");

    // --annotate and --sheet are mutually exclusive (clap conflict, exit 2).
    let output = cochlea()
        .args([
            "spectro",
            a.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--annotate",
            "--sheet",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

#[test]
fn diff_spectro_writes_a_signed_heat_map() {
    let (a, b) = fixtures();
    let out = tmp_path("diff_spectro.png");
    let output = cochlea()
        .args([
            "diff",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--spectro",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let img = image::open(&out).unwrap().to_rgb8();
    // Two different renders must produce both louder-in-b and quieter-in-b
    // regions somewhere.
    assert!(img.pixels().any(|p| p[0] > 32), "no red (louder) pixels");
    assert!(img.pixels().any(|p| p[2] > 32), "no blue (quieter) pixels");

    // Self-diff: the spectrogram region is all black (identical input).
    let self_out = tmp_path("diff_spectro_self.png");
    let output = cochlea()
        .args([
            "diff",
            a.to_str().unwrap(),
            a.to_str().unwrap(),
            "--spectro",
            self_out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let img = image::open(&self_out).unwrap().to_rgb8();
    let nonblack = img
        .enumerate_pixels()
        .filter(|&(_, y, p)| y < img.height() - 10 && (p[0] > 0 || p[2] > 0))
        .count();
    assert_eq!(nonblack, 0, "self-diff must be black above the ruler");
}
