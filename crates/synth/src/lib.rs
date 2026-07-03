//! Instrument layer over fundsp: the [`Patch`] trait (typed param registry,
//! fresh voice graph per note), six presets, the reverb insert, and the
//! counter-based `(seed, sample_index)` RNG — the workspace's only
//! randomness.
//!
//! Determinism rules enforced here (see `docs/determinism.md` and the
//! clippy.toml ban list): voices tick sample-by-sample, prelude64 node
//! constructors, no fundsp `noise()`/`pluck()`/`feedback()`/`fdn()`, no std
//! float transcendentals, libm at construction time only.

mod env;
mod nodes;
mod patch;
mod presets;
mod rng;

pub use env::Adsr;
pub use nodes::{CounterNoise, KarplusStrong, SchroederReverb};
pub use patch::{InsertFx, Patch, Voice, VoiceCtx, engine_params};
pub use presets::PatchBank;
pub use rng::{crng, crng_f32, note_seed};
