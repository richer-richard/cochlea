//! Tiny crate-private helpers shared by the text renderers (`digest`,
//! `compare`) — one implementation each, instead of per-module copies that
//! drift.

use crate::report::Mode;

/// Maximum of an f64 iterator, `None` when empty. (NaN never reaches these
/// call sites: ingestion rejects non-finite samples, and every producer of
/// the folded values guards its math.)
pub(crate) fn max_of(values: impl Iterator<Item = f64>) -> Option<f64> {
    values.fold(None, |acc: Option<f64>, v| {
        Some(acc.map_or(v, |m| m.max(v)))
    })
}

/// The digest/compare wire spelling of [`Mode`].
pub(crate) fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Major => "major",
        Mode::Minor => "minor",
    }
}

/// Note name + octave for a MIDI note number (`60` -> `"C4"`, MIDI's
/// standard octave numbering where middle C is C4). Shared by the digest
/// renderer and the melody extractor.
pub(crate) fn note_name(midi: i32) -> String {
    let pc = crate::report::PitchClass::ALL[midi.rem_euclid(12) as usize];
    let octave = midi.div_euclid(12) - 1;
    format!("{}{octave}", pc.name())
}
