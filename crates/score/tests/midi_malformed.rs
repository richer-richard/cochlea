//! Malformed-input hardening for the SMF importer: every one of these
//! feeds the parser bytes no encoder would produce — hostile meta payloads,
//! truncations, single-byte corruptions, absurd header fields — and asserts
//! only that it comes back with a `Result`, never a panic.
//!
//! An importer reads untrusted files, so "returns an error" and "crashes"
//! are very different failures. `import_midi` is also reachable from the
//! long-lived MCP server, where a panic would otherwise take the process
//! down (the `catch_unwind` backstop contains it, but containing a crash is
//! not the same as not crashing).
//!
//! Regression origin: a time-signature meta event whose denominator byte is
//! its power-of-two *exponent* was shifted unvalidated (`1u32 <<
//! payload[1]`), so any exponent past 31 overflowed the shift — a debug-build
//! panic and a garbage denominator in release. A single byte flip in an
//! otherwise-valid file was enough to reach it.

use cochlea_score::*;

// ------------------------------------------------------------- SMF builder

fn vlq(mut v: u32) -> Vec<u8> {
    let mut out = vec![(v & 0x7F) as u8];
    v >>= 7;
    while v > 0 {
        out.insert(0, 0x80 | (v & 0x7F) as u8);
        v >>= 7;
    }
    out
}

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(data);
    out
}

fn header(format: u16, ntrks: u16, division: u16) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend(format.to_be_bytes());
    data.extend(ntrks.to_be_bytes());
    data.extend(division.to_be_bytes());
    chunk(b"MThd", &data)
}

fn track(events: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut data = Vec::new();
    for (delta, bytes) in events {
        data.extend(vlq(*delta));
        data.extend(bytes);
    }
    data.extend(vlq(0));
    data.extend([0xFF, 0x2F, 0x00]);
    chunk(b"MTrk", &data)
}

/// A well-formed one-note file, the base every mutation starts from.
fn valid_file() -> Vec<u8> {
    let mut file = header(0, 1, 480);
    file.extend(track(&[
        (0, vec![0xFF, 0x58, 0x04, 4, 2, 24, 8]),
        (0, vec![0xC0, 40]),
        (0, vec![0x90, 60, 100]),
        (480, vec![0x80, 60, 0]),
        (0, vec![0xB0, 7, 100]),
        (0, vec![0xE0, 0, 64]),
    ]));
    file
}

// -------------------------------------------------------------- the checks

/// The exact regression: a time-signature denominator exponent past 31.
/// It must import cleanly (keeping 4/4) and say so, not panic.
#[test]
fn an_absurd_time_signature_denominator_is_a_warning_not_a_panic() {
    for exponent in [32u8, 63, 127, 200, 255] {
        let mut file = header(0, 1, 480);
        file.extend(track(&[
            (0, vec![0xFF, 0x58, 0x04, 4, exponent, 24, 8]),
            (0, vec![0x90, 60, 100]),
            (480, vec![0x80, 60, 0]),
        ]));

        let imported = import_midi(&file, SampleRate(48_000))
            .unwrap_or_else(|e| panic!("exponent {exponent} should import, got {e}"));

        assert!(
            imported
                .warnings
                .iter()
                .any(|w| w.contains("out-of-range time-signature denominator")),
            "exponent {exponent} should be reported as out of range, got {:?}",
            imported.warnings
        );
        // The note still arrives — one bad meta event doesn't lose the music.
        assert_eq!(
            imported.score.tracks().len(),
            1,
            "exponent {exponent} should still import its note track"
        );
    }
}

/// Denominators that *are* representable still parse as themselves, so the
/// fix bounds the shift without narrowing legitimate input.
#[test]
fn representable_time_signature_denominators_still_import() {
    // 2^2 = 4 (quarter) and 2^3 = 8 (eighth) are the everyday cases.
    for (num, exponent, den) in [(3u8, 2u8, 4u32), (6, 3, 8), (7, 4, 16)] {
        let mut file = header(0, 1, 480);
        file.extend(track(&[
            (0, vec![0xFF, 0x58, 0x04, num, exponent, 24, 8]),
            (0, vec![0x90, 60, 100]),
            (480, vec![0x80, 60, 0]),
        ]));

        let imported = import_midi(&file, SampleRate(48_000)).expect("valid time signature");
        assert_eq!(
            imported.score.signature(),
            TimeSignature {
                beats: u32::from(num),
                unit: den,
            },
            "{num}/{den} should survive the import"
        );
        assert!(
            !imported
                .warnings
                .iter()
                .any(|w| w.contains("out-of-range time-signature denominator")),
            "{num}/{den} is representable and should not warn: {:?}",
            imported.warnings
        );
    }
}

/// Every meta type crossed with hostile payload bytes.
#[test]
fn hostile_meta_payloads_never_panic() {
    for meta in [0x00u8, 0x01, 0x03, 0x20, 0x2F, 0x51, 0x54, 0x58, 0x59, 0x7F] {
        for b0 in [0u8, 1, 4, 31, 32, 127, 128, 255] {
            for b1 in [0u8, 1, 2, 31, 32, 64, 127, 255] {
                let mut file = header(0, 1, 480);
                file.extend(track(&[
                    (0, vec![0xFF, meta, 0x04, b0, b1, 24, 8]),
                    (0, vec![0x90, 60, 100]),
                    (480, vec![0x80, 60, 0]),
                ]));
                let _ = import_midi(&file, SampleRate(48_000));
            }
        }
    }
}

/// Every prefix of a valid file: a truncation is an error, never a crash.
#[test]
fn truncations_never_panic() {
    let file = valid_file();
    for n in 0..file.len() {
        let _ = import_midi(&file[..n], SampleRate(48_000));
    }
}

/// Every single-byte corruption of a valid file — the mutation class that
/// originally reached the shift overflow.
#[test]
fn single_byte_corruptions_never_panic() {
    let base = valid_file();
    for i in 0..base.len() {
        for v in [0x00u8, 0x01, 0x7F, 0x80, 0xC0, 0xF0, 0xFF] {
            let mut mutated = base.clone();
            mutated[i] = v;
            let _ = import_midi(&mutated, SampleRate(48_000));
        }
    }
}

/// Header field space: unsupported formats and divisions are errors, and
/// nothing in the grid panics.
#[test]
fn header_field_space_never_panics() {
    for format in [0u16, 1, 2, 3, 65535] {
        for ntrks in [0u16, 1, 2, 255, 65535] {
            for division in [0u16, 1, 24, 480, 960, 0x7FFF, 0x8000, 0xFFFF] {
                let mut file = header(format, ntrks, division);
                file.extend(track(&[(0, vec![0x90, 60, 100]), (480, vec![0x80, 60, 0])]));
                let _ = import_midi(&file, SampleRate(48_000));
            }
        }
    }
}

/// Extreme delta times, around the tick-overflow guard.
#[test]
fn extreme_delta_times_never_panic() {
    for delta in [0u32, 1, 127, 128, 0x0FFF_FFFF] {
        for reps in [1usize, 2, 8] {
            let mut events = Vec::new();
            for _ in 0..reps {
                events.push((delta, vec![0x90u8, 60, 100]));
                events.push((delta, vec![0x80u8, 60, 0]));
            }
            let mut file = header(0, 1, 480);
            file.extend(track(&events));
            let _ = import_midi(&file, SampleRate(48_000));
        }
    }
}
