//! The three demos, each stressing one claim (docs/plan.md P5):
//! `metronome` — the timing spine; `chord_pad` — harmony reads correctly;
//! `title_cue` — a mixed sting asserting loudness, sweep monotonicity, and
//! silence. Plus Tier 3 spectrogram sentinels and a CLI end-to-end pass.
//!
//! Sentinels: run with `COCHLEA_BLESS=1` to (re)write
//! `demos/<name>/expected/spectro.png`; without it, images must match the
//! committed sentinel within tolerance.

use cochlea_features::{Audio, Mode, PitchClass, ProbeOpts, probe};
use cochlea_render::{Rendered, render};
use cochlea_score::*;
use cochlea_verify::VerifyExt;

fn demo_path(name: &str, file: &str) -> String {
    format!("{}/../../demos/{name}/{file}", env!("CARGO_MANIFEST_DIR"))
}

fn load_demo(name: &str) -> Score {
    let text = std::fs::read_to_string(demo_path(name, "score.ron")).unwrap();
    Score::from_ron(&text).unwrap()
}

fn audio_of(rendered: &Rendered) -> Audio {
    Audio {
        samples: rendered.mix().to_vec(),
        channels: 2,
        sample_rate: rendered.sample_rate().0,
    }
}

/// Renders a demo, runs its embedded verify block, and checks the Tier 3
/// spectrogram sentinel.
fn run_demo(name: &str) -> Rendered {
    let score = load_demo(name);
    let errors: Vec<_> = score
        .validate(&cochlea_synth::PatchBank::presets())
        .into_iter()
        .filter(|f| f.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{name} lint errors: {errors:?}");

    let rendered = render(&score).unwrap();
    let report = rendered
        .verify(&score)
        .with_specs(score.verify_specs())
        .run();
    assert!(
        report.passed,
        "{name} verify failed:\n{}",
        serde_json::to_string_pretty(&report).unwrap()
    );
    check_sentinel(name, &rendered);
    rendered
}

fn check_sentinel(name: &str, rendered: &Rendered) {
    let spec = cochlea_spectro::mel_spectrogram(
        rendered.mix(),
        2,
        rendered.sample_rate().0,
        &cochlea_spectro::SpectroOpts::new(),
    );
    let img = cochlea_spectro::render_png(&spec, &[]);
    let path = demo_path(name, "expected/spectro.png");
    if std::env::var_os("COCHLEA_BLESS").is_some() {
        std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap()).unwrap();
        cochlea_spectro::write_png(&img, &path).unwrap();
        return;
    }
    let expected = image::open(&path)
        .unwrap_or_else(|e| panic!("missing sentinel {path} (bless with COCHLEA_BLESS=1): {e}"))
        .to_rgb8();
    // Tier 3: per-channel tolerance 8/255, at most 2% of pixels differing.
    let diff = cochlea_spectro::diff_images(&img, &expected, 8, 0.02);
    assert!(
        diff.passed,
        "{name} spectrogram deviates from sentinel: {diff:?}"
    );
}

#[test]
fn metronome_schedule_is_sample_exact() {
    // The spine claim: at 120 BPM / 48 kHz every beat is exactly 24_000
    // samples; the schedule (tempo map) must place every click on it.
    let score = load_demo("metronome");
    let map = score.tempo_map();
    for barn in 1..=8u32 {
        for beat in 1..=4u32 {
            let tick = score.resolve(bar(barn).beat(beat)).unwrap();
            let expected = (u64::from(barn - 1) * 4 + u64::from(beat - 1)) * 24_000;
            assert_eq!(map.sample_at(tick), expected, "bar {barn} beat {beat}");
        }
    }
}

#[test]
fn metronome_onsets_land_on_the_grid() {
    let rendered = run_demo("metronome");
    let report = probe(&audio_of(&rendered), &ProbeOpts::default());
    assert_eq!(report.onsets.count, 32, "one onset per click");
    for (i, &t) in report.onsets.times_ms.iter().enumerate() {
        let scheduled = i as f64 * 500.0;
        // The t=0 click has no preceding STFT frame, so its earliest
        // reportable frame-center is fft/2 + hop ≈ 16 ms — a detector
        // boundary effect, not a scheduling error (the schedule test above
        // proves sample-exactness). Interior clicks hold ±10 ms.
        let tol = if i == 0 { 20.0 } else { 10.0 };
        assert!(
            (t - scheduled).abs() <= tol,
            "click {i}: detected {t} ms vs scheduled {scheduled} ms"
        );
    }
}

#[test]
fn chord_pad_harmony_reads_as_written() {
    let rendered = run_demo("chord_pad");
    let report = probe(&audio_of(&rendered), &ProbeOpts::default());
    assert_eq!(report.key.tonic, PitchClass::C, "{:?}", report.key);
    assert_eq!(report.key.mode, Mode::Major, "{:?}", report.key);
}

#[test]
fn title_cue_hits_its_targets() {
    let rendered = run_demo("title_cue");
    // The RON verify block covers LUFS / true peak / monotone sweep /
    // no-discontinuity / silence; add the DSL-side pitch check on the lead.
    let score = load_demo("title_cue");
    let report = rendered
        .verify(&score)
        .pitch_matches_score("lead", cochlea_verify::Cents(10.0))
        .run();
    assert!(
        report.passed,
        "{}",
        serde_json::to_string_pretty(&report).unwrap()
    );
    // Ten seconds of content: 4 bars at 96 BPM = exactly 10 s before tails.
    let map = score.tempo_map();
    assert_eq!(map.sample_at(score.resolve(bar(5)).unwrap()), 480_000);
}

#[test]
fn the_cochlea_binary_renders_and_verifies_end_to_end() {
    let dir = std::env::temp_dir().join("cochlea-demo-e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("metronome.wav");
    let report = dir.join("report.json");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_cochlea"))
        .args([
            "render",
            &demo_path("metronome", "score.ron"),
            "--out",
            out.to_str().unwrap(),
            "--stems",
            dir.join("stems").to_str().unwrap(),
            "--verify",
            "--report",
            report.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "render --verify should pass");
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(report["passed"], serde_json::Value::Bool(true));
    assert!(dir.join("stems/click.wav").exists());

    // probe the WAV the binary just wrote — the adoption wedge, end to end.
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_cochlea"))
        .args(["probe", out.to_str().unwrap()])
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn lint_fails_on_a_broken_score() {
    let dir = std::env::temp_dir().join("cochlea-demo-e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.ron");
    std::fs::write(
        &bad,
        r#"Score(version: 1, sample_rate: 48000, ppq: 960,
            tempo: [(tick: 0, bpm: 120.0)],
            tracks: [Track(name: "t", instrument: Preset("no_such_preset"),
                           notes: [Note(at: (1,1), dur: "1/4", pitch: "A4", vel: 90)])])"#,
    )
    .unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_cochlea"))
        .args(["lint", bad.to_str().unwrap()])
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(1), "lint exits 1 on errors");
}
