//! Integration tests for `cochlea_verify`: build small scores with the
//! render crate's shipped presets, render them, and exercise every
//! `Verifier` check (and its `VerifySpec` data-form mirror) on both sides
//! of its pass/fail boundary.

use cochlea_render::{Rendered, render};
use cochlea_score::*;
use cochlea_verify::*;

/// Probes a render's mix with default options — used to pull a "measured"
/// value out of a render so a test can assert against reality instead of a
/// hand-computed expectation.
fn probe_mix(rendered: &Rendered) -> cochlea_features::Report {
    let audio = cochlea_features::Audio {
        samples: rendered.mix().to_vec(),
        channels: 2,
        sample_rate: rendered.sample_rate().0,
    };
    cochlea_features::probe(&audio, &cochlea_features::ProbeOpts::default())
}

/// Two bars of a sustained sine at `pitch` — long enough for a stable
/// integrated-loudness reading and a stable YIN estimate.
fn note_score(pitch: Pitch) -> Score {
    Score::new(SampleRate(48_000), Ppq(960))
        .track("lead", Instrument::preset("sine"))
        .note("lead", bar(1), Dur::whole(), pitch, Vel(100))
        .note("lead", bar(2), Dur::whole(), pitch, Vel(100))
}

fn sine_score() -> Score {
    note_score(Pitch::A4)
}

/// A single `noise_hat` hit on beat 2 of bar 1, in an otherwise silent
/// buffer — the onset detector's fixtures elsewhere in this workspace note
/// that some silent lead-in makes the first onset's detected time more
/// reliable.
fn hat_score() -> Score {
    Score::new(SampleRate(48_000), Ppq(960))
        .track("hat", Instrument::preset("noise_hat"))
        .note("hat", bar(1).beat(2), Dur::sixteenth(), Pitch::A4, Vel(110))
}

/// A whole-note `saw_lead` with a `CUTOFF_HZ` automation curve — `keys`
/// controls the shape so tests can build both a clean sweep and a dipped
/// one.
fn cutoff_score(keys: Vec<KeyDef>) -> Score {
    Score::new(SampleRate(48_000), Ppq(960))
        .track("lead", Instrument::preset("saw_lead"))
        .note("lead", bar(1), Dur::whole(), Pitch::A4, Vel(100))
        .automate("lead", Param::CUTOFF_HZ, keys)
}

// --- integrated_lufs ---------------------------------------------------

#[test]
fn integrated_lufs_passes_at_measured_value_and_fails_at_an_absurd_target() {
    let score = sine_score();
    let rendered = render(&score).unwrap();
    let measured = probe_mix(&rendered)
        .loudness
        .integrated_lufs
        .expect("an audible sustained sine has gated loudness");

    let pass = rendered
        .verify(&score)
        .integrated_lufs(measured, Tol(0.5))
        .run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "integrated_lufs");

    let fail = rendered
        .verify(&score)
        .integrated_lufs(measured - 40.0, Tol(0.5))
        .run();
    assert!(!fail.passed, "{:?}", fail.checks);
    let c = &fail.checks[0];
    assert!(!c.expected.is_empty());
    assert!(!c.actual.is_empty());
}

#[test]
fn integrated_lufs_fails_on_silence_with_detail() {
    // A track with no notes renders to a zero-length stem/mix, and
    // `cochlea_features::probe` reports undefined (`None`) loudness for
    // that, same as it does for true silence.
    let score = Score::new(SampleRate(48_000), Ppq(960)).track("lead", Instrument::preset("sine"));
    let rendered = render(&score).unwrap();

    let report = rendered
        .verify(&score)
        .integrated_lufs(-14.0, Tol(0.5))
        .run();
    assert!(!report.passed);
    assert_eq!(report.checks[0].kind, "integrated_lufs");
    assert!(
        report.checks[0]
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("silence"),
        "{:?}",
        report.checks[0]
    );
}

// --- true_peak_below -----------------------------------------------------

#[test]
fn true_peak_below_passes_and_fails_around_the_measured_peak() {
    let score = sine_score();
    let rendered = render(&score).unwrap();
    let measured = probe_mix(&rendered)
        .loudness
        .true_peak_dbtp
        .expect("an audible sine has a measurable true peak");

    let pass = rendered
        .verify(&score)
        .true_peak_below(measured + 1.0)
        .run();
    assert!(pass.passed, "{:?}", pass.checks);

    let fail = rendered
        .verify(&score)
        .true_peak_below(measured - 1.0)
        .run();
    assert!(!fail.passed, "{:?}", fail.checks);
}

// --- onset_at --------------------------------------------------------------

#[test]
fn onset_at_detects_the_hat_within_hop_and_attack_tolerance() {
    let score = hat_score();
    let rendered = render(&score).unwrap();

    // 30 ms covers the onset detector's ~5.33 ms STFT hop resolution
    // (256-sample hop @ 48 kHz) plus the noise_hat patch's 2 ms attack and
    // the adaptive threshold's lag before the transient clears it.
    let report = rendered
        .verify(&score)
        .onset_at("hat", bar(1).beat(2), Ms(30.0))
        .run();
    assert!(report.passed, "{:?}", report.checks);
    assert_eq!(report.checks[0].kind, "onset_at");
}

// --- pitch_matches_score -----------------------------------------------

#[test]
fn pitch_matches_score_passes_on_pitch_and_fails_when_scored_a_semitone_off() {
    let score_a4 = note_score(Pitch::A4);
    let rendered_a4 = render(&score_a4).unwrap();

    let pass = rendered_a4
        .verify(&score_a4)
        .pitch_matches_score("lead", Cents(10.0))
        .run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "pitch_matches_score");

    // Same A4 render, verified against a score that scores the note as B4
    // instead — ~200 cents off, well outside tolerance.
    let score_b4 = note_score(Pitch::B4);
    let fail = rendered_a4
        .verify(&score_b4)
        .pitch_matches_score("lead", Cents(10.0))
        .run();
    assert!(!fail.passed, "{:?}", fail.checks);
}

// --- monotone ------------------------------------------------------------

#[test]
fn monotone_passes_a_rising_sweep_and_fails_a_mid_range_dip() {
    let rising = cutoff_score(keys![(bar(1), 400.0), (bar(3), 4_000.0)]);
    let rendered = render(&rising).unwrap();
    let pass = rendered
        .verify(&rising)
        .monotone("lead", Param::CUTOFF_HZ, bar(1)..bar(3))
        .run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "monotone");

    let dipped = cutoff_score(keys![(bar(1), 400.0), (bar(2), 100.0), (bar(3), 4_000.0)]);
    let rendered_dip = render(&dipped).unwrap();
    let fail = rendered_dip
        .verify(&dipped)
        .monotone("lead", Param::CUTOFF_HZ, bar(1)..bar(3))
        .run();
    assert!(!fail.passed, "{:?}", fail.checks);
}

#[test]
fn monotone_fails_with_detail_when_automation_is_missing() {
    let score = cutoff_score(keys![(bar(1), 400.0), (bar(3), 4_000.0)]);
    let rendered = render(&score).unwrap();

    let report = rendered
        .verify(&score)
        .monotone("lead", Param::GAIN, bar(1)..bar(3))
        .run();
    assert!(!report.passed);
    assert!(
        report.checks[0]
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("no automation"),
        "{:?}",
        report.checks[0]
    );
}

// --- no_discontinuity ------------------------------------------------------

#[test]
fn no_discontinuity_passes_a_clean_sine_and_fails_strict_sustained_noise() {
    let sine = note_score(Pitch::A4);
    let rendered = render(&sine).unwrap();
    let pass = rendered
        .verify(&sine)
        .no_discontinuity("lead", Db(20.0))
        .run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "no_discontinuity");

    // noise_hat sustains full-band noise for the length of the note; a
    // threshold this strict (only -90 dBFS of headroom) flags its ordinary
    // sample-to-sample variation, well outside the ±10 ms note-boundary
    // guards.
    let hat = Score::new(SampleRate(48_000), Ppq(960))
        .track("hat", Instrument::preset("noise_hat"))
        .note("hat", bar(1), Dur::half(), Pitch::A4, Vel(110));
    let rendered_hat = render(&hat).unwrap();
    let fail = rendered_hat
        .verify(&hat)
        .no_discontinuity("hat", Db(90.0))
        .run();
    assert!(!fail.passed, "{:?}", fail.checks);
}

// --- silent_after ------------------------------------------------------

#[test]
fn silent_after_passes_after_the_tail_and_fails_mid_note() {
    // A reverb insert gives the render a genuine decaying tail (a bare
    // note's render ends essentially exactly when its own envelope hits
    // zero, so there'd be no real silent region to test against
    // otherwise).
    let score = Score::new(SampleRate(48_000), Ppq(960))
        .track("lead", Instrument::preset("sine"))
        .insert("lead", Insert::preset("reverb"))
        .note("lead", bar(1), Dur::quarter(), Pitch::A4, Vel(100));
    let rendered = render(&score).unwrap();

    // Find where the tail actually goes quiet (same -60 dBFS / 50 ms
    // window / 10 ms hop definition `silent_after` uses), empirically,
    // rather than hand-deriving the reverb's decay curve.
    let probed = probe_mix(&rendered);
    let last_audible = probed
        .silence
        .last_audible_sample
        .expect("the note itself is audible");
    let quiet_sample = (last_audible as u64 + 5_000).min(rendered.frames() - 1);
    let quiet_tick = score.tempo_map().tick_at(quiet_sample);

    let pass = rendered.verify(&score).silent_after(quiet_tick).run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "silent_after");

    let fail = rendered.verify(&score).silent_after(bar(1)).run();
    assert!(!fail.passed, "{:?}", fail.checks);
}

// --- unknown tracks: fail, never panic ----------------------------------

#[test]
fn unknown_track_fails_every_track_scoped_check_without_panicking() {
    let score = sine_score();
    let rendered = render(&score).unwrap();

    let report = rendered
        .verify(&score)
        .onset_at("nope", bar(1), Ms(30.0))
        .pitch_matches_score("nope", Cents(10.0))
        .monotone("nope", Param::CUTOFF_HZ, bar(1)..bar(2))
        .no_discontinuity("nope", Db(20.0))
        .run();

    assert!(!report.passed);
    assert_eq!(report.checks.len(), 4);
    for check in &report.checks {
        assert!(!check.passed, "{check:?}");
        assert!(
            check
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("unknown track"),
            "{check:?}"
        );
    }
}

// --- with_spec / with_specs mirror the typed DSL ------------------------

#[test]
fn with_specs_mirrors_the_typed_builder_results() {
    let score = sine_score();
    let rendered = render(&score).unwrap();
    let measured = probe_mix(&rendered);
    let target = measured.loudness.integrated_lufs.unwrap();
    let dbtp = measured.loudness.true_peak_dbtp.unwrap() + 1.0;

    let typed = rendered
        .verify(&score)
        .integrated_lufs(target, Tol(0.5))
        .true_peak_below(dbtp)
        .run();

    let specs = vec![
        VerifySpec::IntegratedLufs { target, tol: 0.5 },
        VerifySpec::TruePeakBelow { dbtp },
    ];
    let data_form = rendered.verify(&score).with_specs(&specs).run();
    assert!(data_form.passed);
    assert_eq!(typed.passed, data_form.passed);
    for (t, d) in typed.checks.iter().zip(data_form.checks.iter()) {
        assert_eq!(t.kind, d.kind);
        assert_eq!(t.passed, d.passed);
    }

    // `with_spec` (singular) queues one at a time, same result as the
    // batch form.
    let one_at_a_time = rendered
        .verify(&score)
        .with_spec(&specs[0])
        .with_spec(&specs[1])
        .run();
    assert_eq!(one_at_a_time.passed, data_form.passed);
}

#[test]
fn with_spec_monotone_direction_overrides_inference() {
    // A falling sweep authored with `direction: Falling` in the data form
    // should be checked *as falling*, regardless of what the typed
    // builder's endpoint-comparison inference would pick.
    let score = cutoff_score(keys![(bar(1), 4_000.0), (bar(3), 400.0)]);
    let rendered = render(&score).unwrap();
    let from = score.resolve(bar(1)).unwrap();
    let to = score.resolve(bar(3)).unwrap();

    let spec = VerifySpec::Monotone {
        track: "lead".to_string(),
        param: Param::CUTOFF_HZ,
        from,
        to,
        direction: MonotoneDir::Falling,
    };
    let report = rendered.verify(&score).with_spec(&spec).run();
    assert!(report.passed, "{:?}", report.checks);
    assert_eq!(report.checks[0].kind, "monotone");
}

// --- report shape --------------------------------------------------------

#[test]
fn verify_report_serializes_with_schema_version_and_stable_kinds() {
    let score = sine_score();
    let rendered = render(&score).unwrap();
    let report = rendered
        .verify(&score)
        .true_peak_below(0.0)
        .integrated_lufs(-1_000.0, Tol(0.1)) // deliberately absurd: forces a failing check with `detail`.
        .run();

    assert!(!report.passed);
    assert_eq!(report.checks.len(), 2);

    let json = serde_json::to_value(&report).expect("VerifyReport should serialize");
    assert_eq!(json["schema_version"].as_u64(), Some(1));
    assert_eq!(json["passed"].as_bool(), Some(false));

    let kinds: Vec<&str> = json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["true_peak_below", "integrated_lufs"]);

    // The passing true_peak_below check has no detail, so serde should
    // have dropped the key entirely (`skip_serializing_if`).
    assert!(json["checks"][0].get("detail").is_none());
    // The failing integrated_lufs check should carry no detail either
    // (undefined loudness is the only detail-producing failure mode for
    // this check, and this mix is definitely audible) — but it must still
    // have non-empty expected/actual text.
    assert!(!json["checks"][1]["expected"].as_str().unwrap().is_empty());
    assert!(!json["checks"][1]["actual"].as_str().unwrap().is_empty());
}
