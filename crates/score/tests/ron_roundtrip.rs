//! RON data form: both round-trip directions, the committed example, and
//! load-time failure modes.

use cochlea_score::*;

fn build_score() -> Score {
    Score::new(SampleRate(48_000), Ppq(960))
        .time_signature(4, 4)
        .tempo(Ticks(0), Bpm(112.0))
        .tempo(bar(3), Bpm(140.5))
        .track("lead", Instrument::preset("saw_lead"))
        .insert("lead", Insert::preset("reverb"))
        .note("lead", bar(1).beat(1), Dur::quarter(), Pitch::A4, Vel(96))
        .note(
            "lead",
            bar(1).beat(2).plus(Dur::eighth()),
            Dur::eighth().dotted(),
            Pitch::CS5,
            Vel(88),
        )
        .track("bass", Instrument::preset("square_bass"))
        .note("bass", bar(2), Dur::half(), Pitch::A2, Vel(110))
        .automate(
            "lead",
            Param::CUTOFF_HZ,
            keys![
                (bar(1), 400.0, ease_in_out()),
                (bar(2), 2_000.0, ease_out()),
                (bar(3), 4_000.0),
            ],
        )
        .with_verify(VerifySpec::TruePeakBelow { dbtp: -1.0 })
        .with_verify(VerifySpec::SilentAfter {
            at: Ticks(4 * 3840),
        })
        .with_verify(VerifySpec::TempoIs {
            bpm: 112.0,
            tol_bpm: 1.0,
            min_bpm: Some(60.0),
            max_bpm: None,
        })
        .with_verify(VerifySpec::HasClearRhythm { expected: true })
        .with_verify(VerifySpec::StereoWidthWithin { min: 0.0, max: 1.0 })
        .with_verify(VerifySpec::LraBelow { lu: 12.0 })
        .with_verify(VerifySpec::SectionCount { min: 1, max: 12 })
}

#[test]
fn score_to_ron_to_score_is_identity() {
    let score = build_score();
    let ron = score.to_ron().unwrap();
    let reloaded = Score::from_ron(&ron).unwrap();
    assert_eq!(score, reloaded, "round-trip changed the score:\n{ron}");
}

#[test]
fn ron_to_score_to_ron_is_stable() {
    // Serialization is canonical: a second pass through text is a fixpoint.
    let once = build_score().to_ron().unwrap();
    let twice = Score::from_ron(&once).unwrap().to_ron().unwrap();
    assert_eq!(once, twice);
}

#[test]
fn the_committed_example_loads_validates_and_round_trips() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/scores/first_light.ron"
    );
    let text = std::fs::read_to_string(path).unwrap();
    let score = Score::from_ron(&text).unwrap();
    assert_eq!(score.sample_rate(), SampleRate(48_000));
    assert_eq!(score.tracks().len(), 2);
    assert_eq!(score.tracks()[0].notes.len(), 6);
    assert_eq!(score.verify_specs().len(), 2);
    let errors: Vec<_> = score
        .validate_standalone()
        .into_iter()
        .filter(|f| f.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let reloaded = Score::from_ron(&score.to_ron().unwrap()).unwrap();
    assert_eq!(score, reloaded);
}

#[test]
fn dotted_and_triplet_sugar_canonicalize_but_stay_equal() {
    let s = Score::new(SampleRate(48_000), Ppq(960))
        .track("t", Instrument::preset("sine"))
        .note("t", bar(1), Dur::quarter().dotted(), Pitch::A4, Vel(96));
    let ron = s.to_ron().unwrap();
    assert!(
        ron.contains("\"3/8\""),
        "dotted quarter canonicalizes:\n{ron}"
    );
    assert_eq!(Score::from_ron(&ron).unwrap(), s);
}

#[test]
fn wrong_version_is_rejected() {
    let text = r#"Score(version: 2, sample_rate: 48000, ppq: 960, tempo: [], tracks: [])"#;
    assert!(matches!(
        Score::from_ron(text),
        Err(ScoreError::UnsupportedVersion(2))
    ));
}

#[test]
fn bad_pitch_and_bad_duration_fail_with_context() {
    let bad_pitch = r#"Score(version: 1, sample_rate: 48000, ppq: 960,
        tempo: [(tick: 0, bpm: 120.0)],
        tracks: [Track(name: "t", instrument: Preset("sine"),
                       notes: [Note(at: (1, 1), dur: "1/4", pitch: "X4", vel: 96)])])"#;
    assert!(matches!(
        Score::from_ron(bad_pitch),
        Err(ScoreError::BadPitch(_))
    ));
    let bad_dur = r#"Score(version: 1, sample_rate: 48000, ppq: 960,
        tempo: [(tick: 0, bpm: 120.0)],
        tracks: [Track(name: "t", instrument: Preset("sine"),
                       notes: [Note(at: (1, 1), dur: "zero", pitch: "A4", vel: 96)])])"#;
    assert!(matches!(
        Score::from_ron(bad_dur),
        Err(ScoreError::BadDur(_))
    ));
}

#[test]
fn unknown_fields_are_rejected_not_ignored() {
    let text = r#"Score(version: 1, sample_rate: 48000, ppq: 960, swing: 0.5,
        tempo: [(tick: 0, bpm: 120.0)], tracks: [])"#;
    assert!(Score::from_ron(text).is_err());
}

#[test]
fn far_future_tempo_tick_is_refused_at_load_not_panicked_on() {
    // Adversarial review, Finding 1: a crafted tempo tick near u64::MAX used
    // to reach unchecked `mul_div` in tempo_map() and panic the renderer.
    // It must now fail cleanly at load, before any arithmetic runs.
    let text = r#"Score(version: 1, sample_rate: 48000, ppq: 960,
        tempo: [(tick: 0, bpm: 120.0), (tick: 18446744073709551000, bpm: 120.0)],
        tracks: [Track(name: "lead", instrument: Preset("sine"),
                       notes: [Note(at: (1, 1), dur: "1/4", pitch: "A4", vel: 96)])])"#;
    assert!(matches!(
        Score::from_ron(text),
        Err(ScoreError::PositionTooFar {
            what: "tempo change",
            ..
        })
    ));
}

#[test]
fn a_tempo_tick_at_the_bound_is_accepted_and_its_map_never_overflows() {
    // Exactly at the ceiling: allowed, and building the tempo map (the code
    // that used to panic) completes without overflow. It exceeds the one-hour
    // render cap, but that's the render layer's job to report, not a panic.
    let at_bound = Ticks::MAX.0;
    let score = Score::new(SampleRate(48_000), Ppq(960))
        .try_tempo(Ticks(0), Bpm(120.0))
        .unwrap()
        .try_tempo(Ticks(at_bound), Bpm(90.0))
        .unwrap();
    let map = score.tempo_map();
    // Worst-case-ish arithmetic actually runs: no panic, finite sample index.
    let _ = map.sample_at(Ticks(at_bound));

    // One past the ceiling is refused.
    assert!(matches!(
        Score::new(SampleRate(48_000), Ppq(960)).try_tempo(Ticks(at_bound + 1), Bpm(120.0)),
        Err(ScoreError::PositionTooFar { .. })
    ));
}

#[test]
fn a_raw_tick_note_past_the_bound_is_refused() {
    // The note path is normally bounded by (u32 bar, u32 beat), but a raw-tick
    // position or a huge raw-tick duration must not slip past the same guard.
    let huge = Ticks::MAX.0;
    assert!(matches!(
        Score::new(SampleRate(48_000), Ppq(960))
            .try_track("t", Instrument::preset("sine"))
            .unwrap()
            .try_note("t", Ticks(huge), Dur::ticks(4_096), Pitch::A4, Vel(96)),
        Err(ScoreError::PositionTooFar {
            what: "note end",
            ..
        })
    ));
}

#[test]
fn raw_tick_durations_survive_the_data_form() {
    // 961 ticks is off every musical grid; it canonicalizes to a reduced
    // fraction of the whole note and reloads to the same tick count.
    let s = Score::new(SampleRate(48_000), Ppq(960))
        .track("t", Instrument::preset("sine"))
        .note("t", Ticks(7), Dur::ticks(961), Pitch::A4, Vel(96));
    let reloaded = Score::from_ron(&s.to_ron().unwrap()).unwrap();
    assert_eq!(reloaded.tracks()[0].notes[0].at, Ticks(7));
    assert_eq!(reloaded.tracks()[0].notes[0].dur, Ticks(961));
}
