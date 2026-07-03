//! Assertion DSL over rendered audio + extracted features, RON-embeddable
//! under the score's `verify` block, with a machine-readable JSON failure
//! report. See `docs/plan.md`.
//!
//! [`VerifyExt::verify`] starts a chainable [`Verifier`] over a
//! `cochlea_render::Rendered` and its source `cochlea_score::Score`; each
//! builder method queues one check (loudness, onsets, pitch, automation
//! shape, clicks, silence) evaluated against `cochlea_features` extractors.
//! [`Verifier::run`] assembles a schema-versioned [`VerifyReport`], never
//! panicking on a bad track name — that check simply fails with detail.
//!
//! ```
//! use cochlea_score::*;
//! use cochlea_verify::{Db, VerifyExt};
//!
//! let score = Score::new(SampleRate(48_000), Ppq(960))
//!     .track("lead", Instrument::preset("sine"))
//!     .note("lead", bar(1), Dur::quarter(), Pitch::A4, Vel(96));
//! let rendered = cochlea_render::render(&score).unwrap();
//!
//! let report = rendered
//!     .verify(&score)
//!     .true_peak_below(0.0)
//!     .no_discontinuity("lead", Db(20.0))
//!     .run();
//! assert!(report.passed, "{:?}", report.checks);
//! ```
//!
//! Every check is also a data form, [`cochlea_score::VerifySpec`], embeddable
//! in a score's RON `verify:` block; [`Verifier::with_spec`]/
//! [`Verifier::with_specs`] run those against the same render.

mod checks;
mod report;
mod types;
mod verifier;

pub use report::{CheckResult, VerifyReport};
pub use types::{Cents, Db, Ms, Tol};
pub use verifier::{Verifier, VerifyExt};
