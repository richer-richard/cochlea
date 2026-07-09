//! The workspace's only randomness: a counter-based RNG keyed by
//! `(seed, index)`. A pure integer hash — random access, no state, no
//! entropy, identical on every platform (the stochastic-sources law, `docs/plan.md`).

/// SplitMix64-style finalizer over the pair. Distinct seeds give unrelated
/// streams; within a stream, `index` is the sample position.
#[must_use]
pub fn crng(seed: u64, index: u64) -> u64 {
    // Stafford's Mix13 variant of the SplitMix64 finalizer, applied to the
    // golden-ratio-stepped index xored with the stream seed.
    let mut z = seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Uniform in [-1, 1): the top 24 bits of [`crng`] scaled — exact in f32.
#[must_use]
pub fn crng_f32(seed: u64, index: u64) -> f32 {
    let bits = (crng(seed, index) >> 40) as u32; // top 24 bits, exact in f32
    #[expect(clippy::cast_precision_loss, reason = "24-bit value is exact in f32")]
    let unit = bits as f32 / 8_388_608.0; // 2^23 -> [0, 2)
    unit - 1.0
}

/// A stable seed for a note: hashes the track and note indices into one
/// stream id. Pure function of the schedule (invariant 7).
#[must_use]
pub fn note_seed(track_index: u64, note_index: u64) -> u64 {
    crng(crng(0xC0C4_1EA0_0000_0000 ^ track_index, note_index), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crng_reference_vectors_pin_the_algorithm() {
        // Golden values: if these move, every seeded render moves — that is
        // a re-bless event, not a refactor.
        // crng(0, 0) and crng(0, 1) coincide with the canonical SplitMix64
        // stream from seed 0 (inputs mix to 1·φ and 2·φ).
        assert_eq!(crng(0, 0), 0xE220_A839_7B1D_CDAF);
        assert_eq!(crng(0, 1), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(crng(1, 0), 0x910A_2DEC_8902_5CC1);
        // Cross-checked against an independent Python implementation.
        assert_eq!(crng(0xDEAD_BEEF, 12345), 0x3EB3_070E_35BA_25B3);
    }

    #[test]
    fn crng_f32_is_bounded_and_random_access() {
        for i in 0..10_000 {
            let v = crng_f32(42, i);
            assert!((-1.0..1.0).contains(&v), "{v} at {i}");
        }
        // Random access: order of evaluation is irrelevant.
        let forward: Vec<f32> = (0..100).map(|i| crng_f32(7, i)).collect();
        let backward: Vec<f32> = (0..100).rev().map(|i| crng_f32(7, i)).collect();
        assert_eq!(forward, backward.into_iter().rev().collect::<Vec<_>>());
    }

    #[test]
    fn streams_with_different_seeds_are_unrelated() {
        let a: Vec<u64> = (0..64).map(|i| crng(1, i)).collect();
        let b: Vec<u64> = (0..64).map(|i| crng(2, i)).collect();
        assert_ne!(a, b);
    }
}
