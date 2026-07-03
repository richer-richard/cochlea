//! One module per check kind. Every function here is a pure
//! `(rendered, score, ...) -> CheckResult` — no shared mutable state, so
//! [`crate::Verifier`]'s builder methods just compute and queue.

pub(crate) mod discontinuity;
pub(crate) mod loudness;
pub(crate) mod monotone;
pub(crate) mod onset;
pub(crate) mod pitch;
pub(crate) mod silence;
pub(crate) mod util;
