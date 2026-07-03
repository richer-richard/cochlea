//! The `Patch` trait: how an instrument name becomes sound. A patch
//! declares its typed automatable params and produces a fresh fundsp voice
//! graph per note — never pooled-and-reset (fundsp ADSR closures carry
//! state `reset()` cannot see; fresh construction is the only clean slate).

use cochlea_score::{Param, ParamInfo, Pitch, Polyphony, SampleRate, Vel};
use fundsp::audiounit::AudioUnit;
use fundsp::shared::Shared;

/// Everything a voice needs to know at construction. `seed` keys all
/// stochastic content for this note through the counter RNG;
/// `note_len_samples` lets envelopes place the release exactly (the
/// schedule is known offline — no live gate signals).
#[derive(Debug, Clone, Copy)]
pub struct VoiceCtx {
    pub pitch: Pitch,
    pub vel: Vel,
    pub sample_rate: SampleRate,
    pub note_len_samples: u64,
    pub seed: u64,
}

impl VoiceCtx {
    /// Note length in seconds — derived once per voice, never accumulated.
    pub fn note_len_secs(&self) -> f64 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "note lengths are far below 2^53"
        )]
        let len = self.note_len_samples as f64;
        len / f64::from(self.sample_rate.0)
    }

    /// Velocity as amplitude: `(vel/127)^2`, a perceptual-ish square law.
    pub fn amp(&self) -> f32 {
        let v = f32::from(self.vel.0) / 127.0;
        v * v
    }
}

/// One playing note: a stereo fundsp graph (0 in, 2 out) plus the shared
/// cells the engine writes automation values into at block starts.
pub struct Voice {
    pub unit: Box<dyn AudioUnit>,
    pub controls: Vec<(cochlea_score::Param, Shared)>,
}

/// An instrument implementation over fundsp. Object-safe; the preset
/// registry stores `Arc<dyn Patch>`.
pub trait Patch: Send + Sync {
    fn name(&self) -> &'static str;

    /// The typed automatable parameter registry (engine-level `gain`/`pan`
    /// are appended by the bank, not declared here).
    fn params(&self) -> Vec<ParamInfo>;

    fn polyphony(&self) -> Polyphony;

    /// Release tail in seconds — the voice renders for
    /// `note_len + release_secs` and is then retired. A static function of
    /// the patch, so voice lifetimes are pure schedule math (invariant 7).
    fn release_secs(&self) -> f64;

    /// Builds a fresh voice graph for one note, sample rate already set.
    fn voice(&self, ctx: &VoiceCtx) -> Voice;
}

/// A per-track insert effect (stereo in, stereo out), by name.
pub trait InsertFx: Send + Sync {
    fn name(&self) -> &'static str;

    /// How long the effect rings after input stops (extends the render).
    fn tail_secs(&self) -> f64;

    fn unit(&self, sample_rate: SampleRate) -> Box<dyn AudioUnit>;
}

/// Engine-level params every track has, regardless of patch.
pub fn engine_params() -> Vec<ParamInfo> {
    vec![
        ParamInfo {
            param: Param::GAIN,
            unit: "linear",
            min: 0.0,
            max: 4.0,
            default: 1.0,
        },
        ParamInfo {
            param: Param::PAN,
            unit: "pan",
            min: -1.0,
            max: 1.0,
            default: 0.0,
        },
    ]
}
