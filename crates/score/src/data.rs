//! The RON data form (`version = 1`): a serde mirror of the builder that
//! round-trips both ways. Humans (and agents) author positions as
//! `(bar, beat)` tuples, durations as fraction strings, pitches as names —
//! loading resolves them to ticks through the same `try_` builders the
//! programmatic API uses, so both paths share one validation story.

use serde::{Deserialize, Serialize};

use crate::error::ScoreError;
use crate::param::Param;
use crate::score::{Automation, EaseSpec, Insert, Instrument, KeyDef, Note, Score, Track};
use crate::time::{Bpm, Dur, Pos, Ppq, SampleRate, Ticks, TimeSignature, Vel, bar};
use crate::verify_spec::{MonotoneDir, VerifySpec};

/// The data-form version this build reads and writes.
pub const DATA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "Score", deny_unknown_fields)]
struct ScoreDoc {
    version: u32,
    sample_rate: u32,
    ppq: u32,
    #[serde(default = "default_signature")]
    time_signature: (u32, u32),
    tempo: Vec<TempoDoc>,
    tracks: Vec<TrackDoc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    verify: Vec<VerifyDoc>,
}

fn default_signature() -> (u32, u32) {
    (4, 4)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "Tempo", deny_unknown_fields)]
struct TempoDoc {
    tick: u64,
    bpm: f64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "Track", deny_unknown_fields)]
struct TrackDoc {
    name: String,
    instrument: Instrument,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    inserts: Vec<Insert>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    notes: Vec<NoteDoc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    automation: Vec<AutoDoc>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "Note", deny_unknown_fields)]
struct NoteDoc {
    at: (u32, u32),
    #[serde(default, skip_serializing_if = "Option::is_none")]
    off: Option<DurDoc>,
    dur: DurDoc,
    pitch: String,
    vel: u8,
}

/// A duration: a fraction-of-whole string (`"1/4"`, `"1/8."`, `"1/4t"`) or
/// raw ticks. Serialization always writes the canonical fraction string.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum DurDoc {
    Ticks(u64),
    Frac(String),
}

impl DurDoc {
    fn to_dur(&self) -> Result<Dur, ScoreError> {
        match self {
            DurDoc::Ticks(n) => Ok(Dur::ticks(*n)),
            DurDoc::Frac(s) => Dur::parse(s),
        }
    }

    fn from_ticks(t: Ticks, ppq: Ppq) -> DurDoc {
        DurDoc::Frac(Dur::fraction_string(t, ppq))
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "Auto", deny_unknown_fields)]
struct AutoDoc {
    param: String,
    keys: Vec<KeyDoc>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "Key", deny_unknown_fields)]
struct KeyDoc {
    at: (u32, u32),
    #[serde(default, skip_serializing_if = "Option::is_none")]
    off: Option<DurDoc>,
    value: f32,
    #[serde(default, skip_serializing_if = "is_linear")]
    ease: EaseDoc,
}

fn is_linear(e: &EaseDoc) -> bool {
    matches!(e, EaseDoc::Linear)
}

/// Easing names in the data form. No spring variant: v1 automation rejects
/// springs, so the data form can't author one (the builder can, and
/// validation catches it with a message).
#[derive(Debug, Default, Serialize, Deserialize)]
enum EaseDoc {
    #[default]
    Linear,
    Hold,
    EaseIn,
    EaseOut,
    EaseInOut,
    Bezier(f32, f32, f32, f32),
}

impl EaseDoc {
    fn to_spec(&self) -> EaseSpec {
        match *self {
            EaseDoc::Linear => EaseSpec::Linear,
            EaseDoc::Hold => EaseSpec::Hold,
            EaseDoc::EaseIn => EaseSpec::EaseIn,
            EaseDoc::EaseOut => EaseSpec::EaseOut,
            EaseDoc::EaseInOut => EaseSpec::EaseInOut,
            EaseDoc::Bezier(x1, y1, x2, y2) => EaseSpec::Bezier(x1, y1, x2, y2),
        }
    }

    fn from_spec(spec: EaseSpec) -> Result<EaseDoc, ScoreError> {
        Ok(match spec {
            EaseSpec::Linear => EaseDoc::Linear,
            EaseSpec::Hold => EaseDoc::Hold,
            EaseSpec::EaseIn => EaseDoc::EaseIn,
            EaseSpec::EaseOut => EaseDoc::EaseOut,
            EaseSpec::EaseInOut => EaseDoc::EaseInOut,
            EaseSpec::Bezier(x1, y1, x2, y2) => EaseDoc::Bezier(x1, y1, x2, y2),
            EaseSpec::Spring { .. } => {
                return Err(ScoreError::Serialize(ron::Error::Message(
                    "spring easing has no data form (v1 automation rejects it)".into(),
                )));
            }
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
enum VerifyDoc {
    IntegratedLufs {
        target: f64,
        tol: f64,
    },
    TruePeakBelow {
        dbtp: f64,
    },
    OnsetAt {
        track: String,
        at: (u32, u32),
        tol_ms: f64,
    },
    PitchMatchesScore {
        track: String,
        tol_cents: f64,
    },
    Monotone {
        track: String,
        param: String,
        from: (u32, u32),
        to: (u32, u32),
        direction: MonotoneDir,
    },
    NoDiscontinuity {
        track: String,
        db: f64,
    },
    SilentAfter {
        at: (u32, u32),
    },
}

/// `(bar, beat)` + optional sub-beat offset → a [`Pos`].
fn doc_pos(at: (u32, u32), off: Option<&DurDoc>) -> Result<Pos, ScoreError> {
    let pos = bar(at.0).beat(at.1);
    match off {
        Some(d) => Ok(pos.plus(d.to_dur()?)),
        None => Ok(pos),
    }
}

/// A tick back to `(bar, beat)` + optional exact sub-beat offset — the
/// inverse of grid resolution, always exact.
fn pos_doc(t: Ticks, ppq: Ppq, ts: TimeSignature) -> ((u32, u32), Option<DurDoc>) {
    let tpb = ts.ticks_per_beat(ppq);
    let tpbar = ts.ticks_per_bar(ppq);
    let bar = u32::try_from(t.0 / tpbar).expect("bar count fits u32") + 1;
    let rem = t.0 % tpbar;
    let beat = u32::try_from(rem / tpb).expect("beat fits u32") + 1;
    let off = rem % tpb;
    let off = (off != 0).then(|| DurDoc::from_ticks(Ticks(off), ppq));
    ((bar, beat), off)
}

/// RON options for both directions: `implicit_some` lets authors write
/// `off: "1/8"` instead of `off: Some("1/8")`.
fn ron_options() -> ron::Options {
    ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
}

impl Score {
    /// Parses the RON data form (version 1).
    pub fn from_ron(text: &str) -> Result<Score, ScoreError> {
        let doc: ScoreDoc = ron_options().from_str(text)?;
        if doc.version != DATA_VERSION {
            return Err(ScoreError::UnsupportedVersion(doc.version));
        }
        let mut score = Score::try_new(SampleRate(doc.sample_rate), Ppq(doc.ppq))?
            .try_time_signature(doc.time_signature.0, doc.time_signature.1)?;
        for t in &doc.tempo {
            score = score.try_tempo(Ticks(t.tick), Bpm(t.bpm))?;
        }
        for track in doc.tracks {
            score = score.try_track(&track.name, track.instrument)?;
            for insert in track.inserts {
                score = score.try_insert(&track.name, insert)?;
            }
            for note in &track.notes {
                score = score.try_note(
                    &track.name,
                    doc_pos(note.at, note.off.as_ref())?,
                    note.dur.to_dur()?,
                    note.pitch.parse()?,
                    Vel(note.vel),
                )?;
            }
            for auto in &track.automation {
                let keys: Vec<KeyDef> = auto
                    .keys
                    .iter()
                    .map(|k| {
                        Ok(KeyDef::new(
                            doc_pos(k.at, k.off.as_ref())?,
                            k.value,
                            k.ease.to_spec().to_ease(),
                        ))
                    })
                    .collect::<Result<_, ScoreError>>()?;
                score = score.try_automate(&track.name, Param::custom(auto.param.clone()), keys)?;
            }
        }
        for v in doc.verify {
            let spec = verify_from_doc(v, &score)?;
            score = score.with_verify(spec);
        }
        Ok(score)
    }

    /// Serializes to the RON data form (version 1). Grid positions are
    /// reconstructed from ticks exactly; dotted/triplet authoring sugar
    /// canonicalizes to plain fractions.
    pub fn to_ron(&self) -> Result<String, ScoreError> {
        let (ppq, ts) = (self.ppq, self.time_signature);
        let doc = ScoreDoc {
            version: DATA_VERSION,
            sample_rate: self.sample_rate.0,
            ppq: ppq.0,
            time_signature: (ts.beats, ts.unit),
            tempo: self
                .tempo
                .iter()
                .map(|c| TempoDoc {
                    tick: c.at.0,
                    bpm: c.bpm.0,
                })
                .collect(),
            tracks: self
                .tracks
                .iter()
                .map(|t| track_doc(t, ppq, ts))
                .collect::<Result<_, ScoreError>>()?,
            verify: self.verify.iter().map(|v| verify_doc(v, ppq, ts)).collect(),
        };
        let config = ron::ser::PrettyConfig::new().struct_names(true);
        Ok(ron_options().to_string_pretty(&doc, config)?)
    }
}

fn track_doc(t: &Track, ppq: Ppq, ts: TimeSignature) -> Result<TrackDoc, ScoreError> {
    Ok(TrackDoc {
        name: t.name.clone(),
        instrument: t.instrument.clone(),
        inserts: t.inserts.clone(),
        notes: t.notes.iter().map(|n| note_doc(n, ppq, ts)).collect(),
        automation: t
            .automation
            .iter()
            .map(|a| auto_doc(a, ppq, ts))
            .collect::<Result<_, ScoreError>>()?,
    })
}

fn note_doc(n: &Note, ppq: Ppq, ts: TimeSignature) -> NoteDoc {
    let (at, off) = pos_doc(n.at, ppq, ts);
    NoteDoc {
        at,
        off,
        dur: DurDoc::from_ticks(n.dur, ppq),
        pitch: n.pitch.to_string(),
        vel: n.vel.0,
    }
}

fn auto_doc(a: &Automation, ppq: Ppq, ts: TimeSignature) -> Result<AutoDoc, ScoreError> {
    Ok(AutoDoc {
        param: a.param.as_str().to_owned(),
        keys: a
            .keys
            .iter()
            .map(|k| {
                let (at, off) = pos_doc(k.at, ppq, ts);
                Ok(KeyDoc {
                    at,
                    off,
                    value: k.value,
                    ease: EaseDoc::from_spec(k.ease)?,
                })
            })
            .collect::<Result<_, ScoreError>>()?,
    })
}

fn verify_from_doc(v: VerifyDoc, score: &Score) -> Result<VerifySpec, ScoreError> {
    let resolve = |at: (u32, u32)| score.resolve(bar(at.0).beat(at.1));
    Ok(match v {
        VerifyDoc::IntegratedLufs { target, tol } => VerifySpec::IntegratedLufs { target, tol },
        VerifyDoc::TruePeakBelow { dbtp } => VerifySpec::TruePeakBelow { dbtp },
        VerifyDoc::OnsetAt { track, at, tol_ms } => VerifySpec::OnsetAt {
            track,
            at: resolve(at)?,
            tol_ms,
        },
        VerifyDoc::PitchMatchesScore { track, tol_cents } => {
            VerifySpec::PitchMatchesScore { track, tol_cents }
        }
        VerifyDoc::Monotone {
            track,
            param,
            from,
            to,
            direction,
        } => VerifySpec::Monotone {
            track,
            param: Param::custom(param),
            from: resolve(from)?,
            to: resolve(to)?,
            direction,
        },
        VerifyDoc::NoDiscontinuity { track, db } => VerifySpec::NoDiscontinuity { track, db },
        VerifyDoc::SilentAfter { at } => VerifySpec::SilentAfter { at: resolve(at)? },
    })
}

fn verify_doc(v: &VerifySpec, ppq: Ppq, ts: TimeSignature) -> VerifyDoc {
    // Verify positions are bar/beat-grained in the data form; sub-beat
    // offsets would round here, so the builder-side specs used in practice
    // stick to bar/beat too.
    let unresolve = |t: Ticks| pos_doc(t, ppq, ts).0;
    match v {
        VerifySpec::IntegratedLufs { target, tol } => VerifyDoc::IntegratedLufs {
            target: *target,
            tol: *tol,
        },
        VerifySpec::TruePeakBelow { dbtp } => VerifyDoc::TruePeakBelow { dbtp: *dbtp },
        VerifySpec::OnsetAt { track, at, tol_ms } => VerifyDoc::OnsetAt {
            track: track.clone(),
            at: unresolve(*at),
            tol_ms: *tol_ms,
        },
        VerifySpec::PitchMatchesScore { track, tol_cents } => VerifyDoc::PitchMatchesScore {
            track: track.clone(),
            tol_cents: *tol_cents,
        },
        VerifySpec::Monotone {
            track,
            param,
            from,
            to,
            direction,
        } => VerifyDoc::Monotone {
            track: track.clone(),
            param: param.as_str().to_owned(),
            from: unresolve(*from),
            to: unresolve(*to),
            direction: *direction,
        },
        VerifySpec::NoDiscontinuity { track, db } => VerifyDoc::NoDiscontinuity {
            track: track.clone(),
            db: *db,
        },
        VerifySpec::SilentAfter { at } => VerifyDoc::SilentAfter { at: unresolve(*at) },
    }
}
