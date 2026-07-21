//! MIDI import: hand-built SMF byte streams in, scores out. The builders
//! below construct real chunk/VLQ/running-status encodings so the parser
//! is tested against the wire format, not against itself.

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

/// Events as (delta, raw bytes) pairs, closed with end-of-track.
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

fn tempo_event(us_per_quarter: u32) -> Vec<u8> {
    let b = us_per_quarter.to_be_bytes();
    vec![0xFF, 0x51, 0x03, b[1], b[2], b[3]]
}

fn time_signature_event(num: u8, den_pow2: u8) -> Vec<u8> {
    vec![0xFF, 0x58, 0x04, num, den_pow2, 24, 8]
}

fn track_name_event(name: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0x03, name.len() as u8];
    out.extend(name.as_bytes());
    out
}

// ---------------------------------------------------------------- imports

/// Format 1: tempo track (120 BPM, 3/4) + a named melodic track using
/// running status + a channel-10 drum track.
fn demo_file() -> Vec<u8> {
    let mut bytes = header(1, 3, 480);
    bytes.extend(track(&[
        (0, time_signature_event(3, 2)),
        (0, tempo_event(500_000)),   // 120 BPM
        (960, tempo_event(400_000)), // 150 BPM two quarters in
    ]));
    bytes.extend(track(&[
        (0, track_name_event("lead")),
        (0, vec![0xC0, 81]),       // program: synth lead
        (0, vec![0x90, 60, 100]),  // C4 on
        (480, vec![60, 0]),        // C4 off via running status + vel 0
        (0, vec![64, 90]),         // E4 on (running status)
        (480, vec![0x80, 64, 64]), // E4 off
    ]));
    bytes.extend(track(&[
        (0, vec![0x99, 36, 110]),  // kick on (ch 10)
        (10, vec![0x89, 36, 64]),  // kick off
        (470, vec![0x99, 38, 96]), // snare on
        (10, vec![0x89, 38, 64]),  // snare off
        (230, vec![0x99, 42, 80]), // closed hat on
        (10, vec![0x89, 42, 64]),  // hat off
    ]));
    bytes
}

#[test]
fn format_1_file_imports_with_timing_intact() {
    let MidiImport { score, warnings } = import_midi(&demo_file(), SampleRate(48_000)).unwrap();

    assert_eq!(score.ppq(), Ppq(480), "division becomes PPQ verbatim");
    assert_eq!(score.signature(), TimeSignature { beats: 3, unit: 4 });

    let tempo: Vec<(Ticks, Bpm)> = score.tempo_changes().collect();
    assert_eq!(tempo.len(), 2, "{tempo:?}");
    assert_eq!(tempo[0].0, Ticks(0));
    assert!((tempo[0].1.0 - 120.0).abs() < 1e-9);
    assert_eq!(tempo[1].0, Ticks(960));
    assert!((tempo[1].1.0 - 150.0).abs() < 1e-9);

    // The melodic track: named from meta, program 81 -> saw_lead, exact
    // note ticks/durations, running-status + vel-0-as-off both handled.
    let lead = score
        .tracks()
        .iter()
        .find(|t| t.name == "lead")
        .expect("named track exists");
    assert_eq!(lead.instrument, Instrument::preset("saw_lead"));
    assert_eq!(lead.notes.len(), 2);
    assert_eq!(
        (lead.notes[0].at, lead.notes[0].dur),
        (Ticks(0), Ticks(480))
    );
    assert_eq!(lead.notes[0].pitch, Pitch::C4);
    assert_eq!(lead.notes[0].vel, Vel(100));
    assert_eq!(
        (lead.notes[1].at, lead.notes[1].dur),
        (Ticks(480), Ticks(480))
    );
    assert_eq!(lead.notes[1].pitch, Pitch::E4);

    // Channel 10 split into kick/snare/hat tracks with our conventional
    // percussion pitches.
    let find = |name: &str| {
        score
            .tracks()
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} track missing"))
    };
    assert_eq!(find("kick").instrument, Instrument::preset("kick"));
    assert_eq!(find("kick").notes[0].at, Ticks(0));
    assert_eq!(find("snare").notes[0].at, Ticks(480));
    assert_eq!(find("noise_hat").notes[0].at, Ticks(720));

    // Mapping guesses are reported, not silent.
    assert!(
        warnings.iter().any(|w| w.contains("saw_lead")),
        "{warnings:?}"
    );

    // The imported score round-trips through the RON data form.
    let ron = score.to_ron().unwrap();
    let back = Score::from_ron(&ron).unwrap();
    assert_eq!(back, score);
}

#[test]
fn format_0_single_track_splits_by_channel() {
    let mut bytes = header(0, 1, 96);
    bytes.extend(track(&[
        (0, tempo_event(600_000)), // 100 BPM
        (0, vec![0x90, 60, 80]),   // ch 1
        (0, vec![0x91, 48, 80]),   // ch 2, program never set -> sine
        (96, vec![0x80, 60, 0]),
        (0, vec![0x81, 48, 0]),
    ]));
    let MidiImport { score, .. } = import_midi(&bytes, SampleRate(48_000)).unwrap();
    assert_eq!(score.tracks().len(), 2, "{:?}", score.tracks());
    for t in score.tracks() {
        assert_eq!(t.instrument, Instrument::preset("sine"));
        assert_eq!(t.notes.len(), 1);
    }
}

#[test]
fn unclosed_and_zero_length_notes_survive() {
    let mut bytes = header(0, 1, 96);
    bytes.extend(track(&[
        (0, vec![0x90, 60, 80]),  // never gets an off -> closes at track end
        (48, vec![0x90, 62, 80]), // on and off at the same tick -> 1 tick
        (0, vec![0x80, 62, 0]),
        (48, vec![0xFF, 0x01, 3, b'a', b'b', b'c']), // text meta, skipped
    ]));
    let MidiImport { score, .. } = import_midi(&bytes, SampleRate(48_000)).unwrap();
    let notes = &score.tracks()[0].notes;
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert_eq!(notes[0].dur, Ticks(96), "closed at track end");
    assert_eq!(notes[1].dur, Ticks(1), "zero-length hit kept, one tick");
}

#[test]
fn unsupported_files_error_clearly() {
    // Format 2.
    let mut f2 = header(2, 1, 96);
    f2.extend(track(&[]));
    let err = import_midi(&f2, SampleRate(48_000)).unwrap_err();
    assert!(err.to_string().contains("format 2"), "{err}");

    // SMPTE division (bit 15 set).
    let smpte = header(1, 0, 0x8000 | 0xE7_28);
    let err = import_midi(&smpte, SampleRate(48_000)).unwrap_err();
    assert!(err.to_string().contains("SMPTE"), "{err}");

    // Not a MIDI file at all.
    let err = import_midi(b"RIFF....WAVE", SampleRate(48_000)).unwrap_err();
    assert!(err.to_string().contains("MThd"), "{err}");

    // Truncated mid-track.
    let mut cut = demo_file();
    cut.truncate(cut.len() - 10);
    assert!(import_midi(&cut, SampleRate(48_000)).is_err());
}
