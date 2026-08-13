//! Standard MIDI File export: [`Score`] → SMF format 1. The inverse of
//! [`crate::import_midi`], and honest about the same asymmetry in reverse:
//!
//! - **Timing exports exactly.** Score ticks are SMF ticks verbatim (the
//!   score's PPQ becomes the file's division), and the tempo map's step
//!   changes become tempo meta events. No quantization, no float time on the
//!   tick grid — only the tempo value itself is microsecond-quantized, which
//!   is all SMF can carry.
//! - **Instruments are a best-effort GM label.** cochlea's presets map to the
//!   roughest matching General MIDI program (the inverse of the importer's
//!   family mapping, chosen so a re-import lands back on the same preset), and
//!   the percussion presets export on channel 10 with conventional GM drum
//!   keys. A synth voice is not a GM instrument; the program is a hint for
//!   whatever plays the file, not a claim.
//!
//! Together with the importer this closes the loop: a score can be handed to
//! any DAW or notation tool and brought back, with its timing intact.

use crate::error::ScoreError;
use crate::score::{Instrument, Score, Track};

/// The largest delta-time a 4-byte SMF variable-length quantity can hold
/// (28 bits). Scores never come near it in practice — it's ~279k quarter
/// notes at 960 PPQ — but a delta past it can't be encoded, so export errors
/// cleanly rather than truncating.
const MAX_VLQ: u64 = 0x0FFF_FFFF;

/// Export `score` as a Standard MIDI File (format 1): a tempo/meta track
/// followed by one track per score track. See the module docs for what
/// survives the trip. Errors only if a delta-time exceeds SMF's 4-byte VLQ
/// range (a degenerate, far-future-note score).
pub fn export_midi(score: &Score) -> Result<Vec<u8>, ScoreError> {
    let ppq = u16::try_from(score.ppq().0).map_err(|_| {
        ScoreError::Midi(format!("PPQ {} does not fit SMF division", score.ppq().0))
    })?;

    let mut out = Vec::new();

    // ---- header: format 1, ntrks, metrical division ----
    let n_tracks = u16::try_from(1 + score.tracks().len())
        .map_err(|_| ScoreError::Midi("more than 65535 tracks".to_owned()))?;
    let mut header = Vec::with_capacity(6);
    header.extend_from_slice(&1u16.to_be_bytes());
    header.extend_from_slice(&n_tracks.to_be_bytes());
    header.extend_from_slice(&ppq.to_be_bytes());
    write_chunk(&mut out, b"MThd", &header);

    // ---- track 0: time signature + tempo map ----
    write_chunk(&mut out, b"MTrk", &meta_track(score)?);

    // ---- one track per score track ----
    for track in score.tracks() {
        write_chunk(&mut out, b"MTrk", &note_track(track)?);
    }

    Ok(out)
}

/// Build the tempo/meta track body (without the MTrk header): the time
/// signature at tick 0, then every tempo change as a tempo meta event, then
/// end-of-track.
fn meta_track(score: &Score) -> Result<Vec<u8>, ScoreError> {
    // Collect (tick, event-bytes) pairs, tick-sorted, then delta-encode.
    let mut events: Vec<(u64, Vec<u8>)> = Vec::new();

    // Time signature at tick 0: FF 58 04 <num> <den-exp> <clocks/click> <32nds/quarter>.
    let ts = score.signature();
    let den_exp = u8::try_from(ts.unit.trailing_zeros())
        .expect("time-signature unit is a small power of two");
    // Not a clamp: `TimeSignature::validate` bounds `beats` at `u8::MAX`
    // precisely because this byte cannot hold more. It used to clamp, which
    // wrote 255/4 into the file for a score that said 300/4 — wrong data, no
    // error, and no way for the reader to know.
    let num = u8::try_from(ts.beats).expect("time-signature beats are bounded to u8 by validate");
    events.push((0, vec![0xFF, 0x58, 0x04, num, den_exp, 24, 8]));

    // Tempo meta events: FF 51 03 <us-per-quarter, 24-bit>.
    for (tick, bpm) in score.tempo_changes() {
        let us = (60_000_000.0 / bpm.0)
            .round()
            .clamp(1.0, f64::from(0x00FF_FFFFu32));
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "us is clamped into [1, 2^24) above, so the cast is exact and non-negative"
        )]
        let us = us as u32;
        let b = us.to_be_bytes();
        events.push((tick.0, vec![0xFF, 0x51, 0x03, b[1], b[2], b[3]]));
    }

    // Stable sort by tick keeps the tick-0 time signature ahead of a tick-0
    // tempo (both authored at the top of the piece).
    events.sort_by_key(|(tick, _)| *tick);
    emit_track(&events)
}

/// GM program and export channel for a preset (the inverse of the importer's
/// family mapping). `channel` is 9 (GM percussion) for the drum presets, else
/// 0. `drum_key`, when set, overrides each note's pitch with a conventional
/// GM drum key so a re-import re-groups the right percussion voice.
fn preset_voice(preset: &str) -> (u8, u8, Option<u8>) {
    match preset {
        "square_bass" => (0, 33, None),  // electric bass -> 32..=39 (bass)
        "chord_pad" => (0, 48, None),    // strings -> 40..=55 (pad)
        "saw_lead" => (0, 81, None),     // saw lead -> 56..=87 (lead)
        "pluck" => (0, 24, None),        // nylon guitar -> 0..=15|24..=31 (pluck)
        "marimba" => (0, 12, None),      // GM marimba (chromatic percussion)
        "organ" => (0, 16, None),        // GM drawbar organ
        "kick" => (9, 0, Some(36)),      // bass drum 1
        "snare" => (9, 0, Some(38)),     // acoustic snare
        "noise_hat" => (9, 0, Some(42)), // closed hi-hat
        _ => (0, 120, None),             // sine and any custom -> 120..=127 (else -> sine)
    }
}

/// Build one note track's body: a track-name meta, a program change, then the
/// note-on/note-off stream, then end-of-track.
fn note_track(track: &Track) -> Result<Vec<u8>, ScoreError> {
    let preset = match &track.instrument {
        Instrument::Preset(name) | Instrument::Custom(name) => name.as_str(),
    };
    let (channel, program, drum_key) = preset_voice(preset);

    let mut events: Vec<(u64, u8, Vec<u8>)> = Vec::new();
    for note in &track.notes {
        let key = drum_key.unwrap_or(note.pitch.0) & 0x7F;
        let vel = note.vel.0.clamp(1, 127);
        let off_tick = note.at.0.saturating_add(note.dur.0);
        // Note-on (order 1) and note-off (order 0): at a shared tick, offs
        // are emitted first so a re-onset of the same key isn't cut short.
        events.push((note.at.0, 1, vec![0x90 | channel, key, vel]));
        events.push((off_tick, 0, vec![0x80 | channel, key, 0]));
    }
    events.sort_by_key(|(tick, order, _)| (*tick, *order));

    // Track name meta (FF 03 <len> <utf8>) and program change lead the track
    // at tick 0; fold them into the same delta-encoded stream.
    let mut prefixed: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut name_meta = vec![0xFF, 0x03];
    // Bounded by MAX_VLQ, not just by `u32`: a meta event's length is itself
    // a variable-length quantity, and `write_vlq_into` writes into a 4-byte
    // buffer — a value past 28 bits indexed off the end of it and panicked.
    // Unreachable with any real track name (this is 256 MB of it), but the
    // name is score data, and score data does not get to pick the panic.
    let name_len = u32::try_from(track.name.len())
        .ok()
        .filter(|&n| u64::from(n) <= MAX_VLQ)
        .ok_or_else(|| ScoreError::Midi("track name too long for SMF".to_owned()))?;
    write_vlq_into(&mut name_meta, name_len);
    name_meta.extend_from_slice(track.name.as_bytes());
    prefixed.push((0, name_meta));
    prefixed.push((0, vec![0xC0 | channel, program & 0x7F]));
    prefixed.extend(
        events
            .into_iter()
            .map(|(tick, _order, bytes)| (tick, bytes)),
    );

    emit_track(&prefixed)
}

/// Delta-encode a tick-sorted `(tick, event-bytes)` list into an MTrk body,
/// appending the mandatory end-of-track meta. `events` must be sorted by tick.
fn emit_track(events: &[(u64, Vec<u8>)]) -> Result<Vec<u8>, ScoreError> {
    let mut body = Vec::new();
    let mut last_tick = 0u64;
    for (tick, bytes) in events {
        let delta = tick - last_tick;
        if delta > MAX_VLQ {
            return Err(ScoreError::Midi(format!(
                "delta-time {delta} exceeds SMF's {MAX_VLQ}-tick limit"
            )));
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "delta <= MAX_VLQ (28 bits) checked immediately above"
        )]
        write_vlq_into(&mut body, delta as u32);
        body.extend_from_slice(bytes);
        last_tick = *tick;
    }
    // End of track: delta 0, FF 2F 00.
    write_vlq_into(&mut body, 0);
    body.extend_from_slice(&[0xFF, 0x2F, 0x00]);
    Ok(body)
}

/// Append `id` + big-endian length + `data` as one SMF chunk.
fn write_chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
}

/// Append `value` as a MIDI variable-length quantity (7 bits per byte, high
/// bit set on all but the last). `value` must fit 28 bits (checked by the
/// caller via [`MAX_VLQ`]).
fn write_vlq_into(out: &mut Vec<u8>, value: u32) {
    let mut buf = [0u8; 4];
    let mut i = 0;
    buf[i] = (value & 0x7F) as u8;
    let mut v = value >> 7;
    while v > 0 {
        i += 1;
        buf[i] = ((v & 0x7F) as u8) | 0x80;
        v >>= 7;
    }
    for j in (0..=i).rev() {
        out.push(buf[j]);
    }
}
