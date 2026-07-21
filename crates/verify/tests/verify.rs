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

// --- tempo_is / has_clear_rhythm ----------------------------------------

/// A steady `pluck` hit on every beat, `bars` bars of 4/4 at `bpm` —
/// enough regular attacks for `estimate_tempo` to lock onto a real tempo.
/// `pluck`'s Karplus-Strong excitation reads a much sharper onset than
/// `noise_hat`'s highpass-noise burst (measured: ~0.12 confidence vs
/// ~0.03 at the same tempo/duration — `noise_hat`'s energy sits almost
/// entirely above 7 kHz, which the onset detector's spectral flux weights
/// less sharply than `pluck`'s broadband pluck transient), so this is the
/// preset this workspace's own click-track calibration (`~0.11-0.15`
/// confidence, see `cochlea_features::tempo`'s docs) actually matches.
fn metronome_score(bpm: f64, bars: u32) -> Score {
    let mut score = Score::new(SampleRate(48_000), Ppq(960))
        .tempo(Ticks(0), Bpm(bpm))
        .track("hat", Instrument::preset("pluck"));
    for b in 1..=bars {
        for beat in 1..=4 {
            score = score.note(
                "hat",
                bar(b).beat(beat),
                Dur::sixteenth(),
                Pitch::A4,
                Vel(127),
            );
        }
    }
    score
}

#[test]
fn tempo_is_and_has_clear_rhythm_pass_a_steady_metronome_and_tempo_is_fails_off_target() {
    // 8 bars at 120 BPM = 16 s, comfortably past the ~12 s this workspace's
    // own tempo-detector tests calibrate a confident lock against.
    let score = metronome_score(120.0, 8);
    let rendered = render(&score).unwrap();

    let pass = rendered
        .verify(&score)
        .tempo_is(120.0, BpmTol(2.0))
        .has_clear_rhythm(true)
        .run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "tempo_is");
    assert_eq!(pass.checks[1].kind, "has_clear_rhythm");

    let fail = rendered.verify(&score).tempo_is(90.0, BpmTol(2.0)).run();
    assert!(!fail.passed, "{:?}", fail.checks);
}

#[test]
fn has_clear_rhythm_expected_false_passes_and_expected_true_fails_for_a_sustained_tone() {
    // A held sine has no attacks at all, hence no periodicity worth
    // calling a rhythm — same fixture shape as `cochlea_features`' own
    // `steady_tone_has_no_clear_rhythm` test.
    let score = sine_score();
    let rendered = render(&score).unwrap();

    let pass = rendered.verify(&score).has_clear_rhythm(false).run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "has_clear_rhythm");

    let fail = rendered.verify(&score).has_clear_rhythm(true).run();
    assert!(!fail.passed, "{:?}", fail.checks);
}

// --- stereo_width_within --------------------------------------------------

#[test]
fn stereo_width_within_passes_a_narrow_range_and_fails_a_disjoint_one_for_a_dry_mono_panned_voice()
{
    // No insert: a bare mono-panned voice's dry output is bit-identical
    // on both channels, so this has ~zero stereo width to test against.
    let score = sine_score();
    let rendered = render(&score).unwrap();

    let pass = rendered.verify(&score).stereo_width_within(0.0, 0.01).run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "stereo_width_within");

    let fail = rendered.verify(&score).stereo_width_within(0.5, 1.0).run();
    assert!(!fail.passed, "{:?}", fail.checks);
}

// --- lra_below -------------------------------------------------------------

/// A quiet passage then a loud passage, long enough per section for EBU
/// R128's gated-block history to register both levels distinctly (mirrors
/// `cochlea_features`' own LRA calibration, ~12 s/13 s sections). Velocity
/// maps to amplitude as `(vel/127)^2`, so `Vel(20)` vs `Vel(120)` is a
/// large, unambiguous level step.
fn stepped_loudness_score() -> Score {
    let mut score = Score::new(SampleRate(48_000), Ppq(960))
        .tempo(Ticks(0), Bpm(120.0))
        .track("lead", Instrument::preset("sine"));
    for b in 1..=6 {
        score = score.note("lead", bar(b), Dur::whole(), Pitch::A4, Vel(20));
    }
    for b in 7..=12 {
        score = score.note("lead", bar(b), Dur::whole(), Pitch::A4, Vel(120));
    }
    score
}

#[test]
fn lra_below_passes_a_generous_bound_and_fails_a_strict_one_for_stepped_loudness() {
    let score = stepped_loudness_score();
    let rendered = render(&score).unwrap();

    let pass = rendered.verify(&score).lra_below(30.0).run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "lra_below");

    let fail = rendered.verify(&score).lra_below(1.0).run();
    assert!(!fail.passed, "{:?}", fail.checks);
}

// --- section_count ---------------------------------------------------------

#[test]
fn section_count_reads_one_section_for_a_short_uniform_render() {
    // `detect_structure` needs at least 2*KERNEL_HALF_WIDTH frames (16 s
    // at the default 1 s frame) to look for boundaries at all; anything
    // shorter — like this ordinary short render — always reads as exactly
    // one section.
    let score = sine_score();
    let rendered = render(&score).unwrap();

    let pass = rendered.verify(&score).section_count(1, 1).run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "section_count");

    let fail = rendered.verify(&score).section_count(2, 12).run();
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
fn a_score_ron_string_embedding_all_four_wave2_specs_parses_and_evaluates() {
    // A literal RON `verify:` block, the same text form an agent would
    // author or a `score.ron` file would commit — not a `VerifySpec`
    // constructed directly in Rust — to prove the full pipeline: RON text
    // -> `Score::from_ron` -> render -> `Verifier::with_specs`.
    let text = r#"Score(
        version: 1,
        sample_rate: 48000,
        ppq: 960,
        tempo: [(tick: 0, bpm: 120.0)],
        tracks: [
            Track(
                name: "hat",
                instrument: Preset("pluck"),
                notes: [
                    Note(at: (1, 1), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (1, 2), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (1, 3), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (1, 4), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (2, 1), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (2, 2), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (2, 3), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (2, 4), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (3, 1), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (3, 2), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (3, 3), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (3, 4), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (4, 1), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (4, 2), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (4, 3), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (4, 4), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (5, 1), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (5, 2), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (5, 3), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (5, 4), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (6, 1), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (6, 2), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (6, 3), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (6, 4), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (7, 1), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (7, 2), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (7, 3), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (7, 4), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (8, 1), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (8, 2), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (8, 3), dur: "1/16", pitch: "A4", vel: 127),
                    Note(at: (8, 4), dur: "1/16", pitch: "A4", vel: 127),
                ],
            ),
        ],
        verify: [
            TempoIs(bpm: 120.0, tol_bpm: 2.0),
            HasClearRhythm(expected: true),
            StereoWidthWithin(min: 0.0, max: 1.0),
            LraBelow(lu: 30.0),
        ],
    )"#;

    let score = Score::from_ron(text).expect("valid RON with all four new verify specs");
    assert_eq!(score.verify_specs().len(), 4);

    let rendered = render(&score).unwrap();
    let report = rendered
        .verify(&score)
        .with_specs(score.verify_specs())
        .run();
    assert!(report.passed, "{:?}", report.checks);
    assert_eq!(
        report.checks.iter().map(|c| c.kind).collect::<Vec<_>>(),
        [
            "tempo_is",
            "has_clear_rhythm",
            "stereo_width_within",
            "lra_below"
        ]
    );
}

#[test]
fn wave2_specs_data_form_mirrors_typed_builder_results() {
    let score = metronome_score(120.0, 8);
    let rendered = render(&score).unwrap();

    let typed = rendered
        .verify(&score)
        .tempo_is(120.0, BpmTol(2.0))
        .has_clear_rhythm(true)
        .stereo_width_within(0.0, 1.0)
        .lra_below(30.0)
        .section_count(1, 12)
        .run();
    assert!(typed.passed, "{:?}", typed.checks);

    let specs = vec![
        VerifySpec::TempoIs {
            min_bpm: None,
            max_bpm: None,
            bpm: 120.0,
            tol_bpm: 2.0,
        },
        VerifySpec::HasClearRhythm { expected: true },
        VerifySpec::StereoWidthWithin { min: 0.0, max: 1.0 },
        VerifySpec::LraBelow { lu: 30.0 },
        VerifySpec::SectionCount { min: 1, max: 12 },
    ];
    let data_form = rendered.verify(&score).with_specs(&specs).run();
    assert!(data_form.passed, "{:?}", data_form.checks);
    assert_eq!(typed.checks.len(), data_form.checks.len());
    for (t, d) in typed.checks.iter().zip(data_form.checks.iter()) {
        assert_eq!(t.kind, d.kind);
        assert_eq!(t.passed, d.passed);
    }
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

/// The TempoIs search-range override must actually reach the detector:
/// forcing 30..=70 BPM on a true-120 BPM metronome locks onto the genuine
/// 60 BPM subharmonic (clicks align at every two-beat lag) — measured
/// first via the features API, then asserted through both the typed
/// builder and the RON spec form. The default range keeps finding ~120,
/// so the same assertion fails there. (A range with *no* genuine
/// periodicity in it — e.g. 200..=300 on this metronome — now honestly
/// reports no measurable tempo at all: mean-removed autocovariance is
/// negative at unarticulated lags, where the old raw autocorrelation
/// manufactured a tiny positive peak.)
#[test]
fn tempo_is_range_override_reaches_the_detector() {
    use cochlea_features::{Audio, TempoOpts, estimate_tempo};

    let score = metronome_score(120.0, 8);
    let rendered = render(&score).unwrap();
    let audio = Audio {
        samples: rendered.mix().to_vec(),
        channels: 2,
        sample_rate: rendered.sample_rate().0,
    };

    let forced_report = estimate_tempo(
        &audio,
        &TempoOpts::default().with_min_bpm(30.0).with_max_bpm(70.0),
    );
    let forced_bpm = forced_report
        .bpm
        .expect("the two-beat subharmonic is genuine periodicity");
    assert!(
        (55.0..=65.0).contains(&forced_bpm),
        "the forced range must lock the estimate onto the subharmonic: {forced_bpm}"
    );

    let no_periodicity = estimate_tempo(
        &audio,
        &TempoOpts::default().with_min_bpm(200.0).with_max_bpm(300.0),
    );
    assert_eq!(
        no_periodicity.bpm, None,
        "a range with no articulated periodicity should read as no tempo: {no_periodicity:?}"
    );

    let via_builder = rendered
        .verify(&score)
        .tempo_is_in_range(forced_bpm, BpmTol(1.0), 30.0, 70.0)
        .run();
    assert!(via_builder.passed, "{via_builder:?}");

    let default_range = rendered
        .verify(&score)
        .tempo_is(forced_bpm, BpmTol(1.0))
        .run();
    assert!(
        !default_range.passed,
        "the default range finds ~120 BPM, not {forced_bpm}: {default_range:?}"
    );

    let spec_form = rendered
        .verify(&score)
        .with_spec(&VerifySpec::TempoIs {
            bpm: forced_bpm,
            tol_bpm: 1.0,
            min_bpm: Some(30.0),
            max_bpm: Some(70.0),
        })
        .run();
    assert!(
        spec_form.passed,
        "data form must thread the range: {spec_form:?}"
    );
}

// --- rhythm: grid_alignment_at_least ------------------------------------

/// A metronome's clicks sit exactly on the beat grid — alignment reads at
/// (or extremely near) 1.0 — while an empty render has no grid at all,
/// which per the undefined-metric policy fails a value assertion. (Note a
/// mere sustained-note score is NOT gridless: its note attacks are onsets
/// and get a grid of their own.)
#[test]
fn grid_alignment_passes_a_metronome_and_fails_gridless_material() {
    let score = metronome_score(120.0, 8);
    let rendered = render(&score).unwrap();
    let pass = rendered.verify(&score).grid_alignment_at_least(0.9).run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "grid_alignment_at_least");

    let empty = Score::new(SampleRate(48_000), Ppq(960)).track("lead", Instrument::preset("sine"));
    let rendered_empty = render(&empty).unwrap();
    let fail = rendered_empty
        .verify(&empty)
        .grid_alignment_at_least(0.5)
        .run();
    assert!(!fail.passed, "{:?}", fail.checks);
    assert!(
        fail.checks[0].actual.contains("no usable beat grid"),
        "{:?}",
        fail.checks
    );
}

// --- brightness: the output-side sweep check ----------------------------

/// A two-bar saw pad under a rising cutoff sweep must *audibly* brighten:
/// the same fixture with a flat cutoff must not pass the same assertion.
/// This is the render-side half of the sweep story — `monotone` validates
/// the authored curve, this listens to the stem.
#[test]
fn brightness_rises_hears_a_real_sweep_and_rejects_a_flat_patch() {
    let swept = Score::new(SampleRate(48_000), Ppq(960))
        .track("lead", Instrument::preset("saw_lead"))
        .note("lead", bar(1), Dur::whole(), Pitch::A3, Vel(100))
        .note("lead", bar(2), Dur::whole(), Pitch::A3, Vel(100))
        .automate(
            "lead",
            Param::CUTOFF_HZ,
            keys![(bar(1), 300.0), (bar(3), 6_000.0)],
        );
    let rendered = render(&swept).unwrap();
    let pass = rendered
        .verify(&swept)
        .brightness_rises("lead", bar(1)..bar(3), 1.3)
        .run();
    assert!(pass.passed, "{:?}", pass.checks);
    assert_eq!(pass.checks[0].kind, "brightness_rises");
    // And the mirrored direction must fail on the same render.
    let wrong_way = rendered
        .verify(&swept)
        .brightness_falls("lead", bar(1)..bar(3), 1.3)
        .run();
    assert!(!wrong_way.passed, "{:?}", wrong_way.checks);

    let flat = Score::new(SampleRate(48_000), Ppq(960))
        .track("lead", Instrument::preset("saw_lead"))
        .note("lead", bar(1), Dur::whole(), Pitch::A3, Vel(100))
        .note("lead", bar(2), Dur::whole(), Pitch::A3, Vel(100))
        .automate(
            "lead",
            Param::CUTOFF_HZ,
            keys![(bar(1), 1_200.0), (bar(3), 1_200.0)],
        );
    let rendered_flat = render(&flat).unwrap();
    let fail = rendered_flat
        .verify(&flat)
        .brightness_rises("lead", bar(1)..bar(3), 1.3)
        .run();
    assert!(!fail.passed, "{:?}", fail.checks);
}

/// An unknown track fails gracefully (never panics) and a falling sweep
/// passes the falling form through the RON spec path — proving the new
/// specs survive the full text -> Score -> Verifier pipeline.
#[test]
fn brightness_and_grid_alignment_specs_round_trip_through_ron() {
    let text = r#"Score(
        version: 1,
        sample_rate: 48000,
        ppq: 960,
        tempo: [(tick: 0, bpm: 120.0)],
        tracks: [
            Track(
                name: "lead",
                instrument: Preset("saw_lead"),
                notes: [
                    Note(at: (1, 1), dur: "1/1", pitch: "A3", vel: 100),
                    Note(at: (2, 1), dur: "1/1", pitch: "A3", vel: 100),
                ],
                automation: [
                    Auto(param: "cutoff_hz", keys: [
                        Key(at: (1, 1), value: 6000.0),
                        Key(at: (3, 1), value: 300.0),
                    ]),
                ],
            ),
        ],
        verify: [
            BrightnessFalls(track: "lead", from: (1, 1), to: (3, 1), min_ratio: 1.3),
        ],
    )"#;
    let score = Score::from_ron(text).expect("valid RON with a BrightnessFalls spec");
    let rendered = render(&score).unwrap();
    let report = rendered
        .verify(&score)
        .with_specs(score.verify_specs())
        .run();
    assert!(report.passed, "{report:?}");

    let unknown = rendered
        .verify(&score)
        .brightness_rises("nope", bar(1)..bar(3), 1.2)
        .run();
    assert!(!unknown.passed);
    assert!(
        unknown.checks[0].actual.contains("unknown track") || unknown.checks[0].detail.is_some(),
        "{:?}",
        unknown.checks
    );
}
