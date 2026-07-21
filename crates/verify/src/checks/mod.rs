//! One module per check kind. Every function here is a pure
//! `(rendered, score, ...) -> CheckResult` — no shared mutable state, so
//! [`crate::Verifier`]'s builder methods just compute and queue.
//!
//! **Undefined-metric policy** (the rule every check follows when its
//! metric can't be measured at all — silence, too little audio):
//! *bounded-above* checks **pass** on undefined (`true_peak_below`,
//! `lra_below`: silence has no peak or range to exceed, so the bound
//! holds vacuously), while *value assertions* **fail** on undefined
//! (`integrated_lufs`, `tempo_is`, `pitch_matches_score`: asserting a
//! specific value presupposes there is one — an undefined reading can't
//! satisfy it). A score that legitimately expects no measurable value
//! asserts that directly instead (e.g. `HasClearRhythm(expected: false)`,
//! `SilentAfter`). Each check's doc states which side it's on.

pub(crate) mod brightness;
pub(crate) mod discontinuity;
pub(crate) mod loudness;
pub(crate) mod monotone;
pub(crate) mod onset;
pub(crate) mod pitch;
pub(crate) mod rhythm;
pub(crate) mod silence;
pub(crate) mod stereo;
pub(crate) mod structure;
pub(crate) mod tempo;
pub(crate) mod util;
