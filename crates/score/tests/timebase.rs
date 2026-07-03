//! Unit tests for tick math, bar/beat resolution, and the tempo map.

use cochlea_score::*;

fn score() -> Score {
    Score::new(SampleRate(48_000), Ppq(960))
}

#[test]
fn durations_resolve_exactly_at_960_ppq() {
    let s = score();
    let ticks = |d: Dur| {
        s.clone()
            .track("t", Instrument::preset("sine"))
            .note("t", Ticks(0), d, Pitch::A4, Vel(96))
            .tracks()[0]
            .notes[0]
            .dur
    };
    assert_eq!(ticks(Dur::quarter()), Ticks(960));
    assert_eq!(ticks(Dur::whole()), Ticks(3840));
    assert_eq!(ticks(Dur::eighth()), Ticks(480));
    assert_eq!(ticks(Dur::sixteenth()), Ticks(240));
    assert_eq!(ticks(Dur::quarter().dotted()), Ticks(1440));
    assert_eq!(ticks(Dur::eighth().triplet()), Ticks(320));
    assert_eq!(ticks(Dur::ticks(961)), Ticks(961));
}

#[test]
fn a_duration_off_the_tick_grid_is_an_error_not_a_rounding() {
    // 1/7 of a whole note at 960 PPQ: 3840/7 is not an integer.
    let err = Score::try_new(SampleRate(48_000), Ppq(960))
        .unwrap()
        .try_track("t", Instrument::preset("sine"))
        .unwrap()
        .try_note("t", Ticks(0), Dur::of(1, 7), Pitch::A4, Vel(96))
        .unwrap_err();
    assert!(matches!(err, ScoreError::NonIntegerTick { .. }), "{err}");
}

#[test]
fn bar_beat_positions_resolve_on_the_4_4_grid() {
    let s = score().track("t", Instrument::preset("sine"));
    let at = |p: Pos| s.resolve(p).unwrap();
    assert_eq!(at(bar(1).beat(1)), Ticks(0));
    assert_eq!(at(bar(1).beat(2)), Ticks(960));
    assert_eq!(at(bar(2)), Ticks(3840));
    assert_eq!(at(bar(2).beat(3)), Ticks(3840 + 2 * 960));
    assert_eq!(at(bar(1).beat(1).plus(Dur::eighth())), Ticks(480));
    assert_eq!(at(Pos::from(Ticks(1234))), Ticks(1234));
}

#[test]
fn three_four_changes_the_bar_length() {
    let s = Score::new(SampleRate(48_000), Ppq(960)).time_signature(3, 4);
    assert_eq!(s.resolve(bar(2)).unwrap(), Ticks(2880));
    let err = s.resolve(bar(1).beat(4)).unwrap_err();
    assert!(
        matches!(err, ScoreError::BeatOutOfSignature { .. }),
        "{err}"
    );
}

#[test]
fn tempo_120_bpm_puts_a_quarter_at_half_a_second() {
    let map = score().tempo_map();
    assert_eq!(map.sample_at(Ticks(0)), 0);
    assert_eq!(map.sample_at(Ticks(960)), 24_000); // 0.5 s at 48 kHz
    assert_eq!(map.sample_at(Ticks(3840)), 96_000); // one 4/4 bar = 2 s
    assert_eq!(map.ns_at(Ticks(960)), 500_000_000);
    assert!((map.ms_at(Ticks(960)) - 500.0).abs() < 1e-9);
}

#[test]
fn a_tempo_step_change_re_anchors_exactly() {
    // 120 BPM for one bar, then 60 BPM: bar 2 starts at 2 s, and each
    // quarter after it lasts a full second.
    let s = score().tempo(bar(2), Bpm(60.0));
    let map = s.tempo_map();
    assert_eq!(map.sample_at(Ticks(3840)), 96_000);
    assert_eq!(map.sample_at(Ticks(3840 + 960)), 96_000 + 48_000);
    assert_eq!(map.npq_at(Ticks(0)), 500_000_000);
    assert_eq!(map.npq_at(Ticks(3840)), 1_000_000_000);
}

#[test]
fn tick_at_inverts_sample_at_within_a_tick() {
    let s = score().tempo(bar(3), Bpm(97.3));
    let map = s.tempo_map();
    for t in [0u64, 1, 959, 960, 3839, 3840, 7680, 100_000, 1_000_003] {
        let sample = map.sample_at(Ticks(t));
        let back = map.tick_at(sample).0;
        assert!(
            back.abs_diff(t) <= 1,
            "tick {t} -> sample {sample} -> tick {back}"
        );
    }
}

#[test]
fn fractional_bpm_is_exact_rational_after_one_authoring_rounding() {
    // 121.333 BPM -> npq = round(60e9 / 121.333) = 494_506_853 ns exactly;
    // ticks then convert with zero accumulation.
    let map = score().tempo(Ticks(0), Bpm(121.333)).tempo_map();
    assert_eq!(map.npq_at(Ticks(0)), 494_506_853);
    let billion_ticks = Ticks(1_000_000_000);
    // Direct u128 reference over the full span: round(t * npq * sr / (ppq * 1e9)).
    let num: u128 = 1_000_000_000u128 * 494_506_853 * 48_000;
    let den: u128 = 960u128 * 1_000_000_000;
    let expected = ((num + den / 2) / den) as u64;
    assert_eq!(map.sample_at(billion_ticks), expected);
}

#[test]
fn out_of_range_scores_are_rejected() {
    assert!(Score::try_new(SampleRate(7_999), Ppq(960)).is_err());
    assert!(Score::try_new(SampleRate(48_000), Ppq(23)).is_err());
    assert!(
        Score::try_new(SampleRate(48_000), Ppq(960))
            .unwrap()
            .try_tempo(Ticks(0), Bpm(0.5))
            .is_err()
    );
    assert!(
        Score::try_new(SampleRate(48_000), Ppq(960))
            .unwrap()
            .try_tempo(Ticks(0), Bpm(4001.0))
            .is_err()
    );
}

#[test]
fn zero_velocity_and_zero_duration_notes_are_authoring_errors() {
    let s = || {
        Score::try_new(SampleRate(48_000), Ppq(960))
            .unwrap()
            .try_track("t", Instrument::preset("sine"))
            .unwrap()
    };
    assert!(matches!(
        s().try_note("t", bar(1), Dur::quarter(), Pitch::A4, Vel(0)),
        Err(ScoreError::ZeroVelocity)
    ));
    assert!(matches!(
        s().try_note("t", bar(1), Dur::ticks(0), Pitch::A4, Vel(96)),
        Err(ScoreError::ZeroDuration)
    ));
}

#[test]
fn pitch_names_parse_and_display_round_trip() {
    assert_eq!("A4".parse::<Pitch>().unwrap(), Pitch::A4);
    assert_eq!(Pitch::A4.0, 69);
    assert_eq!("C4".parse::<Pitch>().unwrap().0, 60);
    assert_eq!("C#3".parse::<Pitch>().unwrap(), Pitch::CS3);
    assert_eq!("Db3".parse::<Pitch>().unwrap(), Pitch::CS3);
    assert_eq!("G-1".parse::<Pitch>().unwrap().0, 7);
    assert!("H4".parse::<Pitch>().is_err());
    assert!("C99".parse::<Pitch>().is_err());
    for n in 0..=127u8 {
        let p = Pitch(n);
        assert_eq!(p.to_string().parse::<Pitch>().unwrap(), p);
    }
    assert!((Pitch::A4.hz() - 440.0).abs() < 1e-12);
    assert!((Pitch::A5.hz() - 880.0).abs() < 1e-12);
}

#[test]
fn automation_samples_hold_ease_and_interpolate() {
    let s = score().track("t", Instrument::preset("sine")).automate(
        "t",
        Param::CUTOFF_HZ,
        keys![
            (bar(1), 400.0, ease_in_out()),
            (bar(2), 4_000.0),
            (bar(3), 1_000.0, hold()),
            (bar(4), 2_000.0),
        ],
    );
    let auto = &s.tracks()[0].automation[0];
    assert_eq!(auto.value_at(Ticks(0)), 400.0); // on the key
    assert_eq!(auto.value_at(Ticks(3840)), 4_000.0);
    let mid = auto.value_at(Ticks(1920)); // ease-in-out midpoint = halfway
    assert!((mid - 2_200.0).abs() < 1.0, "{mid}");
    // hold: constant until the next key snaps
    assert_eq!(auto.value_at(Ticks(3840 * 2 + 1_000)), 1_000.0);
    assert_eq!(auto.value_at(Ticks(3840 * 3)), 2_000.0);
    // beyond the last key: hold the boundary
    assert_eq!(auto.value_at(Ticks(100_000)), 2_000.0);
}

#[test]
fn duplicate_automation_keys_on_one_tick_are_rejected() {
    let err = Score::try_new(SampleRate(48_000), Ppq(960))
        .unwrap()
        .try_track("t", Instrument::preset("sine"))
        .unwrap()
        .try_automate("t", Param::GAIN, keys![(bar(1), 1.0), (bar(1), 0.5)])
        .unwrap_err();
    assert!(matches!(err, ScoreError::DuplicateKeyTick { .. }), "{err}");
}

#[test]
fn spring_automation_is_rejected_by_validation() {
    let s = score().track("t", Instrument::preset("sine")).automate(
        "t",
        Param::CUTOFF_HZ,
        keys![(bar(1), 400.0, spring(170.0, 26.0)), (bar(2), 800.0)],
    );
    let findings = s.validate_standalone();
    assert!(
        findings
            .iter()
            .any(|f| f.code == "spring-ease" && f.severity == Severity::Error),
        "{findings:?}"
    );
}

#[test]
fn end_tick_covers_note_ends_and_automation_keys() {
    let s = score()
        .track("t", Instrument::preset("sine"))
        .note("t", bar(1), Dur::whole(), Pitch::A4, Vel(96))
        .automate("t", Param::GAIN, keys![(bar(3), 1.0)]);
    assert_eq!(s.end_tick(), Ticks(2 * 3840)); // the bar-3 key
}
