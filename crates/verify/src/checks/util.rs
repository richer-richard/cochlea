//! Shared helpers for the check implementations: the `Audio` adapter over
//! render buffers, linear-to-dB conversion, and `Pos` resolution.

use cochlea_features::Audio;
use cochlea_score::{Pos, Score, Ticks};

/// Wraps an interleaved stereo buffer (a mix or a stem — the only two
/// shapes `cochlea_render::Rendered` ever hands out) as a
/// [`cochlea_features::Audio`] for [`cochlea_features::probe`].
pub(crate) fn stereo_audio(samples: &[f32], sample_rate: u32) -> Audio {
    Audio {
        samples: samples.to_vec(),
        channels: 2,
        sample_rate,
    }
}

/// Linear amplitude to dB (`20*log10`, via `libm` — the determinism
/// contract's transcendentals rule, `docs/determinism.md`). Non-positive
/// input maps to `-inf`, matching `cochlea_features`' convention for
/// undefined/zero levels.
pub(crate) fn lin_to_db(lin: f64) -> f64 {
    if lin > 0.0 {
        20.0 * libm::log10(lin)
    } else {
        f64::NEG_INFINITY
    }
}

/// Resolves a position against the score's grid. Chainable-builder
/// convention (matching `cochlea_score::Score`'s own panicking builders):
/// an invalid `Pos` (e.g. a beat outside the time signature) is an
/// authoring bug at the call site, not a runtime condition a verification
/// report should describe — so this panics with the underlying error
/// rather than threading a `Result` through the whole `Verifier` chain.
/// (Unknown *track names* are the one input `Verifier` never panics on —
/// see [`crate::CheckResult::unknown_track`].)
pub(crate) fn resolve_pos(score: &Score, pos: impl Into<Pos>) -> Ticks {
    score
        .resolve(pos)
        .unwrap_or_else(|e| panic!("cochlea-verify: invalid position: {e}"))
}
