//! The score: tracks, notes, automation, tempo — a declarative, serializable
//! description of a piece. Builders panic on authoring errors (matching
//! fenestra's conventions); `try_` variants return errors and back the RON
//! loader.

use fenestra_anim::{CubicBezier, Ease};

use crate::error::ScoreError;
use crate::param::Param;
use crate::pitch::Pitch;
use crate::tempo::{TempoMap, TempoStep};
use crate::time::{Bpm, Dur, Pos, Ppq, SampleRate, Ticks, TimeSignature, Vel};
use crate::verify_spec::VerifySpec;

/// What a track plays: a named preset from the synth's registry, or a named
/// custom patch supplied to the renderer in a `PatchBank`. Pure data —
/// custom patches keep their *code* in the synth layer, so a score (and its
/// RON form) never captures a closure. A data-form score referencing a
/// custom name renders only when the bank supplies it; `lint` warns.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Instrument {
    Preset(String),
    Custom(String),
}

impl Instrument {
    pub fn preset(name: impl Into<String>) -> Instrument {
        Instrument::Preset(name.into())
    }

    pub fn custom(name: impl Into<String>) -> Instrument {
        Instrument::Custom(name.into())
    }

    pub fn name(&self) -> &str {
        match self {
            Instrument::Preset(n) | Instrument::Custom(n) => n,
        }
    }
}

/// A per-track insert effect, by preset name (`Insert::preset("reverb")`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Insert {
    Preset(String),
}

impl Insert {
    pub fn preset(name: impl Into<String>) -> Insert {
        Insert::Preset(name.into())
    }

    pub fn name(&self) -> &str {
        match self {
            Insert::Preset(n) => n,
        }
    }
}

/// One note, fully resolved to ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Note {
    pub at: Ticks,
    pub dur: Ticks,
    pub pitch: Pitch,
    pub vel: Vel,
}

impl Note {
    /// First tick past the note (`at + dur`).
    pub fn end(&self) -> Ticks {
        self.at + self.dur
    }
}

/// A serializable easing choice for automation keys. Mirrors
/// `fenestra_anim::Ease` minus springs-as-authoring-sugar: springs carry
/// through as data so validation can reject them with a good message
/// (v1 automation is linear/hold/bezier — see `docs/plan.md`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EaseSpec {
    Linear,
    Hold,
    EaseIn,
    EaseOut,
    EaseInOut,
    Bezier(f32, f32, f32, f32),
    Spring {
        stiffness: f32,
        damping: f32,
        velocity: f32,
    },
}

/// The CSS control points behind the named curves, kept in one place so
/// `From<Ease>` can map them back to readable names in the data form.
const EASE_IN: (f32, f32, f32, f32) = (0.42, 0.0, 1.0, 1.0);
const EASE_OUT: (f32, f32, f32, f32) = (0.0, 0.0, 0.58, 1.0);
const EASE_IN_OUT: (f32, f32, f32, f32) = (0.42, 0.0, 0.58, 1.0);

impl EaseSpec {
    /// The fenestra-anim easing this spec evaluates as.
    pub fn to_ease(self) -> Ease {
        let bez = |(x1, y1, x2, y2)| Ease::Bezier(CubicBezier { x1, y1, x2, y2 });
        match self {
            EaseSpec::Linear => Ease::Linear,
            EaseSpec::Hold => Ease::Hold,
            EaseSpec::EaseIn => bez(EASE_IN),
            EaseSpec::EaseOut => bez(EASE_OUT),
            EaseSpec::EaseInOut => bez(EASE_IN_OUT),
            EaseSpec::Bezier(x1, y1, x2, y2) => bez((x1, y1, x2, y2)),
            EaseSpec::Spring {
                stiffness,
                damping,
                velocity,
            } => Ease::Spring(fenestra_anim::Spring {
                stiffness,
                damping,
                velocity,
            }),
        }
    }
}

impl From<Ease> for EaseSpec {
    fn from(e: Ease) -> EaseSpec {
        match e {
            Ease::Linear => EaseSpec::Linear,
            Ease::Hold => EaseSpec::Hold,
            Ease::Bezier(CubicBezier { x1, y1, x2, y2 }) => match (x1, y1, x2, y2) {
                EASE_IN => EaseSpec::EaseIn,
                EASE_OUT => EaseSpec::EaseOut,
                EASE_IN_OUT => EaseSpec::EaseInOut,
                _ => EaseSpec::Bezier(x1, y1, x2, y2),
            },
            Ease::Spring(s) => EaseSpec::Spring {
                stiffness: s.stiffness,
                damping: s.damping,
                velocity: s.velocity,
            },
        }
    }
}

/// One resolved automation key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoKey {
    pub at: Ticks,
    pub value: f32,
    pub ease: EaseSpec,
}

/// An automation curve for one parameter of one track. Keys are sorted by
/// tick and tick-unique (enforced at build).
#[derive(Debug, Clone, PartialEq)]
pub struct Automation {
    pub param: Param,
    pub keys: Vec<AutoKey>,
}

impl Automation {
    /// Samples the authored curve at a tick. Holds outside the key range,
    /// exact key values on their ticks, eased in between — the same
    /// fenestra-anim `locate` the interactive engine uses.
    ///
    /// The `fps` argument to fenestra's easing evaluation is only read by
    /// spring segments, which v1 validation rejects, so a constant is safe
    /// here.
    pub fn value_at(&self, tick: Ticks) -> f32 {
        let keys: Vec<fenestra_anim::Key<f32>> = self
            .keys
            .iter()
            .map(|k| fenestra_anim::key(k.at.0, k.value).ease(k.ease.to_ease()))
            .collect();
        match fenestra_anim::locate(&keys, fenestra_anim::Frames(tick.0), 1) {
            fenestra_anim::Located::Boundary(v) => v,
            fenestra_anim::Located::Interior { from, to, eased } => {
                fenestra_anim::Interpolate::interpolate(from, to, eased)
            }
        }
    }
}

/// An automation key as authored: a position, a value, and an easing into
/// the next segment. `keys![...]` builds these; [`Score::automate`]
/// resolves them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyDef {
    pos: Pos,
    value: f32,
    ease: Ease,
}

impl KeyDef {
    pub fn new(pos: impl Into<Pos>, value: f32, ease: impl Into<Ease>) -> KeyDef {
        KeyDef {
            pos: pos.into(),
            value,
            ease: ease.into(),
        }
    }
}

/// One named track: an instrument, its insert chain, notes, and automation.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub name: String,
    pub instrument: Instrument,
    pub inserts: Vec<Insert>,
    pub notes: Vec<Note>,
    pub automation: Vec<Automation>,
}

/// A complete score. Build with the chainable methods below, load/save RON
/// with [`Score::from_ron`]/[`Score::to_ron`], compile timing with
/// [`Score::tempo_map`].
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    pub(crate) sample_rate: SampleRate,
    pub(crate) ppq: Ppq,
    pub(crate) time_signature: TimeSignature,
    /// Sorted by tick, tick-unique, always starting at tick 0. The authored
    /// BPM is kept alongside the derived integer ns-per-quarter so the data
    /// form round-trips what the author wrote.
    pub(crate) tempo: Vec<TempoChange>,
    pub(crate) tracks: Vec<Track>,
    pub(crate) verify: Vec<VerifySpec>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TempoChange {
    pub at: Ticks,
    pub bpm: Bpm,
    pub npq: u64,
}

impl Score {
    /// A 4/4 score at 120 BPM (both overridable) with no tracks.
    ///
    /// # Panics
    /// On an out-of-range sample rate (8k..=192k) or PPQ (24..=15360).
    pub fn new(sample_rate: SampleRate, ppq: Ppq) -> Score {
        Score::try_new(sample_rate, ppq).unwrap_or_else(|e| panic!("Score::new: {e}"))
    }

    pub fn try_new(sample_rate: SampleRate, ppq: Ppq) -> Result<Score, ScoreError> {
        if sample_rate.0 < SampleRate::MIN || sample_rate.0 > SampleRate::MAX {
            return Err(ScoreError::OutOfRange {
                what: "sample rate",
                value: f64::from(sample_rate.0),
                min: f64::from(SampleRate::MIN),
                max: f64::from(SampleRate::MAX),
            });
        }
        if ppq.0 < Ppq::MIN || ppq.0 > Ppq::MAX {
            return Err(ScoreError::OutOfRange {
                what: "ppq",
                value: f64::from(ppq.0),
                min: f64::from(Ppq::MIN),
                max: f64::from(Ppq::MAX),
            });
        }
        let default_bpm = Bpm(120.0);
        Ok(Score {
            sample_rate,
            ppq,
            time_signature: TimeSignature { beats: 4, unit: 4 },
            tempo: vec![TempoChange {
                at: Ticks::ZERO,
                bpm: default_bpm,
                npq: default_bpm
                    .nanos_per_quarter()
                    .expect("120 BPM is in range"),
            }],
            tracks: Vec::new(),
            verify: Vec::new(),
        })
    }

    /// Sets the (single, v1) time signature.
    pub fn time_signature(self, beats: u32, unit: u32) -> Score {
        self.try_time_signature(beats, unit)
            .unwrap_or_else(|e| panic!("Score::time_signature: {e}"))
    }

    pub fn try_time_signature(mut self, beats: u32, unit: u32) -> Result<Score, ScoreError> {
        let ts = TimeSignature { beats, unit };
        ts.validate(self.ppq)?;
        self.time_signature = ts;
        Ok(self)
    }

    /// Sets the tempo from `at` onward (a step change; ramps are phase 2).
    /// A second change at the same tick replaces the first.
    pub fn tempo(self, at: impl Into<Pos>, bpm: Bpm) -> Score {
        self.try_tempo(at, bpm)
            .unwrap_or_else(|e| panic!("Score::tempo: {e}"))
    }

    pub fn try_tempo(mut self, at: impl Into<Pos>, bpm: Bpm) -> Result<Score, ScoreError> {
        let at = at.into().resolve(self.ppq, self.time_signature)?;
        let npq = bpm.nanos_per_quarter()?;
        let change = TempoChange { at, bpm, npq };
        match self.tempo.binary_search_by_key(&at, |c| c.at) {
            Ok(i) => self.tempo[i] = change,
            Err(i) => self.tempo.insert(i, change),
        }
        Ok(self)
    }

    /// Adds a named track playing `instrument`.
    ///
    /// # Panics
    /// On a duplicate track name.
    pub fn track(self, name: &str, instrument: Instrument) -> Score {
        self.try_track(name, instrument)
            .unwrap_or_else(|e| panic!("Score::track: {e}"))
    }

    pub fn try_track(mut self, name: &str, instrument: Instrument) -> Result<Score, ScoreError> {
        if self.tracks.iter().any(|t| t.name == name) {
            return Err(ScoreError::DuplicateTrack(name.to_owned()));
        }
        self.tracks.push(Track {
            name: name.to_owned(),
            instrument,
            inserts: Vec::new(),
            notes: Vec::new(),
            automation: Vec::new(),
        });
        Ok(self)
    }

    /// Appends an insert effect to a track's chain (applied in push order,
    /// after the voice sum).
    pub fn insert(self, track: &str, insert: Insert) -> Score {
        self.try_insert(track, insert)
            .unwrap_or_else(|e| panic!("Score::insert: {e}"))
    }

    pub fn try_insert(mut self, track: &str, insert: Insert) -> Result<Score, ScoreError> {
        self.track_mut(track)?.inserts.push(insert);
        Ok(self)
    }

    /// Adds a note. Position and duration resolve to ticks exactly — an
    /// off-grid result is an error, never a rounding.
    pub fn note(self, track: &str, at: impl Into<Pos>, dur: Dur, pitch: Pitch, vel: Vel) -> Score {
        self.try_note(track, at, dur, pitch, vel)
            .unwrap_or_else(|e| panic!("Score::note: {e}"))
    }

    pub fn try_note(
        mut self,
        track: &str,
        at: impl Into<Pos>,
        dur: Dur,
        pitch: Pitch,
        vel: Vel,
    ) -> Result<Score, ScoreError> {
        if vel.0 == 0 {
            return Err(ScoreError::ZeroVelocity);
        }
        let at = at.into().resolve(self.ppq, self.time_signature)?;
        let dur = dur.resolve(self.ppq)?;
        if dur.0 == 0 {
            return Err(ScoreError::ZeroDuration);
        }
        self.track_mut(track)?.notes.push(Note {
            at,
            dur,
            pitch,
            vel,
        });
        Ok(self)
    }

    /// Sets an automation curve for one parameter of one track (replacing
    /// any existing curve for that parameter). Build `keys` with the
    /// [`keys!`](crate::keys) macro.
    pub fn automate(self, track: &str, param: Param, keys: Vec<KeyDef>) -> Score {
        self.try_automate(track, param, keys)
            .unwrap_or_else(|e| panic!("Score::automate: {e}"))
    }

    pub fn try_automate(
        mut self,
        track: &str,
        param: Param,
        keys: Vec<KeyDef>,
    ) -> Result<Score, ScoreError> {
        if keys.is_empty() {
            return Err(ScoreError::EmptyKeys);
        }
        let (ppq, ts) = (self.ppq, self.time_signature);
        let mut resolved: Vec<AutoKey> = keys
            .into_iter()
            .map(|k| {
                Ok(AutoKey {
                    at: k.pos.resolve(ppq, ts)?,
                    value: k.value,
                    ease: k.ease.into(),
                })
            })
            .collect::<Result<_, ScoreError>>()?;
        resolved.sort_by_key(|k| k.at);
        for pair in resolved.windows(2) {
            if pair[0].at == pair[1].at {
                return Err(ScoreError::DuplicateKeyTick { tick: pair[0].at.0 });
            }
        }
        let automation = Automation {
            param: param.clone(),
            keys: resolved,
        };
        let track = self.track_mut(track)?;
        match track.automation.iter_mut().find(|a| a.param == param) {
            Some(existing) => *existing = automation,
            None => track.automation.push(automation),
        }
        Ok(self)
    }

    /// Embeds a verification assertion (the programmatic mirror of the RON
    /// `verify:` block; `cochlea render --verify` runs them).
    pub fn with_verify(mut self, spec: VerifySpec) -> Score {
        self.verify.push(spec);
        self
    }

    fn track_mut(&mut self, name: &str) -> Result<&mut Track, ScoreError> {
        self.tracks
            .iter_mut()
            .find(|t| t.name == name)
            .ok_or_else(|| ScoreError::UnknownTrack(name.to_owned()))
    }

    // --- accessors ---

    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    pub fn ppq(&self) -> Ppq {
        self.ppq
    }

    pub fn signature(&self) -> TimeSignature {
        self.time_signature
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn verify_specs(&self) -> &[VerifySpec] {
        &self.verify
    }

    /// The authored tempo steps as `(tick, bpm)` pairs.
    pub fn tempo_changes(&self) -> impl Iterator<Item = (Ticks, Bpm)> + '_ {
        self.tempo.iter().map(|c| (c.at, c.bpm))
    }

    /// Compiles the tempo map — the exact tick→sample conversion.
    pub fn tempo_map(&self) -> TempoMap {
        let steps: Vec<TempoStep> = self
            .tempo
            .iter()
            .map(|c| TempoStep {
                at: c.at,
                npq: c.npq,
            })
            .collect();
        TempoMap::new(self.ppq, self.sample_rate, &steps)
    }

    /// The last tick with authored content (note ends and automation keys),
    /// or tick 0 for an empty score. Renders extend past this by release
    /// tails; `silent_after` assertions reference it.
    pub fn end_tick(&self) -> Ticks {
        self.tracks
            .iter()
            .flat_map(|t| {
                let notes = t.notes.iter().map(Note::end);
                let keys = t
                    .automation
                    .iter()
                    .flat_map(|a| a.keys.iter().map(|k| k.at));
                notes.chain(keys)
            })
            .max()
            .unwrap_or(Ticks::ZERO)
    }

    /// Resolves a position against this score's grid — for callers (CLI,
    /// verify) that need ticks from bar/beat input.
    pub fn resolve(&self, pos: impl Into<Pos>) -> Result<Ticks, ScoreError> {
        pos.into().resolve(self.ppq, self.time_signature)
    }
}
