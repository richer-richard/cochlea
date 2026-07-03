//! The six shipped presets and the reverb insert, plus the `PatchBank`
//! registry that resolves names and backs score validation via `Catalog`.
//!
//! Determinism rules in force here (docs/determinism.md): fundsp graphs use
//! the `prelude64` constructors (f64 internal state, f32 node interface);
//! all noise comes from the counter RNG; no fundsp `noise()`/`pluck()`/
//! `feedback()`/`fdn()`; envelopes are piecewise-linear closures over note
//! time (pure arithmetic); construction-time libm only.

use std::collections::BTreeMap;
use std::sync::Arc;

use cochlea_score::{Catalog, InstrumentInfo, Param, ParamInfo, Pitch, Polyphony};
use fundsp::prelude::An;
use fundsp::prelude64 as fd;

use crate::env::Adsr;
use crate::nodes::{CounterNoise, KarplusStrong, SchroederReverb};
use crate::patch::{InsertFx, Patch, Voice, VoiceCtx, engine_params};

/// Detune by cents at construction time (libm, not per-sample).
fn cents(c: f64) -> f64 {
    libm::exp2(c / 1200.0)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "pitch frequencies fit f32 comfortably"
)]
fn freq32(pitch: Pitch) -> f32 {
    pitch.hz() as f32
}

/// A piecewise-linear ADSR envelope node for a statically-known note
/// length. fundsp's `envelope` samples the closure at ~2 ms intervals and
/// interpolates linearly — control-rate smoothing, deterministic.
fn adsr_node(
    adsr: Adsr,
    note_len: f64,
    amp: f32,
) -> An<impl fundsp::prelude::AudioNode<Inputs = fd::U0, Outputs = fd::U1>> {
    fd::envelope(move |t| adsr.value(t, note_len) * f64::from(amp))
}

fn boxed_voice(
    ctx: &VoiceCtx,
    controls: Vec<(Param, fundsp::shared::Shared)>,
    graph: An<impl fundsp::prelude::AudioNode<Inputs = fd::U0, Outputs = fd::U2> + 'static>,
) -> Voice {
    let mut unit: Box<dyn fundsp::audiounit::AudioUnit> = Box::new(graph);
    unit.set_sample_rate(f64::from(ctx.sample_rate.0));
    unit.allocate();
    Voice { unit, controls }
}

fn cutoff_param(min: f32, max: f32, default: f32) -> ParamInfo {
    ParamInfo {
        param: Param::CUTOFF_HZ,
        unit: "Hz",
        min,
        max,
        default,
    }
}

/// Pure sine with a gentle AR envelope.
struct SinePatch;

impl Patch for SinePatch {
    fn name(&self) -> &'static str {
        "sine"
    }

    fn params(&self) -> Vec<ParamInfo> {
        Vec::new()
    }

    fn polyphony(&self) -> Polyphony {
        Polyphony::Poly(16)
    }

    fn release_secs(&self) -> f64 {
        0.15
    }

    fn voice(&self, ctx: &VoiceCtx) -> Voice {
        let adsr = Adsr {
            attack: 0.005,
            decay: 0.05,
            sustain: 0.85,
            release: 0.15,
        };
        let graph = (fd::sine_hz(freq32(ctx.pitch))
            * adsr_node(adsr, ctx.note_len_secs(), 0.30 * ctx.amp()))
            >> fd::pan(0.0);
        boxed_voice(ctx, Vec::new(), graph)
    }
}

/// Filtered saw lead: wavetable saw, ADSR, SVF lowpass with automatable
/// cutoff.
struct SawLead;

impl Patch for SawLead {
    fn name(&self) -> &'static str {
        "saw_lead"
    }

    fn params(&self) -> Vec<ParamInfo> {
        vec![cutoff_param(40.0, 18_000.0, 2_400.0)]
    }

    fn polyphony(&self) -> Polyphony {
        Polyphony::Poly(8)
    }

    fn release_secs(&self) -> f64 {
        0.25
    }

    fn voice(&self, ctx: &VoiceCtx) -> Voice {
        let adsr = Adsr {
            attack: 0.01,
            decay: 0.12,
            sustain: 0.7,
            release: 0.25,
        };
        let cutoff = fd::shared(2_400.0);
        let graph = ((fd::saw_hz(freq32(ctx.pitch))
            * adsr_node(adsr, ctx.note_len_secs(), 0.25 * ctx.amp()))
            | fd::var(&cutoff)
            | fd::dc(0.707))
            >> fd::lowpass()
            >> fd::pan(0.0);
        boxed_voice(ctx, vec![(Param::CUTOFF_HZ, cutoff)], graph)
    }
}

/// Mono square bass with a fixed lowpass to tame the buzz.
struct SquareBass;

impl Patch for SquareBass {
    fn name(&self) -> &'static str {
        "square_bass"
    }

    fn params(&self) -> Vec<ParamInfo> {
        Vec::new()
    }

    fn polyphony(&self) -> Polyphony {
        Polyphony::Mono
    }

    fn release_secs(&self) -> f64 {
        0.12
    }

    fn voice(&self, ctx: &VoiceCtx) -> Voice {
        let adsr = Adsr {
            attack: 0.008,
            decay: 0.08,
            sustain: 0.8,
            release: 0.12,
        };
        let graph = (fd::square_hz(freq32(ctx.pitch))
            * adsr_node(adsr, ctx.note_len_secs(), 0.28 * ctx.amp()))
            >> fd::lowpass_hz(900.0, 0.707)
            >> fd::pan(0.0);
        boxed_voice(ctx, Vec::new(), graph)
    }
}

/// Detuned-saw pad (±4 cents) through an automatable lowpass, slow ADSR.
struct ChordPad;

impl Patch for ChordPad {
    fn name(&self) -> &'static str {
        "chord_pad"
    }

    fn params(&self) -> Vec<ParamInfo> {
        vec![cutoff_param(40.0, 12_000.0, 1_200.0)]
    }

    fn polyphony(&self) -> Polyphony {
        Polyphony::Poly(16)
    }

    fn release_secs(&self) -> f64 {
        0.8
    }

    fn voice(&self, ctx: &VoiceCtx) -> Voice {
        let adsr = Adsr {
            attack: 0.15,
            decay: 0.3,
            sustain: 0.8,
            release: 0.8,
        };
        let f = ctx.pitch.hz();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "audio-band frequencies fit f32"
        )]
        let (lo, hi) = ((f * cents(-4.0)) as f32, (f * cents(4.0)) as f32);
        let cutoff = fd::shared(1_200.0);
        let graph = (((fd::saw_hz(lo) + fd::saw_hz(hi))
            * adsr_node(adsr, ctx.note_len_secs(), 0.10 * ctx.amp()))
            | fd::var(&cutoff)
            | fd::dc(0.5))
            >> fd::lowpass()
            >> fd::pan(0.0);
        boxed_voice(ctx, vec![(Param::CUTOFF_HZ, cutoff)], graph)
    }
}

/// Counter-RNG noise through a highpass with a snappy AR envelope.
struct NoiseHat;

impl Patch for NoiseHat {
    fn name(&self) -> &'static str {
        "noise_hat"
    }

    fn params(&self) -> Vec<ParamInfo> {
        Vec::new()
    }

    fn polyphony(&self) -> Polyphony {
        Polyphony::Poly(8)
    }

    fn release_secs(&self) -> f64 {
        0.15
    }

    fn voice(&self, ctx: &VoiceCtx) -> Voice {
        let adsr = Adsr {
            attack: 0.002,
            decay: 0.03,
            sustain: 0.15,
            release: 0.08,
        };
        let graph = ((An(CounterNoise::new(ctx.seed)) >> fd::highpass_hz(7_000.0, 0.7))
            * adsr_node(adsr, ctx.note_len_secs(), 0.35 * ctx.amp()))
            >> fd::pan(0.0);
        boxed_voice(ctx, Vec::new(), graph)
    }
}

/// Karplus-Strong pluck: counter-RNG excitation, natural decay, and a
/// short fade before the voice retires so truncation never clicks.
struct Pluck;

impl Patch for Pluck {
    fn name(&self) -> &'static str {
        "pluck"
    }

    fn params(&self) -> Vec<ParamInfo> {
        Vec::new()
    }

    fn polyphony(&self) -> Polyphony {
        Polyphony::Poly(16)
    }

    fn release_secs(&self) -> f64 {
        1.2
    }

    fn voice(&self, ctx: &VoiceCtx) -> Voice {
        let string = KarplusStrong::new(ctx.seed, ctx.pitch.hz(), 1.1, 0.9 * ctx.amp());
        let end = ctx.note_len_secs() + self.release_secs();
        let fade = fd::envelope(move |t| ((end - t) / 0.05).clamp(0.0, 1.0));
        let graph = (An(string) * fade) >> fd::pan(0.0);
        boxed_voice(ctx, Vec::new(), graph)
    }
}

/// The reverb insert: the in-repo Schroeder (docs/determinism.md explains
/// why fundsp's FDN reverbs are off-limits).
struct ReverbInsert;

impl InsertFx for ReverbInsert {
    fn name(&self) -> &'static str {
        "reverb"
    }

    fn tail_secs(&self) -> f64 {
        2.5
    }

    fn unit(
        &self,
        sample_rate: cochlea_score::SampleRate,
    ) -> Box<dyn fundsp::audiounit::AudioUnit> {
        let mut unit: Box<dyn fundsp::audiounit::AudioUnit> =
            Box::new(An(SchroederReverb::new(0.84, 0.2, 0.3)));
        unit.set_sample_rate(f64::from(sample_rate.0));
        unit.allocate();
        unit
    }
}

/// Name → patch/insert registry. `PatchBank::presets()` holds the six
/// shipped patches plus the reverb insert; custom patches join via
/// [`PatchBank::with_patch`]. Implements [`Catalog`] so `Score::validate`
/// can check params and polyphony against reality.
pub struct PatchBank {
    patches: BTreeMap<String, Arc<dyn Patch>>,
    inserts: BTreeMap<String, Arc<dyn InsertFx>>,
}

impl PatchBank {
    /// The six shipped presets and the reverb insert.
    pub fn presets() -> PatchBank {
        let mut patches: BTreeMap<String, Arc<dyn Patch>> = BTreeMap::new();
        for patch in [
            Arc::new(SinePatch) as Arc<dyn Patch>,
            Arc::new(SawLead),
            Arc::new(SquareBass),
            Arc::new(ChordPad),
            Arc::new(NoiseHat),
            Arc::new(Pluck),
        ] {
            patches.insert(patch.name().to_owned(), patch);
        }
        let mut inserts: BTreeMap<String, Arc<dyn InsertFx>> = BTreeMap::new();
        inserts.insert("reverb".to_owned(), Arc::new(ReverbInsert));
        PatchBank { patches, inserts }
    }

    /// Registers a custom patch (the `Instrument::custom` render path).
    #[must_use]
    pub fn with_patch(mut self, name: &str, patch: Arc<dyn Patch>) -> PatchBank {
        self.patches.insert(name.to_owned(), patch);
        self
    }

    pub fn patch(&self, name: &str) -> Option<&Arc<dyn Patch>> {
        self.patches.get(name)
    }

    pub fn insert_fx(&self, name: &str) -> Option<&Arc<dyn InsertFx>> {
        self.inserts.get(name)
    }
}

impl Catalog for PatchBank {
    fn instrument(&self, name: &str) -> Option<InstrumentInfo> {
        self.patches.get(name).map(|p| {
            let mut params = p.params();
            params.extend(engine_params());
            InstrumentInfo {
                polyphony: p.polyphony(),
                params,
            }
        })
    }

    fn insert(&self, name: &str) -> bool {
        self.inserts.contains_key(name)
    }
}
