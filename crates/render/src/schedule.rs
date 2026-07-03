//! Score → event schedule compilation. Pure: every sample index, voice
//! lifetime, and seed below is a function of the score alone (invariant 7).
//! This is the single place tick→sample conversion happens (rounding rule 2
//! of `docs/determinism.md`).

use std::sync::Arc;

use cochlea_score::{Automation, Pitch, SampleRate, Score, Vel};
use cochlea_synth::{InsertFx, Patch, PatchBank, note_seed};

use crate::error::RenderError;

/// One note with its timing fully resolved to samples.
#[derive(Clone, Copy)]
pub(crate) struct PlannedNote {
    /// First sample of the note.
    pub on: u64,
    /// First sample past the held note (note-off).
    pub off: u64,
    /// First sample past the voice: `off + release`, statically known, so
    /// voice lifetime never depends on rendered signal.
    pub end: u64,
    pub pitch: Pitch,
    pub vel: Vel,
    /// Counter-RNG stream for this note: `note_seed(track_idx, note_idx)`
    /// over *authored* indices, so reordering-stable.
    pub seed: u64,
}

pub(crate) struct TrackPlan {
    pub name: String,
    pub patch: Arc<dyn Patch>,
    pub inserts: Vec<Arc<dyn InsertFx>>,
    /// Sorted by `(on, authored index)` — the total order voice allocation
    /// and stealing run on.
    pub notes: Vec<PlannedNote>,
    /// Automation curves in the tick domain; the engine samples them at
    /// block starts through the tempo map's inverse.
    pub automation: Vec<Automation>,
}

pub(crate) struct Schedule {
    pub sample_rate: SampleRate,
    /// Render length: the longest track. Zero for an empty score.
    pub total_samples: u64,
    pub tracks: Vec<TrackPlan>,
}

/// Release/tail seconds → samples, rounded up so tails never truncate.
fn secs_to_samples(secs: f64, sample_rate: SampleRate) -> u64 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "tails are a few seconds; ceil of a small positive value"
    )]
    let n = libm::ceil(secs * f64::from(sample_rate.0)) as u64;
    n
}

pub(crate) fn compile(score: &Score, bank: &PatchBank) -> Result<Schedule, RenderError> {
    let sample_rate = score.sample_rate();
    let map = score.tempo_map();
    let mut tracks = Vec::with_capacity(score.tracks().len());
    let mut total_samples = 0u64;

    for (track_idx, track) in score.tracks().iter().enumerate() {
        let patch = bank
            .patch(track.instrument.name())
            .ok_or_else(|| RenderError::UnknownInstrument {
                track: track.name.clone(),
                name: track.instrument.name().to_owned(),
                available: "sine, saw_lead, square_bass, chord_pad, noise_hat, pluck".to_owned(),
            })?
            .clone();
        let inserts = track
            .inserts
            .iter()
            .map(|i| {
                bank.insert_fx(i.name())
                    .cloned()
                    .ok_or_else(|| RenderError::UnknownInsert {
                        track: track.name.clone(),
                        name: i.name().to_owned(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let release = secs_to_samples(patch.release_secs(), sample_rate);
        let mut notes: Vec<PlannedNote> = track
            .notes
            .iter()
            .enumerate()
            .map(|(note_idx, n)| {
                let on = map.sample_at(n.at);
                let off = map.sample_at(n.end());
                PlannedNote {
                    on,
                    off,
                    end: off + release,
                    pitch: n.pitch,
                    vel: n.vel,
                    seed: note_seed(track_idx as u64, note_idx as u64),
                }
            })
            .collect();
        notes.sort_by_key(|n| n.on); // stable: authored order breaks ties

        // Samples this track needs: content end plus insert tails. All
        // stems still render to the longest track's length so the stem sum
        // is well-defined samplewise.
        let content_end = notes.iter().map(|n| n.end).max().unwrap_or(0);
        let tail: f64 = inserts.iter().map(|i| i.tail_secs()).sum();
        let len = if content_end == 0 {
            0
        } else {
            content_end + secs_to_samples(tail, sample_rate)
        };
        total_samples = total_samples.max(len);

        tracks.push(TrackPlan {
            name: track.name.clone(),
            patch,
            inserts,
            notes,
            automation: track.automation.clone(),
        });
    }

    if total_samples > u64::from(sample_rate.0) * 3600 {
        return Err(RenderError::TooLong {
            samples: total_samples,
            sample_rate: sample_rate.0,
        });
    }

    Ok(Schedule {
        sample_rate,
        total_samples,
        tracks,
    })
}
