//! Standard MIDI File import: SMF format 0/1 → [`Score`]. Hand-rolled
//! (an SMF is a few chunk headers, variable-length quantities, and running
//! status — a dependency would be bigger than the parser), deterministic,
//! and honest about what survives the trip:
//!
//! - **Timing maps exactly.** SMF delta ticks land on our integer tick
//!   grid verbatim: the file's division becomes the score's PPQ, tempo
//!   meta events become the tempo map's step changes. No quantization, no
//!   float time.
//! - **Instrumentation is a starting point, not a claim.** MIDI carries
//!   General MIDI program numbers; cochlea has a handful of synth presets.
//!   Programs map to the roughest reasonable preset family (bass programs
//!   → `square_bass`, pads/strings → `chord_pad`, leads/brass →
//!   `saw_lead`, plucked things → `pluck`, else `sine`), and channel 10
//!   percussion splits into `kick`/`snare`/`noise_hat` tracks by GM key.
//!   The import exists so an agent can *hear a draft and then re-voice
//!   it* — the returned [`MidiImport::warnings`] say what was guessed.
//! - **What's dropped, dropped loudly**: SMPTE-division files and format-2
//!   files are errors; time signatures past the first, and events cochlea
//!   has no model for (pitch bend, CCs, aftertouch), are skipped with a
//!   warning each kind, never silently.

use std::collections::BTreeMap;

use crate::error::ScoreError;
use crate::pitch::Pitch;
use crate::score::{Instrument, Score};
use crate::time::{Bpm, Dur, Ppq, SampleRate, Ticks, Vel};

/// Result of a MIDI import: the score plus human-readable notes about
/// every mapping guess and dropped feature — surfaced by `cochlea import`
/// and the MCP `import_midi` tool so the agent knows what to revisit.
#[derive(Debug)]
pub struct MidiImport {
    pub score: Score,
    pub warnings: Vec<String>,
}

/// Import a Standard MIDI File (format 0 or 1). `sample_rate` is the
/// score-side render rate (MIDI has no opinion). See the module docs for
/// the mapping rules.
pub fn import_midi(bytes: &[u8], sample_rate: SampleRate) -> Result<MidiImport, ScoreError> {
    let mut r = Reader::new(bytes);

    // ---- header chunk ----
    // Check the magic before trusting any length field — a non-MIDI file
    // should say "not a MIDI file", not whatever garbage its bytes decode
    // to as a chunk length.
    if bytes.len() < 4 || &bytes[..4] != b"MThd" {
        return Err(err("file does not start with an MThd header chunk"));
    }
    let (_, header) = r.chunk()?;
    if header.len() < 6 {
        return Err(err("MThd chunk shorter than 6 bytes"));
    }
    let format = u16::from_be_bytes([header[0], header[1]]);
    let ntrks = u16::from_be_bytes([header[2], header[3]]);
    let division = u16::from_be_bytes([header[4], header[5]]);
    if format > 1 {
        return Err(err(&format!(
            "format {format} is unsupported (only 0 and 1; format 2 has no single timeline)"
        )));
    }
    if division & 0x8000 != 0 {
        return Err(err(
            "SMPTE (frames-per-second) division is unsupported; re-export with metrical division",
        ));
    }
    if division == 0 {
        return Err(err("division of 0 ticks per quarter"));
    }
    if u32::from(division) < Ppq::MIN || u32::from(division) > Ppq::MAX {
        return Err(err(&format!(
            "division {division} outside the supported PPQ range {}..={}",
            Ppq::MIN,
            Ppq::MAX
        )));
    }

    // ---- track chunks ----
    let mut tracks = Vec::new();
    for i in 0..ntrks {
        let (id, data) = r
            .chunk()
            .map_err(|_| err(&format!("missing track chunk {i} of {ntrks}")))?;
        if &id == b"MTrk" {
            tracks.push(parse_track(data, i)?);
        }
        // Unknown chunk types are skipped per the SMF spec.
    }

    assemble(tracks, division, sample_rate)
}

fn err(msg: &str) -> ScoreError {
    ScoreError::Midi(msg.to_owned())
}

// --------------------------------------------------------------- parsing

/// One note event, fully paired.
struct RawNote {
    channel: u8,
    at: u64,
    dur: u64,
    key: u8,
    vel: u8,
}

/// Everything extracted from one MTrk chunk.
struct RawTrack {
    index: u16,
    name: Option<String>,
    notes: Vec<RawNote>,
    /// `(tick, microseconds per quarter)` tempo meta events.
    tempos: Vec<(u64, u32)>,
    /// `(numerator, denominator)` of the first time-signature event.
    time_signature: Option<(u32, u32)>,
    /// Last General MIDI program set per channel (first wins per channel —
    /// one instrument per cochlea track).
    programs: BTreeMap<u8, u8>,
    skipped: Vec<&'static str>,
}

fn parse_track(data: &[u8], index: u16) -> Result<RawTrack, ScoreError> {
    let mut r = Reader::new(data);
    let mut tick = 0u64;
    let mut running_status: Option<u8> = None;
    // (channel, key) -> (start tick, velocity)
    let mut active: BTreeMap<(u8, u8), (u64, u8)> = BTreeMap::new();
    let mut out = RawTrack {
        index,
        name: None,
        notes: Vec::new(),
        tempos: Vec::new(),
        time_signature: None,
        programs: BTreeMap::new(),
        skipped: Vec::new(),
    };
    let mut skipped_kinds: Vec<&'static str> = Vec::new();

    while !r.is_empty() {
        tick = tick
            .checked_add(u64::from(r.vlq()?))
            .ok_or_else(|| err("tick overflow"))?;

        let first = r.u8()?;
        let status = if first & 0x80 != 0 {
            // Channel statuses arm running status; sysex and meta events
            // *cancel* it (SMF spec) — a data byte after one of those with
            // no fresh status byte is malformed, not a continuation.
            running_status = if first < 0xF0 { Some(first) } else { None };
            first
        } else {
            r.rewind(1);
            running_status.ok_or_else(|| err("data byte with no running status"))?
        };

        match status {
            0xFF => {
                // Meta event: type, VLQ length, payload.
                let meta = r.u8()?;
                let len = r.vlq()? as usize;
                let payload = r.take(len)?;
                match meta {
                    0x2F => break, // end of track
                    0x51 if payload.len() == 3 => {
                        let us = u32::from_be_bytes([0, payload[0], payload[1], payload[2]]);
                        out.tempos.push((tick, us));
                    }
                    0x58 => {
                        if payload.len() >= 2 && out.time_signature.is_none() {
                            let num = u32::from(payload[0]);
                            // The denominator is stored as a power of two
                            // (2 = quarter). The byte is unvalidated file
                            // data, and anything past 31 overflows the
                            // shift — a panic on a corrupt file, and no
                            // music has a 2³²-th note. Treat it as
                            // malformed and keep the default 4/4.
                            match 1u32.checked_shl(u32::from(payload[1])) {
                                Some(den) => out.time_signature = Some((num, den)),
                                None => push_once(
                                    &mut skipped_kinds,
                                    "an out-of-range time-signature denominator",
                                ),
                            }
                        } else if out.time_signature.is_some() {
                            push_once(&mut skipped_kinds, "extra time-signature changes");
                        }
                    }
                    0x03 if out.name.is_none() => {
                        out.name = Some(String::from_utf8_lossy(payload).trim().to_owned());
                    }
                    _ => {} // other meta events carry nothing we model
                }
            }
            0xF0 | 0xF7 => {
                // SysEx: VLQ length, payload — nothing cochlea models.
                let len = r.vlq()? as usize;
                r.take(len)?;
                push_once(&mut skipped_kinds, "sysex data");
            }
            _ => {
                let kind = status & 0xF0;
                let channel = status & 0x0F;
                match kind {
                    0x90 => {
                        let key = r.u8()? & 0x7F;
                        let vel = r.u8()? & 0x7F;
                        if vel == 0 {
                            close_note(&mut active, &mut out.notes, channel, key, tick);
                        } else {
                            // A retriggered key closes the previous note at
                            // the retrigger tick.
                            close_note(&mut active, &mut out.notes, channel, key, tick);
                            active.insert((channel, key), (tick, vel));
                        }
                    }
                    0x80 => {
                        let key = r.u8()? & 0x7F;
                        let _release_vel = r.u8()?;
                        close_note(&mut active, &mut out.notes, channel, key, tick);
                    }
                    0xC0 => {
                        let program = r.u8()? & 0x7F;
                        out.programs.entry(channel).or_insert(program);
                    }
                    0xA0 | 0xB0 | 0xE0 => {
                        r.take(2)?;
                        push_once(
                            &mut skipped_kinds,
                            match kind {
                                0xA0 => "polyphonic aftertouch",
                                0xB0 => "control changes",
                                _ => "pitch bends",
                            },
                        );
                    }
                    0xD0 => {
                        r.take(1)?;
                        push_once(&mut skipped_kinds, "channel aftertouch");
                    }
                    _ => return Err(err(&format!("unexpected status byte 0x{status:02X}"))),
                }
            }
        }
    }

    // Notes still sounding at end-of-track close there.
    let end = tick;
    let keys: Vec<(u8, u8)> = active.keys().copied().collect();
    for (channel, key) in keys {
        close_note(&mut active, &mut out.notes, channel, key, end);
    }
    // Notes accumulate in *close* order (an early long note closes after a
    // late short one); the score wants start order, deterministically.
    out.notes.sort_by_key(|n| (n.at, n.channel, n.key, n.dur));
    out.skipped = skipped_kinds;
    Ok(out)
}

fn close_note(
    active: &mut BTreeMap<(u8, u8), (u64, u8)>,
    notes: &mut Vec<RawNote>,
    channel: u8,
    key: u8,
    tick: u64,
) {
    if let Some((start, vel)) = active.remove(&(channel, key)) {
        notes.push(RawNote {
            channel,
            at: start,
            // A zero-length hit (on/off on the same tick) still happened —
            // give it one tick rather than dropping it.
            dur: (tick - start).max(1),
            key,
            vel,
        });
    }
}

fn push_once(kinds: &mut Vec<&'static str>, kind: &'static str) {
    if !kinds.contains(&kind) {
        kinds.push(kind);
    }
}

/// Big-endian byte reader with VLQ support.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn u8(&mut self) -> Result<u8, ScoreError> {
        let b = *self
            .data
            .get(self.pos)
            .ok_or_else(|| err("unexpected end of data"))?;
        self.pos += 1;
        Ok(b)
    }

    fn rewind(&mut self, n: usize) {
        self.pos -= n.min(self.pos);
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ScoreError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.data.len())
            .ok_or_else(|| err("unexpected end of data"))?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// A MIDI variable-length quantity (7 bits per byte, high bit =
    /// continue, at most 4 bytes).
    fn vlq(&mut self) -> Result<u32, ScoreError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let b = self.u8()?;
            value = (value << 7) | u32::from(b & 0x7F);
            if b & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(err("variable-length quantity longer than 4 bytes"))
    }

    /// The next `(chunk id, chunk data)` pair.
    fn chunk(&mut self) -> Result<([u8; 4], &'a [u8]), ScoreError> {
        let id = self.take(4)?;
        let len = u32::from_be_bytes(self.take(4)?.try_into().expect("take(4) returns 4 bytes"));
        let data = self.take(len as usize)?;
        Ok(([id[0], id[1], id[2], id[3]], data))
    }
}

// -------------------------------------------------------------- assembly

/// GM percussion (channel 10) key → `(preset, conventional pitch)`.
fn percussion_voice(key: u8) -> (&'static str, Pitch) {
    match key {
        35 | 36 => ("kick", Pitch::A1),
        37..=40 => ("snare", Pitch::D3), // side stick, snares, claps
        42 | 44 | 46 => ("noise_hat", Pitch::A4),
        49 | 51..=59 => ("noise_hat", Pitch::A5), // cymbals ride/crash
        _ => ("snare", Pitch::D3),
    }
}

/// GM program number → the roughest reasonable preset family.
fn program_preset(program: u8) -> &'static str {
    match program {
        12 => "marimba",                                   // GM marimba (its exact voice)
        16 => "organ",                                     // GM drawbar organ (its exact voice)
        32..=39 => "square_bass",                          // basses
        17..=23 | 40..=55 | 88..=103 => "chord_pad",       // other organs, strings, pads, fx
        56..=87 => "saw_lead",                             // brass, reeds, pipes, synth leads
        0..=11 | 13..=15 | 24..=31 | 104..=119 => "pluck", // keys, guitars, ethnic, percussive
        _ => "sine",
    }
}

fn assemble(
    raw_tracks: Vec<RawTrack>,
    division: u16,
    sample_rate: SampleRate,
) -> Result<MidiImport, ScoreError> {
    let mut warnings = Vec::new();
    let mut score = Score::try_new(sample_rate, Ppq(u32::from(division)))?;

    // Time signature: the first one anywhere (format 1 keeps it in track 0).
    if let Some((num, den)) = raw_tracks.iter().find_map(|t| t.time_signature) {
        match score.try_time_signature(num, den) {
            Ok(s) => score = s,
            Err(e) => {
                warnings.push(format!(
                    "time signature {num}/{den} not usable ({e}); kept 4/4"
                ));
                score = Score::try_new(sample_rate, Ppq(u32::from(division)))?;
            }
        }
    }

    // Tempo map: all tempo events from all tracks, merged and sorted.
    // (`try_tempo` replaces on equal ticks, so a later duplicate wins —
    // matching MIDI's last-event-at-a-tick semantics.)
    let mut tempos: Vec<(u64, u32)> = raw_tracks.iter().flat_map(|t| t.tempos.clone()).collect();
    tempos.sort_by_key(|&(tick, _)| tick);
    if tempos.is_empty() {
        warnings.push("no tempo events; using MIDI's default 120 BPM".to_owned());
    }
    for (tick, us_per_quarter) in tempos {
        if us_per_quarter == 0 {
            warnings.push(format!(
                "tempo event at tick {tick} of 0 µs/quarter skipped"
            ));
            continue;
        }
        let bpm = 60_000_000.0 / f64::from(us_per_quarter);
        score = score.try_tempo(Ticks(tick), Bpm(bpm))?;
    }

    // Tracks: one cochlea track per (MIDI track, channel) with notes.
    let mut used_names: Vec<String> = Vec::new();
    for raw in &raw_tracks {
        for skipped in &raw.skipped {
            warnings.push(format!(
                "track {}: {skipped} have no score model; skipped",
                raw.index
            ));
        }
        let mut channels: Vec<u8> = raw.notes.iter().map(|n| n.channel).collect();
        channels.sort_unstable();
        channels.dedup();

        for channel in channels {
            let notes: Vec<&RawNote> = raw.notes.iter().filter(|n| n.channel == channel).collect();
            if channel == 9 {
                score = assemble_percussion(score, raw, &notes, &mut used_names, &mut warnings)?;
                continue;
            }

            let preset = match raw.programs.get(&channel) {
                Some(&p) => program_preset(p),
                None => "sine",
            };
            let base = raw
                .name
                .clone()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| format!("track{}_ch{}", raw.index, channel + 1));
            let name = unique_name(&base, &mut used_names);
            warnings.push(format!(
                "track {:?}: mapped to preset {preset:?} — re-voice to taste",
                name
            ));
            score = score.try_track(&name, Instrument::preset(preset))?;
            for n in notes {
                score = score.try_note(
                    &name,
                    Ticks(n.at),
                    Dur::ticks(n.dur),
                    Pitch(n.key),
                    Vel(n.vel.max(1)),
                )?;
            }
        }
    }

    if score.tracks().is_empty() {
        warnings.push("no note events found; the score has no tracks".to_owned());
    }

    Ok(MidiImport { score, warnings })
}

/// Channel-10 notes split into kick/snare/hat tracks by GM key.
fn assemble_percussion(
    mut score: Score,
    raw: &RawTrack,
    notes: &[&RawNote],
    used_names: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<Score, ScoreError> {
    // preset -> (track name, notes)
    let mut by_preset: BTreeMap<&'static str, Vec<(&RawNote, Pitch)>> = BTreeMap::new();
    for n in notes {
        let (preset, pitch) = percussion_voice(n.key);
        by_preset.entry(preset).or_default().push((n, pitch));
    }
    for (preset, hits) in by_preset {
        let name = unique_name(preset, used_names);
        warnings.push(format!(
            "track {}: channel 10 percussion -> {:?} ({} hits)",
            raw.index,
            name,
            hits.len()
        ));
        score = score.try_track(&name, Instrument::preset(preset))?;
        for (n, pitch) in hits {
            score = score.try_note(
                &name,
                Ticks(n.at),
                Dur::ticks(n.dur),
                pitch,
                Vel(n.vel.max(1)),
            )?;
        }
    }
    Ok(score)
}

/// `base`, or `base_2`, `base_3`, ... — track names are unique per score.
fn unique_name(base: &str, used: &mut Vec<String>) -> String {
    let mut name = base.to_owned();
    let mut i = 2;
    while used.contains(&name) {
        name = format!("{base}_{i}");
        i += 1;
    }
    used.push(name.clone());
    name
}
