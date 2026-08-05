//! MIDI export, and the round-trip against the importer: timing (ticks,
//! tempo, time signature) must survive a Score → SMF → Score trip exactly;
//! instruments are a lossy GM label, so they're not asserted here.

use cochlea_score::*;

fn sample_score() -> Score {
    Score::new(SampleRate(48_000), Ppq(480))
        .time_signature(3, 4)
        .tempo(Ticks(0), Bpm(120.0))
        .tempo(bar(2), Bpm(90.0))
        .track("lead", Instrument::preset("saw_lead"))
        .note("lead", bar(1), Dur::quarter(), Pitch::A4, Vel(100))
        .note("lead", bar(1).beat(2), Dur::eighth(), Pitch::C5, Vel(80))
        .note("lead", bar(2), Dur::quarter(), Pitch::E5, Vel(90))
        .track("bass", Instrument::preset("square_bass"))
        .note("bass", bar(1), Dur::half(), Pitch::A2, Vel(110))
}

/// Every note's (start tick, duration ticks, MIDI pitch) across all tracks,
/// sorted — the timing fingerprint that must round-trip.
fn note_fingerprint(score: &Score) -> Vec<(u64, u64, u8)> {
    let mut v: Vec<(u64, u64, u8)> = score
        .tracks()
        .iter()
        .flat_map(|t| t.notes.iter().map(|n| (n.at.0, n.dur.0, n.pitch.0)))
        .collect();
    v.sort_unstable();
    v
}

#[test]
fn timing_survives_a_score_to_smf_to_score_round_trip() {
    let score = sample_score();
    let bytes = export_midi(&score).expect("export");
    let back = import_midi(&bytes, SampleRate(48_000)).expect("re-import");

    // PPQ / division and time signature are exact.
    assert_eq!(back.score.ppq(), Ppq(480));
    assert_eq!(back.score.signature(), score.signature());

    // Notes: same starts, durations, and pitches (instrument mapping aside).
    assert_eq!(note_fingerprint(&back.score), note_fingerprint(&score));

    // Tempo changes: same ticks, BPM within microsecond-quantization error.
    let orig: Vec<(u64, f64)> = score.tempo_changes().map(|(t, b)| (t.0, b.0)).collect();
    let round: Vec<(u64, f64)> = back
        .score
        .tempo_changes()
        .map(|(t, b)| (t.0, b.0))
        .collect();
    assert_eq!(orig.len(), round.len(), "tempo change count");
    for ((t0, b0), (t1, b1)) in orig.iter().zip(round.iter()) {
        assert_eq!(t0, t1, "tempo tick");
        assert!((b0 - b1).abs() < 0.01, "tempo bpm {b0} vs {b1}");
    }
}

#[test]
fn export_is_byte_deterministic() {
    let score = sample_score();
    assert_eq!(
        export_midi(&score).unwrap(),
        export_midi(&score).unwrap(),
        "export must be byte-identical across calls"
    );
}

#[test]
fn export_starts_with_a_valid_header() {
    let bytes = export_midi(&sample_score()).unwrap();
    assert_eq!(&bytes[..4], b"MThd", "starts with the header chunk");
    // Header length is 6, format 1, and ntrks = tempo track + 2 score tracks.
    assert_eq!(u32::from_be_bytes(bytes[4..8].try_into().unwrap()), 6);
    assert_eq!(u16::from_be_bytes(bytes[8..10].try_into().unwrap()), 1);
    assert_eq!(u16::from_be_bytes(bytes[10..12].try_into().unwrap()), 3);
}

#[test]
fn marimba_and_organ_survive_the_gm_round_trip() {
    // Most presets are a lossy GM label, but marimba and organ have exact GM
    // equivalents (program 12 = Marimba, 16 = Drawbar Organ), so they must
    // re-import as themselves rather than the roughest family default.
    let score = Score::new(SampleRate(48_000), Ppq(480))
        .track("mallet", Instrument::preset("marimba"))
        .note("mallet", bar(1), Dur::quarter(), Pitch::C5, Vel(100))
        .track("keys", Instrument::preset("organ"))
        .note("keys", bar(1), Dur::half(), Pitch::A2, Vel(90));
    let bytes = export_midi(&score).expect("export");
    let back = import_midi(&bytes, SampleRate(48_000)).expect("re-import");

    let presets: std::collections::HashSet<&str> = back
        .score
        .tracks()
        .iter()
        .map(|t| t.instrument.name())
        .collect();
    assert!(
        presets.contains("marimba"),
        "marimba should round-trip through GM 12, got {presets:?}"
    );
    assert!(
        presets.contains("organ"),
        "organ should round-trip through GM 16, got {presets:?}"
    );
}
