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
fn render_refuses_a_stem_that_would_overwrite_the_mix() {
    // --out lands inside --stems with the mix filename matching a track's stem
    // (the score's one track is "lead"): the per-track stem write would clobber
    // the mixdown. The guard must refuse before rendering.
    let dir = case_dir("stem_collision");
    let score = dir.join("score.ron");
    std::fs::write(&score, SCORE).unwrap();
    let stems = dir.join("stems");
    std::fs::create_dir_all(&stems).unwrap();
    let out = stems.join("lead.wav"); // == the "lead" track's stem path

    let code = cochlea()
        .args([
            "render",
            score.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--stems",
            stems.to_str().unwrap(),
        ])
        .status()
        .unwrap()
        .code();
    assert_ne!(code, Some(0), "a stem overwriting the mix must be refused");
    assert!(
        !out.exists(),
        "the guard should fire before rendering, writing nothing"
    );
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

/// A track name is score data, and `--stems` turns it into a file path.
/// Before the [`cochlea_render::stem_file_name`] rule, a name spelled as a
/// path escaped `--stems` entirely: an absolute one because `Path::join`
/// discards the base, a `../` one by climbing. Reproduced against 0.6.0 —
/// this renders a score whose track names point at a file outside the stems
/// directory and pins that the render refuses before writing anything.
#[test]
fn render_refuses_path_shaped_track_names_in_stems() {
    for (case, spelling) in [("absolute", None), ("relative", Some("../../escaped"))] {
        let dir = case_dir(&format!("stems_escape_{case}"));
        let victim = dir.join("victim.wav");
        std::fs::write(&victim, b"UNTOUCHED").unwrap();

        // Absolute: name the victim itself (minus the extension the writer
        // appends). Relative: climb out of the stems directory.
        let owned;
        let track = match spelling {
            Some(rel) => rel,
            None => {
                owned = victim.with_extension("").to_str().unwrap().to_owned();
                &owned
            }
        };
        let score = dir.join("evil.ron");
        std::fs::write(
            &score,
            format!(
                r#"Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: {track:?}, instrument: Preset("sine"),
        notes: [ Note(at: (1, 1), dur: "1/4", pitch: "A4", vel: 96) ]) ],
)"#
            ),
        )
        .unwrap();

        let out = dir.join("mix.wav");
        let stems = dir.join("stems");
        let output = cochlea()
            .args([
                "render",
                score.to_str().unwrap(),
                "--out",
                out.to_str().unwrap(),
                "--stems",
                stems.to_str().unwrap(),
            ])
            .output()
            .unwrap();

        assert!(
            !output.status.success(),
            "{case}: a path-shaped track name must fail the render"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("portable file name"),
            "{case}: the reason should name the rule: {stderr}"
        );
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"UNTOUCHED",
            "{case}: the file the track name pointed at must be untouched"
        );
        // Assert against each case's *own* escape target. `victim` above is
        // only the absolute case's target, so checking it alone would let the
        // relative case pass while happily writing its file.
        if spelling.is_some() {
            let escaped = dir.join("stems").join("../../escaped.wav");
            assert!(
                !escaped.exists(),
                "{case}: the relative escape target {} must not exist",
                escaped.display()
            );
        }
        // Validate-before-write: not even the mix, which is written first.
        assert!(!out.exists(), "{case}: no mix should have been written");
        assert!(
            !stems.exists(),
            "{case}: no stems directory should have been created"
        );
    }
}

/// `same_file` used to `canonicalize` both sides, which fails for a path that
/// does not exist yet — so for two *outputs*, the pair it most needed to
/// catch, it silently degraded to a raw string compare. Reproduced: the mix
/// was written and then destroyed by a stem, at exit 0.
#[test]
fn render_catches_a_stem_that_aliases_the_mix_through_a_dotdot_spelling() {
    let dir = case_dir("stem_alias_dotdot");
    let score = dir.join("score.ron");
    std::fs::write(&score, SCORE).unwrap();
    let stems = dir.join("stems");
    std::fs::create_dir_all(&stems).unwrap();

    let output = cochlea()
        .args([
            "render",
            score.to_str().unwrap(),
            "--out",
            stems.join("lead.wav").to_str().unwrap(),
            // The same directory, spelled so a lexical compare misses it.
            "--stems",
            stems.join("../stems").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "a stem aliasing the mix through `..` must be refused"
    );
    assert!(
        !stems.join("lead.wav").exists(),
        "the guard should fire before anything is written"
    );
}

/// macOS and Windows are case-insensitive by default, so `d/Lead.wav` and
/// `d/lead.wav` are two paths and one file there — and the guard compared
/// them exactly. Reproduced on macOS against 0.7.0: the mix was written to
/// `--out d/Lead.wav`, then the `lead` stem landed on the same file and
/// destroyed it, at exit 0.
///
/// Refused on every platform, deliberately: a score is portable data, and a
/// pair of outputs that collide on a colleague's laptop should not be
/// accepted here just because this host would keep them apart.
#[test]
fn render_refuses_a_stem_that_differs_from_the_mix_only_by_case() {
    let dir = case_dir("stem_case_collision");
    let score = dir.join("score.ron");
    std::fs::write(&score, SCORE).unwrap();
    let stems = dir.join("stems");
    std::fs::create_dir_all(&stems).unwrap();
    let out = stems.join("Lead.wav"); // the "lead" track's stem, capitalized

    let output = cochlea()
        .args([
            "render",
            score.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--stems",
            stems.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "a stem colliding with the mix by case must be refused: {stderr}"
    );
    // Named, not just non-zero: without this the test passes for any failure
    // at all — an argument-parsing change, a missing directory — while the
    // rule it exists to pin has quietly regressed.
    assert!(
        stderr.contains("would overwrite the mix"),
        "the reason should name the collision: {stderr}"
    );
    assert!(
        !out.exists(),
        "the guard should fire before anything is written"
    );
}

/// The resolver's last-resort fallback used to hand a path back untouched
/// when even its *parent* did not exist — which is exactly what a run
/// creating its own `--stems` directory looks like. `./new` and `new` are two
/// spellings of one directory, compared as different, so the guard passed,
/// the `lead` stem was written, and the report was written over it at exit 0.
#[test]
fn render_refuses_a_report_that_lands_on_a_stem_in_a_directory_it_creates() {
    let dir = case_dir("report_over_unborn_stem");
    std::fs::write(dir.join("score.ron"), SCORE).unwrap();
    let out = dir.join("mix.wav");

    // Relative, from the run's own working directory, because that is where
    // the two spellings actually differ: `Path`'s own equality folds a `.`
    // in the *middle* of a path but keeps a leading one, so `./new/lead.wav`
    // and `new/lead.wav` compared unequal and got past the raw check.
    let output = cochlea()
        .current_dir(&dir)
        .args([
            "render",
            "score.ron",
            "--out",
            "mix.wav",
            "--stems",
            "./new",
            "--report",
            "new/lead.wav",
            "--verify",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "a report landing on a stem must be refused: {stderr}"
    );
    assert!(
        stderr.contains("--report"),
        "the reason should name the flag: {stderr}"
    );
    assert!(
        !dir.join("new").exists(),
        "the guard fires before the stems directory is created"
    );
    assert!(!out.exists(), "...and before the mix is written");
}

/// `spectro` decodes the whole input before writing the PNG, so an aliasing
/// `--out` destroys it at exit 0. A `.wav` output is refused by the PNG
/// encoder by luck, not by design — but `cochlea_decode::load` identifies a
/// file by its magic bytes when the extension isn't an audio one, so an
/// audio file *named* `.png` decoded fine and was then written over. The MCP
/// `spectrogram` tool has always guarded this; the subcommand it mirrors did
/// not.
#[test]
fn spectro_refuses_to_write_its_png_over_its_own_input() {
    let dir = case_dir("spectro_over_input");
    let wav = render_wav(&dir);
    // A WAV named `.png`: decode sniffs the magic bytes, the PNG encoder is
    // happy with the extension, and nothing else stood between them.
    let disguised = dir.join("audio.png");
    std::fs::copy(&wav, &disguised).unwrap();
    let before = std::fs::read(&disguised).unwrap();

    let output = cochlea()
        .args([
            "spectro",
            disguised.to_str().unwrap(),
            "--out",
            disguised.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "--out onto the input must be refused"
    );
    assert_eq!(
        std::fs::read(&disguised).unwrap(),
        before,
        "the input audio must be untouched"
    );
}

/// `eval` writes its report only after every pair has been compared, so a
/// `--json` pointed into the candidate (or reference) set destroyed a golden
/// file while the run reported the comparison it had just made — "1/1
/// passed", exit 0, and a 208-byte JSON report where a WAV had been.
#[test]
fn eval_refuses_to_write_its_report_over_a_candidate_or_reference() {
    let dir = case_dir("eval_over_candidate");
    let wav = render_wav(&dir);
    let cands = dir.join("cands");
    let refs = dir.join("refs");
    std::fs::create_dir_all(&cands).unwrap();
    std::fs::create_dir_all(&refs).unwrap();
    std::fs::copy(&wav, cands.join("a.wav")).unwrap();
    std::fs::copy(&wav, refs.join("a.wav")).unwrap();
    let before = std::fs::read(cands.join("a.wav")).unwrap();

    for target in [cands.join("a.wav"), refs.join("a.wav")] {
        let output = cochlea()
            .args([
                "eval",
                "--candidates",
                cands.to_str().unwrap(),
                "--references",
                refs.to_str().unwrap(),
                "--json",
                target.to_str().unwrap(),
            ])
            .output()
            .unwrap();

        assert!(
            !output.status.success(),
            "--json onto {} must be refused",
            target.display()
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            before,
            "{} must be untouched",
            target.display()
        );
    }
}

/// The same fold, on the read side: `probe` writing its report over the file
/// it is probing, spelled with a different case.
#[test]
fn probe_refuses_to_overwrite_its_input_spelled_with_a_different_case() {
    let dir = case_dir("probe_case_alias");
    let wav = render_wav(&dir);
    let before = std::fs::read(&wav).unwrap();
    let aliased = dir.join("IN.WAV");

    let output = cochlea()
        .args([
            "probe",
            wav.to_str().unwrap(),
            "--json",
            aliased.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "--json onto a case-variant of the input must be refused"
    );
    assert_eq!(
        std::fs::read(&wav).unwrap(),
        before,
        "the probed audio must be untouched"
    );
}

/// `load_score` does not require a `.ron` extension, so a score kept as
/// `d/lead.wav` plus `--stems d` and a track named `lead` used to read the
/// score, render, then overwrite it — at exit 0.
#[test]
fn render_refuses_a_stem_that_would_overwrite_the_input_score() {
    let dir = case_dir("stem_over_score");
    let score = dir.join("lead.wav"); // a score, named like a WAV
    std::fs::write(&score, SCORE).unwrap();

    let output = cochlea()
        .args([
            "render",
            score.to_str().unwrap(),
            "--out",
            dir.join("mix.wav").to_str().unwrap(),
            "--stems",
            dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "a stem landing on the input score must be refused"
    );
    assert_eq!(
        std::fs::read_to_string(&score).unwrap(),
        SCORE,
        "the input score must be intact, not a WAV"
    );
}

/// The same escape, carried by an untrusted *binary* file rather than a
/// hand-written score: MIDI meta event 0x03 becomes the track name verbatim
/// (`crates/score/src/midi.rs`), so `import` then `render --stems` was a
/// two-step arbitrary write from a downloaded `.mid`. Import still succeeds
/// — a score is data, not a write — and the render is what refuses.
#[test]
fn a_midi_carried_track_name_cannot_escape_the_stems_dir() {
    let dir = case_dir("midi_carried_escape");
    let victim = dir.join("victim.wav");
    std::fs::write(&victim, b"UNTOUCHED").unwrap();
    let name = victim.with_extension("").to_str().unwrap().to_owned();

    // Minimal format-0 SMF: a track-name meta event holding the path, one
    // note, end of track. The name is longer than 127 bytes, so its length
    // is a two-byte VLQ.
    let mut vlq = Vec::new();
    let mut n = name.len();
    let mut digits = vec![(n & 0x7F) as u8];
    n >>= 7;
    while n > 0 {
        digits.push(((n & 0x7F) as u8) | 0x80);
        n >>= 7;
    }
    digits.reverse();
    vlq.extend(digits);

    let mut events = vec![0x00, 0xFF, 0x03];
    events.extend(&vlq);
    events.extend(name.as_bytes());
    events.extend([0x00, 0x90, 0x3C, 0x64]); // note on
    events.extend([0x60, 0x80, 0x3C, 0x00]); // note off
    events.extend([0x00, 0xFF, 0x2F, 0x00]); // end of track

    let mut midi = b"MThd".to_vec();
    midi.extend(6u32.to_be_bytes());
    midi.extend([0, 0, 0, 1, 0, 96]); // format 0, 1 track, 96 PPQ
    midi.extend(b"MTrk");
    midi.extend((events.len() as u32).to_be_bytes());
    midi.extend(&events);

    let mid = dir.join("evil.mid");
    let score = dir.join("from_midi.ron");
    std::fs::write(&mid, &midi).unwrap();

    let status = cochlea()
        .args([
            "import",
            mid.to_str().unwrap(),
            "--out",
            score.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "import itself should still succeed");

    let stems = dir.join("stems");
    let output = cochlea()
        .args([
            "render",
            score.to_str().unwrap(),
            "--out",
            dir.join("mix.wav").to_str().unwrap(),
            "--stems",
            stems.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "a MIDI-carried path-shaped track name must fail the render"
    );
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        b"UNTOUCHED",
        "the file the MIDI track name pointed at must be untouched"
    );
}

/// The guard must not over-fire: ordinary track names still export stems.
#[test]
fn render_still_writes_stems_for_ordinary_track_names() {
    let dir = case_dir("stems_ok");
    let score = dir.join("score.ron");
    std::fs::write(&score, SCORE).unwrap();
    let stems = dir.join("stems");

    let status = cochlea()
        .args([
            "render",
            score.to_str().unwrap(),
            "--out",
            dir.join("mix.wav").to_str().unwrap(),
            "--stems",
            stems.to_str().unwrap(),
        ])
        .status()
        .unwrap();

    assert!(
        status.success(),
        "an ordinary score should still export stems"
    );
    assert!(
        stems.join("lead.wav").exists(),
        "the stem should be written"
    );
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
