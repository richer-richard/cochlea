//! Property tests for the timebase: monotonicity, zero drift over 1e9
//! ticks, exactness against a direct u128 reference, and inverse bounds.

use cochlea_score::*;
use proptest::prelude::*;

/// Round-to-nearest (ties up) of `t * npq * sr / (ppq * 1e9)`, computed
/// directly over the whole span in u128 — the reference the anchored
/// segment scheme must match exactly within a single tempo segment.
fn reference_samples(t: u64, npq: u64, sr: u32, ppq: u32) -> u64 {
    let num = u128::from(t) * u128::from(npq) * u128::from(sr);
    let den = u128::from(ppq) * 1_000_000_000;
    let (q, r) = (num / den, num % den);
    u64::try_from(if r * 2 >= den { q + 1 } else { q }).unwrap()
}

fn arb_score_params() -> impl Strategy<Value = (u32, u32, f64)> {
    (
        8_000u32..=192_000,
        prop_oneof![Just(96u32), Just(480), Just(960), Just(1920), Just(15_360)],
        1.0f64..=4_000.0,
    )
}

proptest! {
    /// Single tempo segment: the tempo map IS the u128 reference, out to a
    /// billion ticks — no drift, ever.
    #[test]
    fn zero_drift_over_a_billion_ticks((sr, ppq, bpm) in arb_score_params(),
                                       t in 0u64..=1_000_000_000) {
        let s = Score::try_new(SampleRate(sr), Ppq(ppq)).unwrap()
            .try_tempo(Ticks(0), Bpm(bpm)).unwrap();
        let map = s.tempo_map();
        let npq = map.npq_at(Ticks(0));
        prop_assert_eq!(map.sample_at(Ticks(t)), reference_samples(t, npq, sr, ppq));
    }

    /// Tick→sample is monotone non-decreasing, including across tempo
    /// change boundaries.
    #[test]
    fn sample_at_is_monotone((sr, ppq, bpm) in arb_score_params(),
                             bpm2 in 1.0f64..=4_000.0,
                             change_at in 1u64..=1_000_000,
                             t1 in 0u64..=2_000_000,
                             dt in 0u64..=1_000_000) {
        let s = Score::try_new(SampleRate(sr), Ppq(ppq)).unwrap()
            .try_tempo(Ticks(0), Bpm(bpm)).unwrap()
            .try_tempo(Ticks(change_at), Bpm(bpm2)).unwrap();
        let map = s.tempo_map();
        prop_assert!(map.sample_at(Ticks(t1)) <= map.sample_at(Ticks(t1 + dt)));
    }

    /// The inverse conversion lands within one sample's worth of ticks of
    /// the original (sample quantization, not error accumulation).
    #[test]
    fn tick_at_inverts_within_sample_quantization((sr, ppq, bpm) in arb_score_params(),
                                                  t in 0u64..=100_000_000) {
        let s = Score::try_new(SampleRate(sr), Ppq(ppq)).unwrap()
            .try_tempo(Ticks(0), Bpm(bpm)).unwrap();
        let map = s.tempo_map();
        let npq = map.npq_at(Ticks(0));
        // ticks spanned by one sample, rounded up, plus the ±0.5 rounding
        let ticks_per_sample =
            (u128::from(ppq) * 1_000_000_000).div_ceil(u128::from(npq) * u128::from(sr));
        let bound = u64::try_from(ticks_per_sample).unwrap() + 1;
        let back = map.tick_at(map.sample_at(Ticks(t))).0;
        prop_assert!(back.abs_diff(t) <= bound,
            "t={t} back={back} bound={bound}");
    }

    /// tick_at is monotone in the sample argument.
    #[test]
    fn tick_at_is_monotone((sr, ppq, bpm) in arb_score_params(),
                           bpm2 in 1.0f64..=4_000.0,
                           change_at in 1u64..=100_000,
                           s1 in 0u64..=10_000_000,
                           ds in 0u64..=1_000_000) {
        let s = Score::try_new(SampleRate(sr), Ppq(ppq)).unwrap()
            .try_tempo(Ticks(0), Bpm(bpm)).unwrap()
            .try_tempo(Ticks(change_at), Bpm(bpm2)).unwrap();
        let map = s.tempo_map();
        prop_assert!(map.tick_at(s1) <= map.tick_at(s1 + ds));
    }

    /// Grid positions resolve exactly and reconstruct through the data
    /// form's (bar, beat) inverse: resolution is a bijection on the grid.
    #[test]
    fn grid_resolution_is_exact(barn in 1u32..=10_000, beat in 1u32..=4) {
        let s = Score::new(SampleRate(48_000), Ppq(960));
        let t = s.resolve(bar(barn).beat(beat)).unwrap();
        prop_assert_eq!(t.0, u64::from(barn - 1) * 3840 + u64::from(beat - 1) * 960);
    }
}
