//! The nine shipped presets and the reverb insert, plus the `PatchBank`
//! registry that resolves names and backs score validation via `Catalog`.
//!
//! Determinism rules in force here (docs/determinism.md): fundsp graphs use
//! the `prelude64` constructors (f64 internal state, f32 node interface);
//! all noise comes from the counter RNG; no fundsp `noise()`/`pluck()`/
//! `feedback()`/`fdn()`; envelope closures are pure arithmetic over note
//! time (piecewise-linear ADSR, and rational `1/(1+t/tau)` pseudo-
//! exponential decays for the drums — no per-sample transcendentals);
//! construction-time libm only.

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

/// Kick drum: a sine whose frequency drops from ~3x the scored pitch down
/// to it over ~25 ms (the classic pitch-envelope thump), with a snappy
/// rational-decay amplitude envelope. Both envelopes are pure arithmetic —
/// `1/(1 + t/tau)` decays, no per-sample transcendentals. Score the kick
/// low (A1/C2-region pitches read as a fundamental, not a note).
struct Kick;

impl Patch for Kick {
    fn name(&self) -> &'static str {
        "kick"
    }

    fn params(&self) -> Vec<ParamInfo> {
        Vec::new()
    }

    fn polyphony(&self) -> Polyphony {
        Polyphony::Mono
    }

    fn release_secs(&self) -> f64 {
        0.25
    }

    fn voice(&self, ctx: &VoiceCtx) -> Voice {
        let f0 = ctx.pitch.hz();
        let amp = f64::from(0.85 * ctx.amp());
        // Frequency: f0 * (1 + 2.2 / (1 + t/tau_f)) — starts at 3.2x f0,
        // settles to f0. Amplitude: 1 ms linear attack, then a squared
        // rational decay that reaches -60 dB-ish within ~0.3 s.
        let freq = fd::envelope(move |t| f0 * (1.0 + 2.2 / (1.0 + t / 0.012)));
        let amp_env = fd::envelope(move |t| {
            let attack = (t / 0.001).clamp(0.0, 1.0);
            let decay = 1.0 / (1.0 + t / 0.09);
            amp * attack * decay * decay
        });
        let graph = ((freq >> fd::sine()) * amp_env) >> fd::pan(0.0);
        boxed_voice(ctx, Vec::new(), graph)
    }
}

/// Snare drum: a short two-partial tonal body at the scored pitch plus a
/// counter-RNG noise burst through a highpass — the noise decays slower
/// than the body, the usual snare shape. Pure-arithmetic envelopes (see
/// [`Kick`]). Score it around D3/E3 for a classic snare body.
struct Snare;

impl Patch for Snare {
    fn name(&self) -> &'static str {
        "snare"
    }

    fn params(&self) -> Vec<ParamInfo> {
        Vec::new()
    }

    fn polyphony(&self) -> Polyphony {
        Polyphony::Poly(4)
    }

    fn release_secs(&self) -> f64 {
        0.3
    }

    fn voice(&self, ctx: &VoiceCtx) -> Voice {
        let f0 = ctx.pitch.hz();
        let amp = f64::from(ctx.amp());
        let body_amp = 0.5 * amp;
        let noise_amp = 0.55 * amp;
        let body_env = fd::envelope(move |t| {
            let attack = (t / 0.001).clamp(0.0, 1.0);
            let decay = 1.0 / (1.0 + t / 0.035);
            body_amp * attack * decay * decay
        });
        let noise_env = fd::envelope(move |t| {
            let attack = (t / 0.001).clamp(0.0, 1.0);
            let decay = 1.0 / (1.0 + t / 0.06);
            noise_amp * attack * decay * decay
        });
        #[expect(
            clippy::cast_possible_truncation,
            reason = "snare partials sit far inside f32's audio band"
        )]
        let (b1, b2) = (f0 as f32, (f0 * 1.6) as f32);
        let body = (fd::sine_hz(b1) + fd::sine_hz(b2)) * body_env;
        let noise = (An(CounterNoise::new(ctx.seed)) >> fd::highpass_hz(2_200.0, 0.7)) * noise_env;
        let graph = (body + noise) >> fd::pan(0.0);
        boxed_voice(ctx, Vec::new(), graph)
    }
}

/// Detuned-saw pad (±4 cents) through an automatable lowpass, slow ADSR.
/// The two saws pan apart (±0.35, constant-power) — the pad is the
/// engine's built-in source of real stereo width.
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
        // Each detuned saw gets its own filter chain and pans to its own
        // side (both filters share the one automatable cutoff cell) —
        // opposing constant-power pans, so the pad carries real stereo
        // width instead of collapsing to dead center.
        let note_len = ctx.note_len_secs();
        let level = 0.10 * ctx.amp();
        let side = |freq: f32, pan: f32| {
            ((fd::saw_hz(freq) * adsr_node(adsr, note_len, level)) | fd::var(&cutoff) | fd::dc(0.5))
                >> fd::lowpass()
                >> fd::pan(pan)
        };
        let graph = side(lo, -0.35) + side(hi, 0.35);
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

/// FM bell: a single-operator FM voice with an inharmonic modulator ratio
/// (metallic, bell-like partials), a near-instant attack, no sustain, and a
/// long decay to silence, plus an automatable `brightness` (the modulation
/// index — more sidebands as it rises). The palette's one non-subtractive
/// voice: it widens the timbral range past the saws/squares the critique
/// called narrow, and shows the score IR carrying a *timbre* knob an agent
/// can sweep, not just a filter cutoff.
struct FmBell;

impl Patch for FmBell {
    fn name(&self) -> &'static str {
        "fm_bell"
    }

    fn params(&self) -> Vec<ParamInfo> {
        vec![ParamInfo {
            param: Param::BRIGHTNESS,
            unit: "index",
            min: 0.0,
            max: 12.0,
            default: 3.0,
        }]
    }

    fn polyphony(&self) -> Polyphony {
        Polyphony::Poly(12)
    }

    fn release_secs(&self) -> f64 {
        0.6
    }

    fn voice(&self, ctx: &VoiceCtx) -> Voice {
        let carrier = freq32(ctx.pitch);
        // A *harmonic* modulator ratio keeps every FM sideband a harmonic of
        // the carrier, so the perceived fundamental — and the pitch a probe
        // reads back — stays exactly on the played note. (An inharmonic ratio
        // sounds more clangorous but smears the pitch, which would break the
        // listen-and-assert loop this engine exists for.)
        let ratio = 2.0_f32;
        // Rest at the declared `brightness` default (`params()` above) so an
        // un-automated voice sounds exactly as the catalog advertises — the
        // engine only overwrites this cell when the score automates the param.
        let index = fd::shared(3.0);
        let note_len = ctx.note_len_secs();
        // Amplitude: sharp attack, no sustain, long ringing decay.
        let amp = Adsr {
            attack: 0.002,
            decay: 0.9,
            sustain: 0.0,
            release: 0.6,
        };
        // The modulation index decays faster than the amplitude, so the strike
        // is bright/metallic and the tail settles toward a clean carrier tone
        // — the classic FM-bell gesture, and what keeps the sustained pitch
        // unambiguous.
        let index_decay = Adsr {
            attack: 0.001,
            decay: 0.28,
            sustain: 0.0,
            release: 0.28,
        };
        // Single-operator FM: instantaneous freq = carrier + depth·modulator,
        // where depth = brightness · carrier · index_env (Hz), fed into a
        // frequency-input sine.
        let depth = fd::var(&index) * fd::dc(carrier) * adsr_node(index_decay, note_len, 1.0);
        let modulator = fd::sine_hz(carrier * ratio) * depth;
        let fm = (modulator + fd::dc(carrier)) >> fd::sine();
        let graph = (fm * adsr_node(amp, note_len, 0.26 * ctx.amp())) >> fd::pan(0.0);
        boxed_voice(ctx, vec![(Param::BRIGHTNESS, index)], graph)
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
    /// The nine shipped presets and the reverb insert.
    pub fn presets() -> PatchBank {
        let mut patches: BTreeMap<String, Arc<dyn Patch>> = BTreeMap::new();
        for patch in [
            Arc::new(SinePatch) as Arc<dyn Patch>,
            Arc::new(SawLead),
            Arc::new(SquareBass),
            Arc::new(ChordPad),
            Arc::new(NoiseHat),
            Arc::new(Pluck),
            Arc::new(Kick),
            Arc::new(Snare),
            Arc::new(FmBell),
        ] {
            patches.insert(patch.name().to_owned(), patch);
        }
        let mut inserts: BTreeMap<String, Arc<dyn InsertFx>> = BTreeMap::new();
        inserts.insert("reverb".to_owned(), Arc::new(ReverbInsert));
        PatchBank { patches, inserts }
    }

    /// Every registered patch name, sorted (BTreeMap order) — powers error
    /// messages and the self-describing authoring reference.
    pub fn patch_names(&self) -> Vec<&str> {
        self.patches.keys().map(String::as_str).collect()
    }

    /// Every registered insert name, sorted.
    pub fn insert_names(&self) -> Vec<&str> {
        self.inserts.keys().map(String::as_str).collect()
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

    fn instrument_names(&self) -> Vec<String> {
        self.patches.keys().cloned().collect()
    }

    fn insert_names(&self) -> Vec<String> {
        self.inserts.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cochlea_score::{SampleRate, Vel};

    /// Invariant that the fm_bell brightness bug violated: every preset must
    /// *rest* at the default it advertises. A patch declares a param's
    /// `default` in `params()` (that value is what the catalog, the docs, and
    /// the score reference promise), but the sound actually comes from the
    /// `Shared` cell the voice builds — and the render engine only writes that
    /// cell when the score *automates* the param
    /// (`crates/render/src/engine.rs`). So a cell whose initial value differs
    /// from the declared default makes an un-automated voice render at an
    /// undocumented value. This checks the whole palette at once, and also
    /// pins that every declared (non-engine) param is actually wired to a cell.
    #[test]
    fn every_preset_rests_at_its_declared_param_defaults() {
        let bank = PatchBank::presets();
        let ctx = VoiceCtx {
            pitch: Pitch::A4,
            vel: Vel(100),
            sample_rate: SampleRate(48_000),
            note_len_samples: 48_000,
            seed: 1,
        };
        for name in bank.patch_names() {
            let patch = bank.patch(name).expect("registered patch resolves");
            let voice = patch.voice(&ctx);
            for info in patch.params() {
                let (_, shared) = voice
                    .controls
                    .iter()
                    .find(|(param, _)| *param == info.param)
                    .unwrap_or_else(|| {
                        panic!(
                            "preset {name:?} declares param {:?} but its voice exposes no \
                             control cell for it — the param is advertised yet unwired",
                            info.param
                        )
                    });
                let resting = shared.value();
                assert!(
                    (resting - info.default).abs() < 1e-6,
                    "preset {name:?} param {:?}: control cell rests at {resting} but the \
                     declared default is {} — an un-automated voice would render at an \
                     undocumented value",
                    info.param,
                    info.default,
                );
            }
        }
    }
}
