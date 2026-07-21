//! Integration tests for the tempo/rhythm split: `analyze_rhythm` (grid
//! alignment, offbeat ratio, the grid-based `clear_rhythm`) and the
//! humanized-robustness story — how far detection degrades under the
//! timing jitter, dynamic variation, dropped hits, and extra hits a real
//! performance contains. Fixtures are synthesized here with `libm`, never
//! a synth dependency — mirrors `tests/probe.rs`'s style.

use cochlea_features::{ProbeOpts, TempoOpts, analyze_rhythm, estimate_tempo, probe};

mod common;
use common::*;

/// Onset times for a straight click track: first click one interval in
/// (t=0 onsets bias the detector's earliest frame; see the metronome demo
/// notes), then every `interval_s` until `seconds`.
fn straight_times(interval_s: f64, seconds: f64) -> Vec<f64> {
    let mut times = Vec::new();
    let mut t = interval_s;
    while t < seconds {
        times.push(t);
        t += interval_s;
    }
    times
}

/// Deterministic pseudo-random stream for humanization — SplitMix64-style
/// finalizer over a counter, mapped to `[-1.0, 1.0)`. Test-local: fixture
/// randomness never touches the crate's RNG or a stateful generator.
fn noise(seed: u64, index: u64) -> f64 {
    let mut z = seed
        .wrapping_add(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(index.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z as f64 / u64::MAX as f64) * 2.0 - 1.0
}

/// A 120 BPM click track `seconds` long with uniform timing jitter of up
/// to `jitter_ms` on every hit — the "human drummer" fixture.
fn humanized_click_track(jitter_ms: f64, seconds: f64, seed: u64) -> Vec<f32> {
    let times: Vec<f64> = straight_times(0.5, seconds)
        .iter()
        .enumerate()
        .map(|(i, &t)| t + noise(seed, i as u64) * jitter_ms / 1000.0)
        .collect();
    click_track(&times, seconds, SR)
}

fn rhythm_of(samples: Vec<f32>) -> (cochlea_features::TempoReport, cochlea_features::RhythmReport)
{
    let audio = mono_audio(samples, SR);
    let report = probe(&audio, &ProbeOpts::default());
    let tempo = estimate_tempo(&audio, &TempoOpts::default());
    let rhythm = analyze_rhythm(
        &report.onsets,
        &tempo,
        report.source.duration_ms / 1000.0,
    );
    (tempo, rhythm)
}

/// Prints every fixture's measured tempo/rhythm numbers — the calibration
/// record behind the thresholds in `rhythm.rs` and the robustness table in
/// the book. Run with `--nocapture` to read them; the assertions in the
/// tests below are the durable form.
#[test]
fn calibration_readings() {
    let fixtures: Vec<(&str, Vec<f32>)> = vec![
        ("click 120", click_track(&straight_times(0.5, 12.0), 12.0, SR)),
        ("click 120 jitter ±5ms", humanized_click_track(5.0, 12.0, 7)),
        ("click 120 jitter ±10ms", humanized_click_track(10.0, 12.0, 7)),
        ("click 120 jitter ±20ms", humanized_click_track(20.0, 12.0, 7)),
        ("click 120 jitter ±30ms", humanized_click_track(30.0, 12.0, 7)),
        ("eighths at 120", click_track(&straight_times(0.25, 12.0), 12.0, SR)),
        ("sustained tone", sine_wave(440.0, 0.5, 12.0, SR)),
        (
            "random onsets",
            click_track(
                &(0..24)
                    .map(|i| 0.3 + (noise(11, i) + 1.0) * 5.7)
                    .collect::<Vec<f64>>(),
                12.0,
                SR,
            ),
        ),
    ];
    for (name, samples) in fixtures {
        let (tempo, rhythm) = rhythm_of(samples);
        println!(
            "{name}: bpm={:?} conf={:.3} stability={:?} align={:?} offbeat={:?} clear={}",
            tempo.bpm.map(|b| (b * 10.0).round() / 10.0),
            tempo.confidence,
            tempo.stability,
            rhythm.grid_alignment,
            rhythm.offbeat_ratio,
            rhythm.clear_rhythm,
        );
    }
}

#[test]
fn click_track_is_clear_aligned_and_on_beat() {
    let (tempo, rhythm) = rhythm_of(click_track(&straight_times(0.5, 12.0), 12.0, SR));
    let bpm = tempo.bpm.expect("click track has a tempo");
    assert!((bpm - 120.0).abs() <= 1.0, "bpm = {bpm}");
    assert!(rhythm.clear_rhythm, "{rhythm:?} tempo={tempo:?}");
    let align = rhythm.grid_alignment.expect("grid exists");
    assert!(align >= 0.9, "alignment = {align}");
    let offbeat = rhythm.offbeat_ratio.expect("aligned onsets exist");
    assert!(offbeat <= 0.1, "clicks sit on the beat: offbeat = {offbeat}");
}

/// Straight eighth notes against a 120 BPM grid: every other hit is an
/// off-beat — `offbeat_ratio` is the syncopation signal, and it must read
/// the difference between this and the quarter-note track above.
#[test]
fn eighth_notes_read_as_aligned_but_offbeat() {
    let (tempo, rhythm) = rhythm_of(click_track(&straight_times(0.25, 12.0), 12.0, SR));
    let bpm = tempo.bpm.expect("eighths have a tempo");
    let align = rhythm.grid_alignment.expect("grid exists");
    assert!(align >= 0.9, "alignment = {align}");
    let offbeat = rhythm.offbeat_ratio.expect("aligned onsets exist");
    // If the detector called the eighth-note pulse itself the beat
    // (240 BPM), every hit is on-beat; at the prior-preferred 120 BPM,
    // half the hits are off-beats. Either reading is musically defensible;
    // what matters is internal consistency between bpm and offbeat_ratio.
    if (bpm - 120.0).abs() <= 2.0 {
        assert!(offbeat >= 0.4, "at 120 BPM half the hits are offbeat: {offbeat}");
    } else {
        assert!((bpm - 240.0).abs() <= 4.0, "unexpected bpm {bpm}");
        assert!(offbeat <= 0.1, "at 240 BPM every hit is a beat: {offbeat}");
    }
    assert!(rhythm.clear_rhythm, "{rhythm:?}");
}

#[test]
fn sustained_tone_has_no_clear_rhythm() {
    let (tempo, rhythm) = rhythm_of(sine_wave(440.0, 0.5, 12.0, SR));
    assert!(
        !rhythm.clear_rhythm,
        "a sustained tone has no attacks: {rhythm:?} tempo={tempo:?}"
    );
}

/// Uniformly random onset times: whatever grid the tempo stage hallucinates,
/// most hits must NOT align to it — the geometric floor for this tolerance
/// is ~0.3, and `clear_rhythm` must stay false.
#[test]
fn random_onsets_do_not_align() {
    let times: Vec<f64> = (0..24).map(|i| 0.3 + (noise(11, i) + 1.0) * 5.7).collect();
    let (tempo, rhythm) = rhythm_of(click_track(&times, 12.0, SR));
    if let Some(align) = rhythm.grid_alignment {
        assert!(align < 0.7, "random onsets aligned {align} — tolerance too loose");
    }
    assert!(!rhythm.clear_rhythm, "{rhythm:?} tempo={tempo:?}");
}

/// The robustness contract, measured: a performance with human timing
/// jitter keeps its pulse. Measured behavior (see `calibration_readings`):
/// up to ±10 ms the exact BPM survives (conf 0.96 → 0.51); at ±20–30 ms
/// the estimate octave-folds to the half tempo — the classic metrical
/// ambiguity, since smeared beats make the two-beat lag as clear as the
/// one-beat lag — but a pulse octave-related to the truth is still
/// detected, `grid_alignment` stays 1.0 (the DP grid follows the human
/// timing), and `clear_rhythm` holds throughout.
#[test]
fn human_timing_jitter_degrades_gracefully() {
    for jitter_ms in [5.0, 10.0, 20.0, 30.0] {
        let (tempo, rhythm) = rhythm_of(humanized_click_track(jitter_ms, 12.0, 7));
        let bpm = tempo.bpm.expect("jittered click track still has a pulse");
        let octave_ok = [60.0, 120.0, 240.0]
            .iter()
            .any(|&target| (bpm - target).abs() <= 2.0);
        assert!(
            octave_ok,
            "±{jitter_ms} ms jitter broke the pulse beyond octave folding: {bpm}"
        );
        if jitter_ms <= 10.0 {
            assert!(
                (bpm - 120.0).abs() <= 2.0,
                "±{jitter_ms} ms jitter should keep the exact BPM: {bpm}"
            );
        }
        assert!(
            rhythm.clear_rhythm,
            "±{jitter_ms} ms jitter should still read as a clear rhythm: {rhythm:?}"
        );
    }
}

/// Real performances contain mistakes: a dropped backbeat and a flammed
/// extra hit must not flip the whole read. One dropped + one extra hit in
/// 22 is ~9% of the material — the grid holds.
#[test]
fn dropped_and_extra_hits_do_not_break_the_read() {
    let mut times = straight_times(0.5, 12.0);
    times.remove(9); // a dropped hit mid-phrase
    times.push(5.37); // an extra hit nowhere near the grid
    times.sort_by(f64::total_cmp);
    let (tempo, rhythm) = rhythm_of(click_track(&times, 12.0, SR));
    let bpm = tempo.bpm.expect("still a pulse");
    assert!((bpm - 120.0).abs() <= 2.0, "bpm = {bpm}");
    assert!(rhythm.clear_rhythm, "{rhythm:?}");
}

/// The drum-solo scenario, in miniature: the *pattern* changes halfway
/// (quarters → dense eighths with off-grid ornaments) while the *speed*
/// never does. Tempo stability must stay high — the pulse held — even
/// though the surface rhythm changed completely.
#[test]
fn pattern_change_at_constant_speed_keeps_tempo_stability() {
    let mut times = straight_times(0.5, 6.0);
    let mut t = 6.0;
    while t < 12.0 {
        times.push(t);
        t += 0.25;
    }
    let (tempo, _) = rhythm_of(click_track(&times, 12.0, SR));
    let bpm = tempo.bpm.expect("a pulse exists throughout");
    let stability = tempo.stability.expect("12 s is long enough to window");
    assert!(
        stability >= 0.75,
        "speed never changed (bpm={bpm}); stability must stay high: {stability}"
    );
}

/// An actual speed change (100 → 140 BPM halfway) must *lower* stability —
/// this is the axis that separates "the rhythm got confusing" from "the
/// tempo actually moved".
#[test]
fn real_speed_change_lowers_tempo_stability() {
    let mut times = straight_times(0.6, 6.0);
    let mut t = 6.0;
    while t < 12.0 {
        times.push(t);
        t += 60.0 / 140.0;
    }
    let (tempo, _) = rhythm_of(click_track(&times, 12.0, SR));
    let stability = tempo.stability.expect("12 s is long enough to window");
    assert!(
        stability <= 0.75,
        "a real 100->140 speed change should show up in stability: {stability} ({tempo:?})"
    );
}

#[test]
fn no_tempo_means_no_grid_fields_and_never_panics() {
    let audio = mono_audio(silence(4.0, SR), SR);
    let report = probe(&audio, &ProbeOpts::default());
    assert_eq!(report.rhythm.grid_alignment, None);
    assert_eq!(report.rhythm.offbeat_ratio, None);
    assert!(!report.rhythm.clear_rhythm);
}
