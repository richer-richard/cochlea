//! Note observations → [`Score`]: the audio-to-score arrow of the compose
//! loop, in its pure, score-side half.
//!
//! `render` turns a score into audio and `probe` turns audio into numbers;
//! this turns those numbers back into an *editable score*, so an agent can
//! hear a sketch, get RON it can revise, and render it again. `import_midi`
//! is the same shape for a different input, and this module deliberately
//! mirrors its ethos: **every guess comes back as a warning**, never as a
//! silent claim.
//!
//! # Why plain data in
//!
//! This crate must not depend on `cochlea-features` (the dependency law in
//! `docs/plan.md`: `features` and `score` are independent leaves, so
//! `probe` works on an arbitrary WAV with no score in sight). So the
//! pitch-tracking lives on the caller's side and arrives here as
//! [`NoteObservation`] — plain milliseconds, a MIDI number, a velocity.
//! Everything below is integer tick math over that, unit-testable with no
//! audio anywhere near it.
//!
//! # The conversion
//!
//! Milliseconds are the *input's* native unit (an analyzer measures frames,
//! not ticks), so the float→tick conversion happens exactly once per
//! boundary, here, at authoring time — never accumulated, matching how
//! [`Bpm`] converts once to integer nanoseconds
//! (`docs/determinism.md`). After that everything — quantization, duration
//! floors, overlap repair — is `u64` tick arithmetic.
//!
//! Quantization snaps each start to the nearest [`TranscribeOpts::grid`]
//! multiple and each end to the grid too, so a transcription lands on a
//! musical grid rather than on the tracker's frame boundaries. A note that
//! quantizes to zero length keeps one grid unit: a hit that happened should
//! not vanish because it was short.
//!
//! A grid is a *phase* as much as a spacing, so
//! [`TranscribeOpts::grid_anchor_ms`] pins where its lines fall. Left at
//! zero it assumes the audio starts exactly on a beat, which is wrong for
//! any recording with a count-in, room tone, or a pickup — the notes then
//! quantize against a grid offset from the real one, corrupting the rhythm
//! rather than merely the tempo. Callers that detected a beat grid pass the
//! first beat's time, and everything snaps relative to that instant. Ticks
//! below the anchor snap symmetrically, so a pickup keeps its place.
//!
//! Because the input is a single monophonic line, notes that quantization
//! pulls onto each other are repaired rather than emitted as chords: an
//! overlapping note is shortened to end where the next begins, and a note
//! landing on a tick already taken is dropped. Both are counted in the
//! warnings.

use crate::error::ScoreError;
use crate::pitch::Pitch;
use crate::score::{Instrument, Score};
use crate::time::{Bpm, Dur, Ppq, SampleRate, Ticks, Vel};

/// One note as an analyzer heard it: wall-clock milliseconds from the start
/// of the buffer, a MIDI note number, and a velocity.
///
/// The velocity is the caller's estimate — pitch tracking recovers *when*
/// and *what*, never *how hard*. [`NoteObservation::from_peak_dbfs`] maps a
/// measured peak level into one; a caller with no level information should
/// pass a constant and let the warning say so.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteObservation {
    /// Note start, milliseconds from the start of the analyzed buffer.
    pub start_ms: f64,
    /// Note end, milliseconds. Ends at or before the start are repaired to
    /// the minimum duration rather than rejected.
    pub end_ms: f64,
    /// MIDI note number (60 = C4). Values outside `0..=127` are clamped,
    /// with a warning.
    pub midi: i32,
    /// Performance velocity. Values outside `1..=127` are clamped, with a
    /// warning — above 127 would otherwise become a greater-than-unity gain
    /// downstream.
    pub vel: u8,
}

impl NoteObservation {
    /// An observation whose velocity comes from a measured peak level.
    ///
    /// `peak_dbfs` is mapped linearly over [`VEL_FLOOR_DBFS`]`..=0 dBFS`
    /// onto velocity `1..=127`: a full-scale note reads 127, one at the
    /// floor (or below, or silent) reads 1. Linear-in-decibels is the
    /// honest choice — it matches how the level was measured, and pretending
    /// to recover a performer's MIDI velocity curve from an amplitude
    /// envelope would be a fiction.
    pub fn from_peak_dbfs(start_ms: f64, end_ms: f64, midi: i32, peak_dbfs: f64) -> Self {
        let vel = if peak_dbfs.is_finite() {
            let t = ((peak_dbfs - VEL_FLOOR_DBFS) / -VEL_FLOOR_DBFS).clamp(0.0, 1.0);
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "t is clamped to 0..=1, so the product is within 1..=127"
            )]
            let v = (1.0 + t * 126.0).round() as u8;
            v
        } else {
            // -inf dBFS (a silent window) is the floor, not an error.
            1
        };
        Self {
            start_ms,
            end_ms,
            midi,
            vel,
        }
    }
}

/// Peak level mapped to velocity 1; anything quieter is velocity 1 too.
/// −48 dBFS is far below anything a pitch tracker calls a note, so the
/// usable dynamic range isn't compressed into the top of the scale.
pub const VEL_FLOOR_DBFS: f64 = -48.0;

/// The shortest note a transcription will emit, in grid units — a note that
/// quantizes to nothing still happened.
const MIN_GRID_UNITS: u64 = 1;

/// One note resolved to integer ticks, before overlap repair.
struct PlannedNote {
    start: u64,
    len: u64,
    pitch: Pitch,
    vel: u8,
}

/// How to turn observations into a score.
#[derive(Debug, Clone)]
pub struct TranscribeOpts {
    /// Render rate written into the score.
    pub sample_rate: SampleRate,
    /// Tick resolution.
    pub ppq: Ppq,
    /// The tempo the score is written at. Transcription does not *change*
    /// the music's speed — this is the grid the milliseconds are read
    /// against, so a wrong tempo yields right-sounding but oddly-notated
    /// rhythm.
    pub bpm: Bpm,
    /// Quantization grid as a note duration (`Dur::sixteenth()` and friends),
    /// or `None` to keep the analyzer's raw timing at tick resolution.
    pub grid: Option<Dur>,
    /// Where the quantization grid's phase is pinned, in milliseconds from
    /// the start of the analyzed buffer.
    ///
    /// A grid is a phase as much as a spacing. Pinned at `0.0` (the
    /// default) it assumes the audio begins exactly on a beat, so any
    /// recording with a lead-in — count-in, room tone, a pickup — is
    /// quantized against a grid offset from its real one, which corrupts
    /// the rhythm rather than merely the tempo. Callers that detected a
    /// beat grid should pass the first beat's time here; everything then
    /// snaps relative to that instant instead of to the file's first
    /// sample.
    pub grid_anchor_ms: f64,
    /// Instrument preset for the transcribed track.
    pub preset: String,
    /// Track name.
    pub track_name: String,
}

impl TranscribeOpts {
    /// Defaults: 48 kHz, 960 PPQ, 120 BPM, sixteenth-note grid, one `sine`
    /// track named `lead`.
    pub fn new() -> Self {
        Self {
            sample_rate: SampleRate(48_000),
            ppq: Ppq(960),
            bpm: Bpm(120.0),
            grid: Some(Dur::sixteenth()),
            grid_anchor_ms: 0.0,
            preset: "sine".to_owned(),
            track_name: "lead".to_owned(),
        }
    }

    /// Pin the grid's phase to this instant (milliseconds from the start of
    /// the buffer) — normally the first detected beat.
    #[must_use]
    pub fn with_grid_anchor_ms(mut self, anchor_ms: f64) -> Self {
        self.grid_anchor_ms = anchor_ms;
        self
    }

    /// Set the tempo the milliseconds are read against.
    #[must_use]
    pub fn with_bpm(mut self, bpm: Bpm) -> Self {
        self.bpm = bpm;
        self
    }

    /// Set the quantization grid (`None` keeps raw tick timing).
    #[must_use]
    pub fn with_grid(mut self, grid: Option<Dur>) -> Self {
        self.grid = grid;
        self
    }

    /// Set the instrument preset.
    #[must_use]
    pub fn with_preset(mut self, preset: &str) -> Self {
        self.preset = preset.to_owned();
        self
    }

    /// Set the track name.
    #[must_use]
    pub fn with_track_name(mut self, name: &str) -> Self {
        self.track_name = name.to_owned();
        self
    }

    /// Set the tick resolution.
    #[must_use]
    pub fn with_ppq(mut self, ppq: Ppq) -> Self {
        self.ppq = ppq;
        self
    }

    /// Set the render sample rate written into the score.
    #[must_use]
    pub fn with_sample_rate(mut self, sample_rate: SampleRate) -> Self {
        self.sample_rate = sample_rate;
        self
    }
}

impl Default for TranscribeOpts {
    fn default() -> Self {
        Self::new()
    }
}

/// A transcription: the score, plus every guess it took to get there.
#[derive(Debug)]
pub struct Transcription {
    /// The assembled score — renderable and editable as-is.
    pub score: Score,
    /// Human-readable notes on every assumption, repair, and dropped input.
    /// Surfaced by `cochlea transcribe` and the MCP `transcribe_audio` tool
    /// so the agent knows exactly what to revisit.
    pub warnings: Vec<String>,
}

/// Turn note observations into a score at `opts.bpm`.
///
/// Timing: each boundary converts from milliseconds to ticks once, then
/// snaps to `opts.grid` if set. Notes are emitted in start order; a note
/// that would end at or before it starts is given the minimum duration.
///
/// Every assumption lands in [`Transcription::warnings`]: the tempo used,
/// the grid, clamped pitches, repaired durations, and notes dropped for
/// falling outside the tick range.
pub fn transcribe(
    observations: &[NoteObservation],
    opts: &TranscribeOpts,
) -> Result<Transcription, ScoreError> {
    let mut warnings = Vec::new();
    let mut score = Score::try_new(opts.sample_rate, opts.ppq)?;
    score = score.try_tempo(Ticks::ZERO, opts.bpm)?;
    score = score.try_track(&opts.track_name, Instrument::preset(&opts.preset))?;

    warnings.push(format!(
        "timing was read against {} BPM — a different tempo renotates the same sound",
        opts.bpm.0
    ));
    warnings.push(format!(
        "preset {:?} is a starting point; re-voice to taste",
        opts.preset
    ));

    // Grid in ticks (0 = no quantization). Resolved once: a grid coarser
    // than the tick resolution is the whole point, but a zero grid would
    // divide by zero below.
    let grid_ticks = match opts.grid {
        Some(dur) => {
            let t = dur.resolve(opts.ppq)?.0;
            if t == 0 {
                warnings.push(
                    "the requested quantization grid is finer than one tick; timing kept raw"
                        .to_owned(),
                );
                0
            } else {
                warnings.push(format!("timing quantized to a {t}-tick grid"));
                t
            }
        }
        None => {
            warnings.push("timing kept at raw tick resolution (no quantization)".to_owned());
            0
        }
    };

    // Milliseconds per tick at this tempo — the single float→tick constant.
    // ticks = ms * ppq / ms_per_quarter, with ms_per_quarter = 60_000 / bpm.
    let ticks_per_ms = f64::from(opts.ppq.0) * opts.bpm.0 / 60_000.0;

    // Where the grid's phase is pinned. Converted through the same
    // millisecond→tick rounding as the notes, so anchor and notes agree.
    let anchor_ticks = if grid_ticks == 0 {
        0
    } else {
        match ms_to_ticks(opts.grid_anchor_ms, ticks_per_ms) {
            Some(0) => 0,
            Some(t) => {
                warnings.push(format!(
                    "the grid is phase-locked to the first detected beat at {:.0} ms, not to the \
                     start of the file",
                    opts.grid_anchor_ms
                ));
                t
            }
            None => {
                warnings.push(
                    "the requested grid anchor is not a usable time; the grid is pinned to the \
                     start of the file instead"
                        .to_owned(),
                );
                0
            }
        }
    };

    let mut sorted: Vec<&NoteObservation> = observations.iter().collect();
    sorted.sort_by(|a, b| {
        a.start_ms
            .total_cmp(&b.start_ms)
            .then_with(|| a.midi.cmp(&b.midi))
    });

    let (mut clamped_pitch, mut clamped_vel) = (0usize, 0usize);
    let (mut repaired_len, mut dropped) = (0usize, 0usize);
    let (mut truncated, mut collapsed) = (0usize, 0usize);

    // Resolve every observation to integer ticks first; overlap repair
    // below needs to see neighbours, which a straight emit-as-you-go loop
    // cannot.
    let mut planned: Vec<PlannedNote> = Vec::with_capacity(sorted.len());
    for obs in sorted {
        let Some(start) = ms_to_ticks(obs.start_ms, ticks_per_ms) else {
            dropped += 1;
            continue;
        };
        let Some(end) = ms_to_ticks(obs.end_ms, ticks_per_ms) else {
            dropped += 1;
            continue;
        };

        let start = snap_anchored(start, grid_ticks, anchor_ticks);
        let end = snap_anchored(end, grid_ticks, anchor_ticks);
        let unit = grid_ticks.max(1);
        let len = if end > start {
            end - start
        } else {
            repaired_len += 1;
            unit * MIN_GRID_UNITS
        };

        // Bound the authored position the same way the loader does, rather
        // than letting `try_note` reject the whole transcription over one
        // stray observation.
        if start >= Ticks::MAX.0 || len > Ticks::MAX.0 || start + len > Ticks::MAX.0 {
            dropped += 1;
            continue;
        }

        let midi = if (0..=127).contains(&obs.midi) {
            obs.midi
        } else {
            clamped_pitch += 1;
            obs.midi.clamp(0, 127)
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped into 0..=127 immediately above"
        )]
        let pitch = Pitch(midi as u8);

        // Velocity is bounded the same way pitch is. `Vel` is a bare `u8`
        // newtype and the score IR only rejects zero, but a velocity above
        // 127 becomes a gain above unity downstream (`vel / 127.0` in
        // cochlea-synth), so an out-of-range observation would quietly
        // render *louder than full scale*. Clamp and say so.
        let vel = if (1..=127).contains(&obs.vel) {
            obs.vel
        } else {
            clamped_vel += 1;
            obs.vel.clamp(1, 127)
        };

        planned.push(PlannedNote {
            start,
            len,
            pitch,
            vel,
        });
    }

    // Overlap repair. The input is a *monophonic* line, so two notes
    // sounding at once is never something the analyzer heard — it is an
    // artifact of quantization pulling neighbours together. Rounding both
    // ends to the grid can land a short note's start and end on the same
    // tick (it then takes the minimum duration) while the next note snaps
    // to that same tick, which without this step emits a stack of
    // simultaneous notes instead of a melody.
    //
    // Two notes on the same tick: keep the first, since that is the onset
    // the tracker actually reported there. Otherwise truncate the earlier
    // note to end where the next begins — always leaving it at least one
    // tick, because the next start is strictly greater.
    let mut repaired: Vec<PlannedNote> = Vec::with_capacity(planned.len());
    for note in planned {
        if let Some(last) = repaired.last_mut() {
            if note.start == last.start {
                collapsed += 1;
                continue;
            }
            if last.start + last.len > note.start {
                last.len = note.start - last.start;
                truncated += 1;
            }
        }
        repaired.push(note);
    }

    for note in repaired {
        score = score.try_note(
            &opts.track_name,
            Ticks(note.start),
            Dur::ticks(note.len),
            note.pitch,
            Vel(note.vel),
        )?;
    }

    if clamped_pitch > 0 {
        warnings.push(format!(
            "{clamped_pitch} note(s) fell outside MIDI 0..=127 and were clamped"
        ));
    }
    if clamped_vel > 0 {
        warnings.push(format!(
            "{clamped_vel} note(s) fell outside velocity 1..=127 and were clamped"
        ));
    }
    if truncated > 0 {
        warnings.push(format!(
            "{truncated} note(s) overlapped after quantization and were shortened to end where \
             the next begins"
        ));
    }
    if collapsed > 0 {
        warnings.push(format!(
            "{collapsed} note(s) quantized onto a tick already taken and were dropped — the grid \
             is coarser than the playing"
        ));
    }
    if repaired_len > 0 {
        warnings.push(format!(
            "{repaired_len} note(s) quantized to zero length and were given the minimum duration"
        ));
    }
    if dropped > 0 {
        warnings.push(format!(
            "{dropped} note(s) fell outside the representable tick range and were dropped"
        ));
    }
    if observations.is_empty() {
        warnings.push("no notes were detected; the score has an empty track".to_owned());
    }

    Ok(Transcription { score, warnings })
}

/// Milliseconds → ticks, rounded to nearest. `None` for a value that is not
/// a finite, non-negative, representable tick count.
fn ms_to_ticks(ms: f64, ticks_per_ms: f64) -> Option<u64> {
    if !ms.is_finite() || ms < 0.0 {
        return None;
    }
    let ticks = libm::round(ms * ticks_per_ms);
    if !ticks.is_finite() || ticks < 0.0 || ticks > Ticks::MAX.0 as f64 {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bounded to 0..=Ticks::MAX immediately above"
    )]
    let t = ticks as u64;
    Some(t)
}

/// Snap `t` to the nearest multiple of `grid` (integer, round-half-up).
/// `grid == 0` means no quantization.
fn snap(t: u64, grid: u64) -> u64 {
    if grid == 0 {
        return t;
    }
    let rem = t % grid;
    if rem * 2 >= grid {
        t - rem + grid
    } else {
        t - rem
    }
}

/// [`snap`], but with the grid's phase pinned at `anchor` rather than at
/// tick zero — the grid lines fall on `anchor ± n·grid`.
///
/// Ticks below the anchor snap symmetrically (the distance below is
/// snapped, then subtracted), so a pickup note before the first detected
/// beat lands on a real grid line instead of being dragged forward to it.
fn snap_anchored(t: u64, grid: u64, anchor: u64) -> u64 {
    if grid == 0 {
        return t;
    }
    if t >= anchor {
        anchor + snap(t - anchor, grid)
    } else {
        anchor.saturating_sub(snap(anchor - t, grid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(start_ms: f64, end_ms: f64, midi: i32) -> NoteObservation {
        NoteObservation {
            start_ms,
            end_ms,
            midi,
            vel: 96,
        }
    }

    #[test]
    fn snapping_rounds_to_the_nearest_grid_multiple() {
        assert_eq!(snap(0, 240), 0);
        assert_eq!(snap(119, 240), 0); // just under half
        assert_eq!(snap(120, 240), 240); // exactly half rounds up
        assert_eq!(snap(241, 240), 240);
        assert_eq!(snap(1000, 0), 1000); // no grid
    }

    #[test]
    fn ms_convert_at_120_bpm_960_ppq() {
        // 120 BPM: a quarter note is 500 ms = 960 ticks.
        let tpm = 960.0 * 120.0 / 60_000.0;
        assert_eq!(ms_to_ticks(500.0, tpm), Some(960));
        assert_eq!(ms_to_ticks(0.0, tpm), Some(0));
        assert_eq!(ms_to_ticks(-1.0, tpm), None);
        assert_eq!(ms_to_ticks(f64::NAN, tpm), None);
        assert_eq!(ms_to_ticks(f64::INFINITY, tpm), None);
    }

    #[test]
    fn quarter_notes_land_on_the_beat() {
        let notes = [
            obs(0.0, 500.0, 60),
            obs(500.0, 1000.0, 62),
            obs(1000.0, 1500.0, 64),
        ];
        let t = transcribe(&notes, &TranscribeOpts::new()).expect("transcribes");
        let track = &t.score.tracks()[0];
        assert_eq!(track.notes.len(), 3);
        assert_eq!(track.notes[0].at, Ticks(0));
        assert_eq!(track.notes[1].at, Ticks(960));
        assert_eq!(track.notes[2].at, Ticks(1920));
        assert_eq!(track.notes[0].pitch, Pitch(60));
    }

    #[test]
    fn slightly_late_hits_snap_back_to_the_grid() {
        // 8 ms late at 120 BPM is well inside a sixteenth (240 ticks/125 ms).
        let notes = [obs(508.0, 1004.0, 60)];
        let t = transcribe(&notes, &TranscribeOpts::new()).expect("transcribes");
        assert_eq!(t.score.tracks()[0].notes[0].at, Ticks(960));
    }

    #[test]
    fn raw_timing_is_kept_when_the_grid_is_none() {
        let notes = [obs(508.0, 1004.0, 60)];
        let opts = TranscribeOpts::new().with_grid(None);
        let t = transcribe(&notes, &opts).expect("transcribes");
        // 508 ms * 960 * 120 / 60000 = 975.36 -> 975
        assert_eq!(t.score.tracks()[0].notes[0].at, Ticks(975));
    }

    #[test]
    fn a_zero_length_note_keeps_the_minimum_duration() {
        let notes = [obs(500.0, 500.0, 60)];
        let t = transcribe(&notes, &TranscribeOpts::new()).expect("transcribes");
        let note = &t.score.tracks()[0].notes[0];
        assert_eq!(note.dur, Ticks(240));
        assert!(
            t.warnings.iter().any(|w| w.contains("minimum duration")),
            "{:?}",
            t.warnings
        );
    }

    #[test]
    fn out_of_range_pitches_are_clamped_and_reported() {
        let notes = [obs(0.0, 500.0, -5), obs(500.0, 1000.0, 300)];
        let t = transcribe(&notes, &TranscribeOpts::new()).expect("transcribes");
        let track = &t.score.tracks()[0];
        assert_eq!(track.notes[0].pitch, Pitch(0));
        assert_eq!(track.notes[1].pitch, Pitch(127));
        assert!(
            t.warnings.iter().any(|w| w.contains("clamped")),
            "{:?}",
            t.warnings
        );
    }

    #[test]
    fn non_finite_and_far_future_observations_are_dropped_not_fatal() {
        let notes = [
            obs(f64::NAN, 500.0, 60),
            obs(0.0, f64::INFINITY, 60),
            obs(1e15, 1e15 + 500.0, 60),
            obs(0.0, 500.0, 60), // the one good note
        ];
        let t = transcribe(&notes, &TranscribeOpts::new()).expect("transcribes");
        assert_eq!(t.score.tracks()[0].notes.len(), 1);
        assert!(
            t.warnings.iter().any(|w| w.contains("dropped")),
            "{:?}",
            t.warnings
        );
    }

    #[test]
    fn observations_are_emitted_in_start_order() {
        let notes = [
            obs(1000.0, 1500.0, 64),
            obs(0.0, 500.0, 60),
            obs(500.0, 1000.0, 62),
        ];
        let t = transcribe(&notes, &TranscribeOpts::new()).expect("transcribes");
        let ats: Vec<u64> = t.score.tracks()[0].notes.iter().map(|n| n.at.0).collect();
        assert_eq!(ats, vec![0, 960, 1920]);
    }

    #[test]
    fn an_empty_transcription_is_a_valid_empty_score() {
        let t = transcribe(&[], &TranscribeOpts::new()).expect("transcribes");
        assert_eq!(t.score.tracks().len(), 1);
        assert!(t.score.tracks()[0].notes.is_empty());
        assert!(
            t.warnings.iter().any(|w| w.contains("no notes")),
            "{:?}",
            t.warnings
        );
    }

    #[test]
    fn out_of_range_velocities_are_clamped_and_reported() {
        // `Vel` is a bare u8 newtype and the IR only rejects zero, so an
        // out-of-range observation from a direct library caller would
        // otherwise reach the synth as a >1.0 gain (vel / 127.0).
        let notes = [
            NoteObservation {
                start_ms: 0.0,
                end_ms: 500.0,
                midi: 60,
                vel: 200,
            },
            NoteObservation {
                start_ms: 500.0,
                end_ms: 1000.0,
                midi: 62,
                vel: 0,
            },
        ];
        let t = transcribe(&notes, &TranscribeOpts::new()).expect("transcribes");
        let track = &t.score.tracks()[0];
        assert_eq!(track.notes[0].vel, Vel(127), "200 should clamp to full");
        assert_eq!(track.notes[1].vel, Vel(1), "0 should clamp to the floor");
        assert!(
            t.warnings.iter().any(|w| w.contains("velocity 1..=127")),
            "{:?}",
            t.warnings
        );
    }

    #[test]
    fn anchored_snapping_pins_the_grid_phase() {
        // Grid 240, anchor 100: lines fall on ..., -140, 100, 340, 580.
        assert_eq!(snap_anchored(100, 240, 100), 100);
        assert_eq!(snap_anchored(219, 240, 100), 100); // just under half
        assert_eq!(snap_anchored(220, 240, 100), 340); // exactly half rounds up
        assert_eq!(snap_anchored(345, 240, 100), 340);
        // Below the anchor, symmetric rather than dragged forward. Tick 0
        // sits 100 below the anchor; the neighbouring lines are 100 and
        // -140, so 100 is the nearer one (and the only representable one).
        assert_eq!(snap_anchored(90, 240, 100), 100);
        assert_eq!(snap_anchored(0, 240, 100), 100);
        // Below the anchor the grid continues downward: with anchor 500 the
        // lines are 500, 260, 20, so tick 1 snaps to 20.
        assert_eq!(snap_anchored(1, 240, 500), 20);
        // When the nearest line below would be negative, the result clamps
        // at zero rather than wrapping (ticks are unsigned): anchor 100,
        // grid 150 would put it at -50.
        assert_eq!(snap_anchored(0, 150, 100), 0);
        assert_eq!(snap_anchored(1000, 0, 100), 1000); // no grid
        // A zero anchor is exactly the unanchored behavior.
        for t in [0u64, 119, 120, 241, 1000] {
            assert_eq!(snap_anchored(t, 240, 0), snap(t, 240), "t = {t}");
        }
    }

    #[test]
    fn a_lead_in_quantizes_against_the_beat_not_the_file_start() {
        // A performance that begins 60 ms in — a count-in, room tone, a
        // late start. The notes are exactly on the beat *relative to that
        // instant*, so with the grid pinned there they must land on exact
        // beat multiples, not be smeared by the 60 ms offset.
        let lead_in = 60.0;
        let notes: Vec<NoteObservation> = (0..4)
            .map(|i| {
                let start = lead_in + f64::from(i) * 500.0;
                obs(start, start + 500.0, 60 + i)
            })
            .collect();
        let opts = TranscribeOpts::new().with_grid_anchor_ms(lead_in);
        let t = transcribe(&notes, &opts).expect("transcribes");
        let ats: Vec<u64> = t.score.tracks()[0].notes.iter().map(|n| n.at.0).collect();

        // 60 ms at 120 BPM / 960 PPQ is 115 ticks; each beat is 960.
        assert_eq!(
            ats,
            vec![115, 1075, 2035, 2995],
            "grid pinned to the first beat"
        );
        // Every note sits an exact whole number of beats from the anchor.
        for at in &ats {
            assert_eq!(
                (at - 115) % 960,
                0,
                "note at {at} is not a whole beat from the anchor"
            );
        }
        assert!(
            t.warnings.iter().any(|w| w.contains("phase-locked")),
            "{:?}",
            t.warnings
        );
    }

    #[test]
    fn without_an_anchor_the_lead_in_is_discarded() {
        // Same input, grid left pinned at the file start: every note is
        // pulled onto the file's own grid, so the 60 ms lead-in vanishes
        // and the performance is renotated as if it began on the beat.
        // Kept beside the anchored case so the difference is explicit
        // rather than incidental.
        let lead_in = 60.0;
        let notes: Vec<NoteObservation> = (0..4)
            .map(|i| {
                let start = lead_in + f64::from(i) * 500.0;
                obs(start, start + 500.0, 60 + i)
            })
            .collect();
        let t = transcribe(&notes, &TranscribeOpts::new()).expect("transcribes");
        let ats: Vec<u64> = t.score.tracks()[0].notes.iter().map(|n| n.at.0).collect();
        assert_eq!(
            ats,
            vec![0, 960, 1920, 2880],
            "the lead-in is snapped away without an anchor"
        );
        assert_eq!(ats[0], 0, "the pickup offset is lost entirely");
    }

    #[test]
    fn quantization_never_emits_simultaneous_notes() {
        // The input is a monophonic line, so overlapping output is always a
        // quantization artifact. At 120 BPM / 960 PPQ a 1/16 grid is 240
        // ticks (125 ms): an 80 ms note snaps start and end to the same
        // tick, takes the minimum duration, and its successor snaps to that
        // same tick too — which used to emit a stack.
        let notes = [
            obs(90.0, 170.0, 60),
            obs(170.0, 250.0, 62),
            obs(250.0, 800.0, 64),
        ];
        let t = transcribe(&notes, &TranscribeOpts::new()).expect("transcribes");
        let track = &t.score.tracks()[0];

        let mut spans: Vec<(u64, u64)> = track
            .notes
            .iter()
            .map(|n| (n.at.0, n.at.0 + n.dur.0))
            .collect();
        spans.sort_unstable();
        for pair in spans.windows(2) {
            assert!(
                pair[0].1 <= pair[1].0,
                "note {:?} overlaps {:?} — a monophonic line must not stack",
                pair[0],
                pair[1]
            );
        }
        assert!(
            track.notes.iter().all(|n| n.dur.0 > 0),
            "every emitted note keeps a positive duration"
        );
    }

    #[test]
    fn an_overlapping_note_is_shortened_to_the_next_start() {
        // Two notes a beat apart, the first held way past the second.
        let notes = [obs(0.0, 2000.0, 60), obs(500.0, 1000.0, 62)];
        let t = transcribe(&notes, &TranscribeOpts::new()).expect("transcribes");
        let track = &t.score.tracks()[0];
        assert_eq!(track.notes.len(), 2);
        assert_eq!(track.notes[0].at, Ticks(0));
        assert_eq!(
            track.notes[0].dur,
            Ticks(960),
            "the held note should stop where the next starts"
        );
        assert_eq!(track.notes[1].at, Ticks(960));
        assert!(
            t.warnings.iter().any(|w| w.contains("overlapped")),
            "{:?}",
            t.warnings
        );
    }

    #[test]
    fn notes_landing_on_one_tick_keep_the_first_and_report_it() {
        // Both inside the same 1/16 cell, so both snap to tick 0.
        let notes = [obs(0.0, 20.0, 60), obs(20.0, 40.0, 67)];
        let t = transcribe(&notes, &TranscribeOpts::new()).expect("transcribes");
        let track = &t.score.tracks()[0];
        assert_eq!(track.notes.len(), 1, "one tick holds one note");
        assert_eq!(track.notes[0].pitch, Pitch(60), "the first onset wins");
        assert!(
            t.warnings.iter().any(|w| w.contains("already taken")),
            "{:?}",
            t.warnings
        );
    }

    #[test]
    fn velocity_maps_from_peak_level() {
        let full = NoteObservation::from_peak_dbfs(0.0, 100.0, 60, 0.0);
        let floor = NoteObservation::from_peak_dbfs(0.0, 100.0, 60, VEL_FLOOR_DBFS);
        let below = NoteObservation::from_peak_dbfs(0.0, 100.0, 60, -120.0);
        let silent = NoteObservation::from_peak_dbfs(0.0, 100.0, 60, f64::NEG_INFINITY);
        let half = NoteObservation::from_peak_dbfs(0.0, 100.0, 60, VEL_FLOOR_DBFS / 2.0);
        assert_eq!(full.vel, 127);
        assert_eq!(floor.vel, 1);
        assert_eq!(below.vel, 1);
        assert_eq!(silent.vel, 1);
        assert_eq!(half.vel, 64);
    }

    #[test]
    fn the_transcribed_score_round_trips_through_ron() {
        let notes = [obs(0.0, 500.0, 60), obs(500.0, 1000.0, 67)];
        let t =
            transcribe(&notes, &TranscribeOpts::new().with_preset("pluck")).expect("transcribes");
        let ron = t.score.to_ron().expect("serializes");
        let back = Score::from_ron(&ron).expect("parses");
        assert_eq!(back.tracks()[0].notes.len(), 2);
        assert_eq!(back.tracks()[0].notes[1].pitch, Pitch(67));
    }

    #[test]
    fn a_faster_tempo_notates_the_same_sound_in_fewer_ticks() {
        let notes = [obs(0.0, 500.0, 60), obs(500.0, 1000.0, 62)];
        let slow =
            transcribe(&notes, &TranscribeOpts::new().with_bpm(Bpm(60.0))).expect("transcribes");
        let fast =
            transcribe(&notes, &TranscribeOpts::new().with_bpm(Bpm(240.0))).expect("transcribes");
        // 500 ms is a half note at 60 BPM but an eighth at 240.
        assert_eq!(slow.score.tracks()[0].notes[1].at, Ticks(480));
        assert_eq!(fast.score.tracks()[0].notes[1].at, Ticks(1920));
    }
}
