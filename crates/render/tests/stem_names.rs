//! Stem file names are derived from track names, and a track name is score
//! data — free-form text that can be hand-authored or, through `cochlea
//! import`, lifted verbatim out of a MIDI track-name meta event. Before the
//! [`cochlea_render::stem_file_name`] rule, `write_stems_as` fed that text
//! straight to `Path::join`, which *replaces* the base path when the
//! argument is absolute: a track named `/tmp/x` wrote `/tmp/x.wav` and one
//! named `../../x` climbed out of the stems directory. Reproduced against
//! 0.6.0 on both front ends, and (over MCP) it escaped `--root` while every
//! path argument stayed inside it.
//!
//! These tests pin the rule, and pin that a rejected name leaves *nothing*
//! on disk — a partial stem set is its own kind of surprise.

use cochlea_render::{RenderError, stem_file_name};
use cochlea_score::{Dur, Instrument, Pitch, Ppq, SampleRate, Score, Vel, bar};

fn case_dir(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("stems_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A one-note score whose single track carries `name`.
fn score_with_track(name: &str) -> Score {
    Score::new(SampleRate(48_000), Ppq(960))
        .track(name, Instrument::preset("sine"))
        .note(name, bar(1), Dur::quarter(), Pitch::A4, Vel(96))
}

#[test]
fn ordinary_names_are_accepted_verbatim() {
    for name in [
        "lead",
        "bass",
        "track1_ch2",
        "Kick Drum",
        "sci-fi.pad",
        "..dots",  // leading dots are fine: only a *component* `..` traverses
        ".hidden", // as is a leading dot; it names a hidden file, not a path
        "naïve",   // non-ASCII is an ordinary file name
        // A bare `..` is safe *here* and deliberately allowed: the appended
        // extension makes it `...wav`, an ordinary file inside the stems
        // directory, not a parent component. Only `../…` traverses, and the
        // separator check catches that.
        "..",
    ] {
        assert_eq!(
            stem_file_name(name).unwrap(),
            format!("{name}.wav"),
            "{name:?} is an ordinary file name and must be allowed"
        );
    }
}

#[test]
fn path_shaped_names_are_refused() {
    // Each of these escaped the stems directory before the fix.
    for name in [
        "/etc/passwd",          // absolute: `join` discards the base entirely
        "/tmp/pwned",           // the reproduced absolute case
        "../../escaped",        // the reproduced relative case
        "../sibling",           // a parent component with a separator
        "sub/dir",              // even a downward separator is not ours to make
        "a\\b",                 // Windows separator, refused on every platform
        "C:\\Windows\\evil",    // Windows prefix
        "\\\\server\\share\\x", // UNC
        "with\0nul",            // rejected cleanly instead of an OS error
        "",                     // degenerate: would write a file named `.wav`
        "trailing/",            //
        // Not separators, but not portable either — each means something
        // different (or nothing) on Windows, and a score is portable data.
        "C:",          // a drive prefix on Windows, an ordinary name on Unix
        "..:x",        // an NTFS alternate data stream on the *parent* dir
        "stream:name", // ditto, the general shape
        "why?",        // <>"|?* are illegal in a Win32 file name
        "a*b",         //
        "pipe|it",     //
        "quote\"it",   //
        "less<than",   //
        "bell\u{7}",   // control characters
        "NUL",         // Win32 device names resolve from any directory,
        "nul",         // case-insensitively,
        "CON",         //
        "COM1",        //
        "LPT9",        //
        "NUL.foo",     // and are matched before the first dot
    ] {
        let err = stem_file_name(name)
            .expect_err(&format!("{name:?} must be refused as a stem file name"));
        assert!(
            matches!(err, RenderError::UnwritableStemName { .. }),
            "{name:?} gave the wrong error: {err}"
        );
        // The message has to name the offending track, or the author can't
        // tell which of thirty tracks to rename. Quoted with `{:?}`, so a
        // name containing a backslash or a NUL stays legible on one line.
        assert!(
            err.to_string().contains(&format!("{name:?}")),
            "error should quote the track name {name:?}: {err}"
        );
    }
}

#[test]
fn a_refused_name_writes_nothing_at_all() {
    // Validate-before-write: one bad track must not leave a half-written
    // stem set behind, and must not even create the directory.
    let dir = case_dir("nothing_written");
    let stems = dir.join("stems");
    let score = score_with_track("good").track("../../escaped", Instrument::preset("sine"));

    let rendered = cochlea_render::render(&score).expect("score renders");
    let err = rendered
        .write_stems(&stems)
        .expect_err("a path-shaped track name must refuse the whole write");
    assert!(
        matches!(err, RenderError::UnwritableStemName { .. }),
        "{err}"
    );

    assert!(
        !stems.exists(),
        "the stems directory must not be created when a name is refused"
    );
    // And nothing climbed out of it either.
    assert!(
        !dir.join("escaped.wav").exists() && !dir.parent().unwrap().join("escaped.wav").exists(),
        "a refused name must not have written outside the stems directory"
    );
}

#[test]
fn an_absolute_track_name_cannot_write_outside_the_stems_dir() {
    // The reproduced sandbox escape, at the library boundary: the track name
    // is an absolute path to a file that already exists, and the write must
    // not touch it.
    let dir = case_dir("absolute_escape");
    let victim = dir.join("victim.wav");
    std::fs::write(&victim, b"UNTOUCHED").unwrap();

    let score = score_with_track(victim.with_extension("").to_str().unwrap());
    let rendered = cochlea_render::render(&score).expect("score renders");
    let err = rendered
        .write_stems(dir.join("stems"))
        .expect_err("an absolute track name must be refused");
    assert!(
        matches!(err, RenderError::UnwritableStemName { .. }),
        "{err}"
    );

    assert_eq!(
        std::fs::read(&victim).unwrap(),
        b"UNTOUCHED",
        "the file the track name pointed at must be untouched"
    );
}

/// A well-formed *name* is not enough. If something already sits at the
/// stem's path and links out of the directory, `File::create` follows it —
/// the same trap `resolve_write` was hardened against on the MCP side.
/// Reproduced: a symlink planted at `<root>/stems/lead.wav` let a render
/// truncate a file outside `--root` while every name and argument was
/// legitimate.
#[test]
#[cfg(unix)]
fn a_symlink_at_a_stem_path_cannot_redirect_the_write_outside() {
    let dir = case_dir("symlink_escape");
    let stems = dir.join("stems");
    std::fs::create_dir_all(&stems).unwrap();
    let victim = dir.join("victim.wav");
    std::fs::write(&victim, b"UNTOUCHED").unwrap();
    std::os::unix::fs::symlink(&victim, stems.join("lead.wav")).unwrap();

    let rendered = cochlea_render::render(&score_with_track("lead")).expect("score renders");
    let err = rendered
        .write_stems(&stems)
        .expect_err("a symlink pointing out of the stems dir must be refused");
    assert!(
        matches!(err, RenderError::UnwritableStemName { .. }),
        "{err}"
    );
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        b"UNTOUCHED",
        "the symlink target outside the stems dir must be untouched"
    );
}

/// ...but the rule is containment, not a ban on links: one pointing *inside*
/// the directory is ordinary and must still work.
#[test]
#[cfg(unix)]
fn a_symlink_that_stays_inside_the_stems_dir_is_fine() {
    let dir = case_dir("symlink_inside");
    let stems = dir.join("stems");
    std::fs::create_dir_all(&stems).unwrap();
    let real = stems.join("real.wav");
    std::fs::write(&real, b"placeholder").unwrap();
    std::os::unix::fs::symlink(&real, stems.join("lead.wav")).unwrap();

    cochlea_render::render(&score_with_track("lead"))
        .expect("score renders")
        .write_stems(&stems)
        .expect("a link that stays inside the stems dir is not an escape");
    assert_eq!(&std::fs::read(&real).unwrap()[..4], b"RIFF");
}

/// Two tracks whose names differ only by case are distinct to a score but
/// one file on macOS and Windows, so writing both would silently drop a
/// stem. Refused as a set, before anything is written.
#[test]
fn names_that_differ_only_by_case_are_refused() {
    let dir = case_dir("case_collision");
    let stems = dir.join("stems");
    let score = score_with_track("Lead").track("lead", Instrument::preset("sine"));

    let err = cochlea_render::render(&score)
        .expect("score renders")
        .write_stems(&stems)
        .expect_err("case-colliding stem names must be refused");
    assert!(
        matches!(err, RenderError::CollidingStemNames { .. }),
        "{err}"
    );
    assert!(!stems.exists(), "nothing should be written");
}

/// A name long enough to fail at the filesystem is caught by the rule
/// instead, so the all-or-nothing promise holds rather than breaking partway
/// through the set with ENAMETOOLONG.
#[test]
fn an_overlong_name_is_refused_by_the_rule_not_the_filesystem() {
    let long = "a".repeat(300);
    let err = stem_file_name(&long).expect_err("an overlong name must be refused");
    assert!(
        err.to_string().contains("too long"),
        "the reason should say so: {err}"
    );
}

#[test]
fn ordinary_scores_still_write_their_stems() {
    // The guard must not cost the ordinary case anything.
    let dir = case_dir("happy_path");
    let stems = dir.join("stems");
    let score = score_with_track("lead").track("bass", Instrument::preset("sine"));

    cochlea_render::render(&score)
        .expect("score renders")
        .write_stems(&stems)
        .expect("ordinary track names must still write");

    assert!(stems.join("lead.wav").exists());
    assert!(stems.join("bass.wav").exists());
}
