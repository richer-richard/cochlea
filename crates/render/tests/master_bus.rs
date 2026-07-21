//! The master-bus stage: default is byte-inert, gain is exact, and the
//! limiter's sample-peak ceiling is a hard guarantee, not a tendency.

use cochlea_render::render;
use cochlea_score::*;

/// A deliberately hot two-track score: enough coincident energy to push
/// sample peaks near/over any reasonable ceiling once gain is applied.
fn hot_score() -> Score {
    Score::new(SampleRate(48_000), Ppq(960))
        .tempo(Ticks(0), Bpm(120.0))
        .track("a", Instrument::preset("saw_lead"))
        .note("a", bar(1), Dur::whole(), Pitch::A4, Vel(127))
        .track("b", Instrument::preset("square_bass"))
        .note("b", bar(1), Dur::whole(), Pitch::A2, Vel(127))
        .track("c", Instrument::preset("sine"))
        .note("c", bar(1), Dur::whole(), Pitch::A3, Vel(127))
}

fn sample_peak(mix: &[f32]) -> f32 {
    mix.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
}

fn db_to_lin(db: f32) -> f32 {
    libm::powf(10.0, db / 20.0)
}

#[test]
fn default_master_renders_byte_identically_to_no_master() {
    let plain = render(&hot_score()).unwrap();
    let with_default = render(&hot_score().with_master(Master::new())).unwrap();
    assert_eq!(plain.mix(), with_default.mix());
}

#[test]
fn master_gain_scales_the_mix() {
    let plain = render(&hot_score()).unwrap();
    let gained = render(&hot_score().with_master(Master::new().gain_db(-6.0))).unwrap();
    // The bus rounds exactly once: f64 stem sum -> gain -> f32. Rebuild
    // that reference from the (pre-master, identical) stems rather than
    // from the already-rounded plain mix, which would double-round and
    // miss by an ulp.
    let factor = libm::pow(10.0, f64::from(-6.0f32) / 20.0);
    let stems: Vec<&[f32]> = gained.stems().map(|(_, s)| s).collect();
    for (i, &g) in gained.mix().iter().enumerate() {
        let acc: f64 = stems.iter().map(|s| f64::from(s[i])).sum();
        #[expect(clippy::cast_possible_truncation, reason = "test-side reference math")]
        let expected = (acc * factor) as f32;
        assert_eq!(g, expected, "sample {i}");
    }
    // Stems are pre-master: identical between the two renders.
    assert_eq!(plain.stem("a").unwrap(), gained.stem("a").unwrap());
}

#[test]
fn limiter_holds_the_sample_peak_ceiling_exactly() {
    let ceiling_db = -6.0f32;
    let score = hot_score().with_master(
        Master::new()
            .gain_db(12.0)
            .limiter(Limiter::new(ceiling_db)),
    );
    let rendered = render(&score).unwrap();
    let ceiling = db_to_lin(ceiling_db);

    let peak = sample_peak(rendered.mix());
    assert!(
        peak <= ceiling * 1.000_001,
        "sample peak {peak} exceeded ceiling {ceiling}"
    );
    // And the limiter actually worked for its living: the un-limited
    // version of the same gained score does exceed the ceiling.
    let unlimited = render(&hot_score().with_master(Master::new().gain_db(12.0))).unwrap();
    assert!(
        sample_peak(unlimited.mix()) > ceiling,
        "fixture too quiet to exercise the limiter"
    );
    // Limiting is not silencing: the render still has substance.
    assert!(peak > ceiling * 0.5, "peak {peak} suspiciously low");
}

#[test]
fn master_round_trips_through_ron() {
    let score = hot_score().with_master(
        Master::new()
            .gain_db(3.5)
            .limiter(Limiter::new(-1.0).lookahead_ms(8.0).release_ms(120.0)),
    );
    let text = score.to_ron().unwrap();
    assert!(text.contains("master"), "{text}");
    let back = Score::from_ron(&text).unwrap();
    assert_eq!(back.master(), score.master());

    // A master-less score serializes without a master section at all.
    let plain_text = hot_score().to_ron().unwrap();
    assert!(!plain_text.contains("master"), "{plain_text}");

    // And the RON loader validates ranges through the same try_ path.
    let bad = plain_text.replace("tracks: [", "master: Master(gain_db: 99.0),\n    tracks: [");
    assert!(
        Score::from_ron(&bad).is_err(),
        "gain_db 99 must be rejected"
    );
}

#[test]
fn limiter_parameters_are_range_checked() {
    assert!(Limiter::try_new(1.0).is_err(), "positive ceiling");
    assert!(Limiter::try_new(-100.0).is_err(), "absurd ceiling");
    assert!(Limiter::try_new(-1.0).is_ok());
    assert!(Limiter::new(-1.0).try_lookahead_ms(60.0).is_err());
    assert!(Limiter::new(-1.0).try_release_ms(0.0).is_err());
    assert!(Master::new().try_gain_db(f32::NAN).is_err());
}
