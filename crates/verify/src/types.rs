//! Newtype wrappers for the tolerances and thresholds the check builder
//! methods take. Each wraps a plain `f64` in the check's natural unit —
//! see the corresponding method on [`crate::Verifier`] for how the value
//! is interpreted.

/// A tolerance in LU (loudness units) — the unit LUFS deltas are measured
/// in. Used by [`crate::Verifier::integrated_lufs`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tol(pub f64);

/// A time tolerance in milliseconds. Used by [`crate::Verifier::onset_at`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ms(pub f64);

/// A pitch tolerance in cents (1/100 of an equal-tempered semitone). Used
/// by [`crate::Verifier::pitch_matches_score`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cents(pub f64);

/// A level in decibels. Used by [`crate::Verifier::no_discontinuity`] (a
/// "louder than -db dBFS" jump threshold) and mirrors the `db` field of
/// [`cochlea_score::VerifySpec::NoDiscontinuity`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Db(pub f64);

/// A tempo tolerance in BPM. Used by [`crate::Verifier::tempo_is`]. A
/// distinct type from `cochlea_score::Bpm` (which authors tempo, not
/// tolerances) so the two are never accidentally interchangeable at a
/// call site.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BpmTol(pub f64);
