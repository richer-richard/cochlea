//! Monophonic melody extraction ("transcription-lite"): the YIN f0 track
//! quantized to equal-tempered MIDI notes and segmented into note events.
//! This is the read-back half of the compose loop — an agent that wrote a
//! melody as score notes can probe the render and diff what it *hears* as
//! notes against what it wrote, without a human ear.
//!
//! Deliberately monophonic and lite: one f0 per frame (YIN's model), no
//! polyphony, no onset fusion — a note event here is "the pitch track sat
//! on this semitone for at least [`MIN_NOTE_MS`]". Chords and dense mixes
//! read as whatever single line YIN locks onto; that's a documented
//! property of the underlying tracker, not a bug in the segmentation.
//!
//! Method: quantize each voiced frame to its nearest MIDI note, then clean
//! the sequence with two 1-frame repairs (a single-frame pitch spike
//! between two agreeing neighbors takes their value — YIN octave blips are
//! almost always one frame long — and a single unvoiced frame between two
//! frames of the same note is bridged, so an amplitude dip doesn't split a
//! held note), then group consecutive same-note runs and drop any run
//! shorter than [`MIN_NOTE_MS`]. Each surviving run reports the median f0
//! of its genuinely-voiced frames and its deviation from the note center
//! in cents.

use serde::{Deserialize, Serialize};

use crate::audio::Audio;
use crate::pitch;
use crate::util::note_name;

/// Minimum duration for a note event, milliseconds — runs shorter than
/// this are ornaments, glide transients, or tracker noise, not notes an
/// agent should diff against a score. ~7 analysis hops at 48 kHz.
const MIN_NOTE_MS: f64 = 75.0;

/// One melody note event: a contiguous run of the pitch track on a single
/// equal-tempered semitone. See the module docs for the extraction rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MelodyNote {
    /// Start time, milliseconds (first frame's start time).
    pub start_ms: f64,
    /// End time, milliseconds (one hop past the last frame's start time,
    /// matching [`crate::PitchSegment`]'s tiling convention).
    pub end_ms: f64,
    /// The MIDI note number the run was quantized to.
    pub midi: i32,
    /// Note name + octave (`"A4"`), MIDI's standard numbering (C4 = 60).
    pub name: String,
    /// Median f0 of the run's voiced frames, Hz.
    pub f0_hz: f64,
    /// Deviation of `f0_hz` from `midi`'s center, cents.
    pub cents_off: f64,
}

/// Extract melody note events from `audio` — the standalone entry point
/// (probe embeds the same result at `pitch.melody`). Runs its own YIN pass;
/// callers that already have a probe [`crate::Report`] should read the
/// field instead.
pub fn extract_melody(audio: &Audio) -> Vec<MelodyNote> {
    let mono = audio.mono();
    let track = pitch::f0_track(&mono, audio.sample_rate);
    notes_from_track(&track, audio.sample_rate)
}

/// The extraction over an already-computed f0 track (one
/// `(frame start sample, f0)` per hop) — shared with [`crate::probe`] so
/// the YIN pass runs once.
pub(crate) fn notes_from_track(
    track: &[(usize, Option<f64>)],
    sample_rate: u32,
) -> Vec<MelodyNote> {
    if track.is_empty() || sample_rate == 0 {
        return Vec::new();
    }
    let hop_ms = pitch::hop_len() as f64 / f64::from(sample_rate) * 1000.0;

    // Per-frame quantized note numbers (None = unvoiced).
    let mut midis: Vec<Option<i32>> = track
        .iter()
        .map(|(_, f0)| f0.map(pitch::nearest_midi))
        .collect();

    // 1-frame repairs (see module docs): spike between agreeing neighbors,
    // and unvoiced gap between two frames of the same note.
    for i in 1..midis.len().saturating_sub(1) {
        if let Some(m) = midis[i - 1]
            && midis[i + 1] == Some(m)
            && midis[i] != Some(m)
        {
            midis[i] = Some(m);
        }
    }

    let mut notes = Vec::new();
    let mut i = 0;
    while i < midis.len() {
        let Some(m) = midis[i] else {
            i += 1;
            continue;
        };
        let run_start = i;
        while i < midis.len() && midis[i] == Some(m) {
            i += 1;
        }
        let run_end = i; // exclusive

        let start_ms = frame_ms(track, run_start, hop_ms, sample_rate);
        let end_ms = frame_ms(track, run_end - 1, hop_ms, sample_rate) + hop_ms;
        if end_ms - start_ms < MIN_NOTE_MS {
            continue;
        }

        // Median f0 over the frames that were *genuinely* voiced (repairs
        // above can only add frames whose f0 slot is None or another
        // note's — either way, not this run's evidence).
        let mut f0s: Vec<f64> = track[run_start..run_end]
            .iter()
            .filter_map(|(_, f0)| *f0)
            .filter(|&f0| pitch::nearest_midi(f0) == m)
            .collect();
        let Some(f0_hz) = pitch::median(&mut f0s) else {
            continue;
        };

        notes.push(MelodyNote {
            start_ms,
            end_ms,
            midi: m,
            name: note_name(m),
            f0_hz,
            cents_off: pitch::cents_off(f0_hz, m),
        });
    }
    notes
}

/// Peak absolute sample level in `[start_ms, end_ms)`, as dBFS — a plain
/// numeric measurement over the mono downmix, no score types involved.
///
/// This exists for callers turning a heard note into an authored one (the
/// `transcribe` path): pitch tracking recovers *when* and *what*, never
/// *how hard*, so the loudest sample under a note is the only honest
/// evidence available for its velocity. Returns [`f64::NEG_INFINITY`] for
/// a silent, empty, or out-of-range window — the caller decides what a
/// silent note means.
///
/// Finite bounds are clamped to the buffer; an inverted or empty window
/// reads as silence rather than an error. A *non-finite* bound resolves to
/// zero, the same convention [`Audio::window`]'s frame mapping uses — so
/// `+inf` is not a spelling of "to the end of the buffer", it yields an
/// empty window.
pub fn peak_dbfs_between(audio: &Audio, start_ms: f64, end_ms: f64) -> f64 {
    if audio.sample_rate == 0 {
        return f64::NEG_INFINITY;
    }
    let mono = audio.mono();
    let to_index = |ms: f64| -> usize {
        if !ms.is_finite() || ms <= 0.0 {
            return 0;
        }
        let idx = libm::round(ms / 1000.0 * f64::from(audio.sample_rate));
        if idx >= mono.len() as f64 {
            mono.len()
        } else {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "bounded to 0..=mono.len() immediately above"
            )]
            let i = idx as usize;
            i
        }
    };
    let start = to_index(start_ms);
    let end = to_index(end_ms).max(start);
    let peak = mono[start..end]
        .iter()
        .fold(0.0f64, |acc, &s| acc.max(f64::from(s).abs()));
    if peak > 0.0 {
        20.0 * libm::log10(peak)
    } else {
        f64::NEG_INFINITY
    }
}

/// Start time of frame `idx`, milliseconds — from the track's own sample
/// offsets, so timing agrees with the YIN pass exactly rather than being
/// re-derived from an assumed hop.
fn frame_ms(track: &[(usize, Option<f64>)], idx: usize, hop_ms: f64, sample_rate: u32) -> f64 {
    track.get(idx).map_or_else(
        || idx as f64 * hop_ms,
        |&(start, _)| start as f64 / f64::from(sample_rate) * 1000.0,
    )
}
