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

/// Master-bus processing: an output gain and an optional brick-wall
/// limiter, applied to the f64 stem sum after mixing (the mix becomes
/// `master(Σ stems)`; with the default master the pipeline is untouched
/// and `mix == Σ stems` byte-for-byte as before). This is the tool for
/// hitting loudness targets: push the bus with `gain_db`, let the limiter
/// hold the ceiling, and assert both with `IntegratedLufs` +
/// `TruePeakBelow`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Master {
    gain_db: f32,
    limiter: Option<Limiter>,
}

impl Default for Master {
    fn default() -> Self {
        Master::new()
    }
}

impl Master {
    /// Unity gain, no limiter — the do-nothing master every score starts
    /// with.
    pub fn new() -> Master {
        Master {
            gain_db: 0.0,
            limiter: None,
        }
    }

    /// Output gain in dB, applied before the limiter. Range `-40..=24`.
    ///
    /// # Panics
    /// On an out-of-range value (use [`Master::try_gain_db`] to handle it).
    pub fn gain_db(self, db: f32) -> Master {
        self.try_gain_db(db)
            .unwrap_or_else(|e| panic!("Master::gain_db: {e}"))
    }

    pub fn try_gain_db(mut self, db: f32) -> Result<Master, ScoreError> {
        if !db.is_finite() || !(-40.0..=24.0).contains(&db) {
            return Err(ScoreError::OutOfRange {
                what: "master gain_db",
                value: f64::from(db),
                min: -40.0,
                max: 24.0,
            });
        }
        self.gain_db = db;
        Ok(self)
    }

    /// Install a limiter after the gain stage.
    pub fn limiter(mut self, limiter: Limiter) -> Master {
        self.limiter = Some(limiter);
        self
    }

    /// The configured gain, dB.
    pub fn gain_db_value(&self) -> f32 {
        self.gain_db
    }

    /// The configured limiter, if any.
    pub fn limiter_value(&self) -> Option<&Limiter> {
        self.limiter.as_ref()
    }

    /// Whether this master changes nothing (unity gain, no limiter) — the
    /// renderer skips the master stage entirely in that case, keeping the
    /// pre-0.3.0 byte-exact pipeline.
    pub fn is_default(&self) -> bool {
        self.gain_db == 0.0 && self.limiter.is_none()
    }
}

/// A brick-wall lookahead limiter on the master bus. Sample peaks in the
/// rendered mix never exceed `ceiling_db` — exactly, by construction (the
/// gain at every sample is at most `ceiling / windowed-peak`). Note the
/// guarantee is on *sample* peaks; inter-sample (true) peaks can read up
/// to a fraction of a dB higher, so leave ~1 dB of headroom under a
/// `TruePeakBelow` target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limiter {
    ceiling_db: f32,
    lookahead_ms: f32,
    release_ms: f32,
}

impl Limiter {
    /// A limiter holding `ceiling_db` (range `-40..=0`), with 5 ms
    /// lookahead and 50 ms release.
    ///
    /// # Panics
    /// On an out-of-range ceiling (use [`Limiter::try_new`]).
    pub fn new(ceiling_db: f32) -> Limiter {
        Limiter::try_new(ceiling_db).unwrap_or_else(|e| panic!("Limiter::new: {e}"))
    }

    pub fn try_new(ceiling_db: f32) -> Result<Limiter, ScoreError> {
        if !ceiling_db.is_finite() || !(-40.0..=0.0).contains(&ceiling_db) {
            return Err(ScoreError::OutOfRange {
                what: "limiter ceiling_db",
                value: f64::from(ceiling_db),
                min: -40.0,
                max: 0.0,
            });
        }
        Ok(Limiter {
            ceiling_db,
            lookahead_ms: 5.0,
            release_ms: 50.0,
        })
    }

    /// Lookahead window, ms (range `0..=50`): the gain reaches its reduced
    /// value this far before a peak arrives.
    ///
    /// # Panics
    /// On an out-of-range value (use [`Limiter::try_lookahead_ms`]).
    pub fn lookahead_ms(self, ms: f32) -> Limiter {
        self.try_lookahead_ms(ms)
            .unwrap_or_else(|e| panic!("Limiter::lookahead_ms: {e}"))
    }

    pub fn try_lookahead_ms(mut self, ms: f32) -> Result<Limiter, ScoreError> {
        if !ms.is_finite() || !(0.0..=50.0).contains(&ms) {
            return Err(ScoreError::OutOfRange {
                what: "limiter lookahead_ms",
                value: f64::from(ms),
                min: 0.0,
                max: 50.0,
            });
        }
        self.lookahead_ms = ms;
        Ok(self)
    }

    /// Release time constant, ms (range `1..=1000`): how quickly gain
    /// recovers toward unity after a peak passes.
    ///
    /// # Panics
    /// On an out-of-range value (use [`Limiter::try_release_ms`]).
    pub fn release_ms(self, ms: f32) -> Limiter {
        self.try_release_ms(ms)
            .unwrap_or_else(|e| panic!("Limiter::release_ms: {e}"))
    }

    pub fn try_release_ms(mut self, ms: f32) -> Result<Limiter, ScoreError> {
        if !ms.is_finite() || !(1.0..=1000.0).contains(&ms) {
            return Err(ScoreError::OutOfRange {
                what: "limiter release_ms",
                value: f64::from(ms),
                min: 1.0,
                max: 1000.0,
            });
        }
        self.release_ms = ms;
        Ok(self)
    }

    /// The ceiling, dB.
    pub fn ceiling_db_value(&self) -> f32 {
        self.ceiling_db
    }

    /// The lookahead, ms.
    pub fn lookahead_ms_value(&self) -> f32 {
        self.lookahead_ms
    }

    /// The release, ms.
    pub fn release_ms_value(&self) -> f32 {
        self.release_ms
    }
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
    pub(crate) master: Master,
    pub(crate) verify: Vec<VerifySpec>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TempoChange {
    pub at: Ticks,
    pub bpm: Bpm,
    pub npq: u64,
}

/// Refuse an authored tick past [`Ticks::MAX`] — the domain bound that keeps
/// the exact tempo arithmetic from overflowing `u64`. Applied where a
/// position is finalized (tempo changes, note ends, automation keys) so both
/// the programmatic builder and the RON loader share one guard; grid
/// positions authored as `(bar, beat)` sit far below it, but a raw-tick
/// position or duration could otherwise slip past into unchecked `mul_div`.
fn check_tick(t: Ticks, what: &'static str) -> Result<Ticks, ScoreError> {
    if t > Ticks::MAX {
        return Err(ScoreError::PositionTooFar {
            what,
            tick: t.0,
            max: Ticks::MAX.0,
        });
    }
    Ok(t)
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
            master: Master::new(),
            verify: Vec::new(),
        })
    }

    /// Sets the master-bus processing (gain + optional limiter). The
    /// default master changes nothing — see [`Master`].
    pub fn with_master(mut self, master: Master) -> Score {
        self.master = master;
        self
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
        let at = check_tick(
            at.into().resolve(self.ppq, self.time_signature)?,
            "tempo change",
        )?;
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
        // Bound the note's *end* (at + dur), which subsumes bounding `at`:
        // a raw-tick position or raw-tick duration could otherwise push a
        // tick past what the tempo arithmetic can represent. checked_add so
        // a near-u64::MAX raw duration can't wrap before the comparison.
        match at.0.checked_add(dur.0) {
            Some(end) if end <= Ticks::MAX.0 => {}
            _ => {
                return Err(ScoreError::PositionTooFar {
                    what: "note end",
                    tick: at.0.saturating_add(dur.0),
                    max: Ticks::MAX.0,
                });
            }
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
                    at: check_tick(k.pos.resolve(ppq, ts)?, "automation key")?,
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

    /// The master-bus configuration.
    pub fn master(&self) -> &Master {
        &self.master
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
