//! Score IR: integer-tick timebase, tempo map, bar/beat math, tracks, notes,
//! automation, and the RON data form (version 1).
//!
//! Time is `u64` ticks (960 PPQ default); tick→sample conversion is exact
//! rational arithmetic through `fenestra_anim::mul_div`, applied once at
//! event-schedule time with the rounding rules of `docs/determinism.md`.
//! Automation easing comes from fenestra-anim (linear/hold/bezier in v1 —
//! all pure arithmetic, bit-deterministic across platforms).
//!
//! ```
//! use cochlea_score::*;
//!
//! let score = Score::new(SampleRate(48_000), Ppq(960))
//!     .time_signature(4, 4)
//!     .tempo(Ticks(0), Bpm(120.0))
//!     .track("lead", Instrument::preset("saw_lead"))
//!     .note("lead", bar(1).beat(1), Dur::quarter(), Pitch::A4, Vel(96))
//!     .automate("lead", Param::CUTOFF_HZ,
//!         keys![(bar(1), 400.0, ease_in_out()), (bar(3), 4_000.0)]);
//! assert_eq!(score.tempo_map().sample_at(Ticks(960)), 24_000); // 1 quarter at 120 BPM
//! ```

mod data;
mod error;
mod midi;
mod midi_export;
mod param;
mod pitch;
mod reference;
mod score;
mod tempo;
mod time;
mod transcribe;
mod validate;
mod verify_spec;

pub use data::DATA_VERSION;
pub use error::ScoreError;
pub use midi::{MidiImport, import_midi};
pub use midi_export::export_midi;
pub use param::Param;
pub use pitch::Pitch;
pub use reference::authoring_reference;
pub use score::{
    AutoKey, Automation, EaseSpec, Insert, Instrument, KeyDef, Limiter, Master, Note, Score, Track,
};
pub use tempo::TempoMap;
pub use time::{Bpm, Dur, Pos, Ppq, SampleRate, Ticks, TimeSignature, Vel, bar};
pub use transcribe::{NoteObservation, TranscribeOpts, Transcription, VEL_FLOOR_DBFS, transcribe};
pub use validate::{Catalog, InstrumentInfo, LintFinding, ParamInfo, Polyphony, Severity};
pub use verify_spec::{MonotoneDir, VerifySpec};

// Easing vocabulary re-exported from fenestra-anim so score authors need one
// import. Springs are re-exported too — validation rejects them on
// automation with an explanation, which beats a missing symbol.
pub use fenestra_anim::{Ease, ease_in, ease_in_out, ease_out, hold, linear, spring};

/// Builds a `Vec<KeyDef>` for [`Score::automate`]: each entry is
/// `(position, value)` with an optional easing third element.
///
/// ```
/// use cochlea_score::*;
/// let keys = keys![(bar(1), 400.0, ease_in_out()), (bar(3), 4_000.0)];
/// assert_eq!(keys.len(), 2);
/// ```
#[macro_export]
macro_rules! keys {
    ($(($pos:expr, $value:expr $(, $ease:expr)? )),+ $(,)?) => {
        ::std::vec![ $( $crate::KeyDef::new($pos, $value, $crate::keys!(@ease $($ease)?)) ),+ ]
    };
    (@ease) => { $crate::linear() };
    (@ease $e:expr) => { $e };
}
