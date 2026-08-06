//! Integer-tick ground truth: ticks, PPQ, sample rate, tempo, durations, and
//! bar/beat positions. Wall-clock seconds never appear here — time is `u64`
//! ticks, and everything else is derived from them exactly (rational
//! arithmetic) at well-defined points.

use crate::error::ScoreError;

/// A tick count or tick index — the timeline's ground truth. At the default
/// 960 PPQ a quarter note is 960 ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ticks(pub u64);

impl Ticks {
    /// Tick zero — the start of the score.
    pub const ZERO: Ticks = Ticks(0);

    /// The largest tick an authored position (note, tempo change, or
    /// automation key) may resolve to. `2^32` ticks.
    ///
    /// This is the domain bound that keeps the exact rational tick→sample
    /// and tick→nanosecond arithmetic in `tempo.rs` from ever overflowing
    /// `u64`. The worst case is the slowest tempo (1 BPM ⇒ 6·10¹⁰ ns/quarter)
    /// at the coarsest grid (24 PPQ): `MAX · 6e10 / 24 ≈ 1.07·10¹⁹` ns, still
    /// under `u64::MAX ≈ 1.84·10¹⁹`. It also comfortably exceeds the most
    /// ticks a one-hour render can contain (~3.69·10⁹ at 4000 BPM / 15360
    /// PPQ), so no renderable score is ever refused for being *authored* too
    /// far out — anything between here and the one-hour cap is caught cleanly
    /// at render time instead. Positions past this bound are refused at
    /// authoring/load time with [`ScoreError::PositionTooFar`]; before this
    /// bound existed, a crafted far-future tempo tick reached unchecked
    /// `mul_div` and panicked the renderer (adversarial review, Finding 1).
    pub const MAX: Ticks = Ticks(1 << 32);
}

impl std::ops::Add for Ticks {
    type Output = Ticks;
    fn add(self, rhs: Ticks) -> Ticks {
        Ticks(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Ticks {
    type Output = Ticks;
    fn sub(self, rhs: Ticks) -> Ticks {
        Ticks(self.0 - rhs.0)
    }
}

impl std::fmt::Display for Ticks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Pulses (ticks) per quarter note. 960 is the workspace default; the valid
/// range is 24..=15_360.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ppq(pub u32);

impl Ppq {
    pub(crate) const MIN: u32 = 24;
    pub(crate) const MAX: u32 = 15_360;

    /// Ticks in a whole note (`ppq * 4`).
    pub(crate) fn whole_note_ticks(self) -> u64 {
        u64::from(self.0) * 4
    }
}

/// Output sample rate in Hz. Valid range 8_000..=192_000 (the bounds keep
/// the rational tick→sample math comfortably inside `u64`, see `TempoMap`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleRate(pub u32);

impl SampleRate {
    pub(crate) const MIN: u32 = 8_000;
    pub(crate) const MAX: u32 = 192_000;
}

/// Beats per minute, an *authoring* unit only. Stored scores convert once to
/// integer nanoseconds-per-quarter-note (`round`), after which all timing is
/// exact rational arithmetic — see `docs/determinism.md`, rounding rule 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bpm(pub f64);

impl Bpm {
    pub(crate) const MIN: f64 = 1.0;
    pub(crate) const MAX: f64 = 4_000.0;

    /// Is this a tempo the score IR accepts (finite, `1..=4000`)?
    ///
    /// The single rule, so a front end can reject `--bpm` at the flag
    /// boundary instead of failing deep inside score assembly — and so the
    /// bounds live in exactly one place.
    pub fn validate(self) -> Result<(), ScoreError> {
        if !self.0.is_finite() || self.0 < Self::MIN || self.0 > Self::MAX {
            return Err(ScoreError::OutOfRange {
                what: "bpm",
                value: self.0,
                min: Self::MIN,
                max: Self::MAX,
            });
        }
        Ok(())
    }

    /// The one authoring-time rounding: BPM to integer ns per quarter note.
    /// 120 BPM → exactly 500_000_000 ns.
    pub(crate) fn nanos_per_quarter(self) -> Result<u64, ScoreError> {
        self.validate()?;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "bounded to [1, 4000] BPM above, so the quotient is in [1.5e7, 6e10]"
        )]
        Ok((60_000_000_000.0 / self.0).round() as u64)
    }
}

/// The score's (single, v1) time signature: `beats` per bar, a `unit`-th
/// note per beat. 4/4 is `beats: 4, unit: 4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSignature {
    pub beats: u32,
    pub unit: u32,
}

impl TimeSignature {
    pub(crate) fn validate(self, ppq: Ppq) -> Result<(), ScoreError> {
        if self.beats == 0 || !matches!(self.unit, 1 | 2 | 4 | 8 | 16 | 32) {
            return Err(ScoreError::OutOfRange {
                what: "time signature",
                value: f64::from(self.beats),
                min: 1.0,
                max: 32.0,
            });
        }
        if !ppq.whole_note_ticks().is_multiple_of(u64::from(self.unit)) {
            return Err(ScoreError::BadTimeSignatureUnit {
                unit: self.unit,
                ppq: ppq.0,
            });
        }
        Ok(())
    }

    /// Ticks per beat at `ppq` — exact by `validate`.
    pub(crate) fn ticks_per_beat(self, ppq: Ppq) -> u64 {
        ppq.whole_note_ticks() / u64::from(self.unit)
    }

    /// Ticks per bar at `ppq`.
    pub(crate) fn ticks_per_bar(self, ppq: Ppq) -> u64 {
        self.ticks_per_beat(ppq) * u64::from(self.beats)
    }
}

/// MIDI velocity, 1..=127. Zero is rejected at validation — MIDI reserves
/// it for note-off, so a `Vel(0)` note is always an authoring mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Vel(pub u8);

/// A musical duration. Either an exact fraction of a whole note
/// (`Dur::quarter()` = 1/4, chainable `.dotted()` / `.triplet()`) or a raw
/// tick count. Fractions resolve against PPQ *exactly*: a duration that
/// lands between ticks is an error, never a rounding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dur(DurKind);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DurKind {
    /// `num`/`den` of a whole note.
    Frac { num: u32, den: u32 },
    /// A raw tick count (the escape hatch).
    Ticks(u64),
}

impl Dur {
    /// An exact `num`/`den` fraction of a whole note.
    pub fn of(num: u32, den: u32) -> Dur {
        assert!(num > 0 && den > 0, "a duration must be positive");
        Dur(DurKind::Frac { num, den })
    }

    pub fn whole() -> Dur {
        Dur::of(1, 1)
    }
    pub fn half() -> Dur {
        Dur::of(1, 2)
    }
    pub fn quarter() -> Dur {
        Dur::of(1, 4)
    }
    pub fn eighth() -> Dur {
        Dur::of(1, 8)
    }
    pub fn sixteenth() -> Dur {
        Dur::of(1, 16)
    }
    pub fn thirty_second() -> Dur {
        Dur::of(1, 32)
    }

    /// A raw tick count — for durations off the fraction grid.
    pub fn ticks(n: u64) -> Dur {
        Dur(DurKind::Ticks(n))
    }

    /// Dotted: half again as long (×3/2).
    pub fn dotted(self) -> Dur {
        match self.0 {
            DurKind::Frac { num, den } => Dur(DurKind::Frac {
                num: num * 3,
                den: den * 2,
            }),
            DurKind::Ticks(n) => Dur(DurKind::Ticks(n * 3 / 2)),
        }
    }

    /// Triplet: two-thirds as long (×2/3).
    pub fn triplet(self) -> Dur {
        match self.0 {
            DurKind::Frac { num, den } => Dur(DurKind::Frac {
                num: num * 2,
                den: den * 3,
            }),
            DurKind::Ticks(n) => Dur(DurKind::Ticks(n * 2 / 3)),
        }
    }

    /// Resolves to ticks at `ppq`. Exact or an error.
    pub fn resolve(self, ppq: Ppq) -> Result<Ticks, ScoreError> {
        match self.0 {
            DurKind::Frac { num, den } => {
                let total = ppq.whole_note_ticks() * u64::from(num);
                if !total.is_multiple_of(u64::from(den)) {
                    return Err(ScoreError::NonIntegerTick {
                        num,
                        den,
                        ppq: ppq.0,
                    });
                }
                Ok(Ticks(total / u64::from(den)))
            }
            DurKind::Ticks(n) => Ok(Ticks(n)),
        }
    }

    /// Parses `"1/4"`, `"3/16"`, dotted `"1/4."`, triplet `"1/8t"`.
    pub fn parse(s: &str) -> Result<Dur, ScoreError> {
        let bad = || ScoreError::BadDur(s.to_owned());
        let (body, dotted, triplet) = if let Some(b) = s.strip_suffix('.') {
            (b, true, false)
        } else if let Some(b) = s.strip_suffix('t') {
            (b, false, true)
        } else {
            (s, false, false)
        };
        let (num, den) = body.split_once('/').ok_or_else(bad)?;
        let num: u32 = num.trim().parse().map_err(|_| bad())?;
        let den: u32 = den.trim().parse().map_err(|_| bad())?;
        if num == 0 || den == 0 {
            return Err(bad());
        }
        let mut dur = Dur::of(num, den);
        if dotted {
            dur = dur.dotted();
        }
        if triplet {
            dur = dur.triplet();
        }
        Ok(dur)
    }

    /// The canonical fraction form for serialization: ticks reduced against
    /// the whole note. Dotted/triplet sugar canonicalizes (a dotted quarter
    /// writes as `"3/8"`); parsing accepts either spelling.
    pub(crate) fn fraction_string(ticks: Ticks, ppq: Ppq) -> String {
        let whole = ppq.whole_note_ticks();
        let g = gcd(ticks.0, whole);
        format!("{}/{}", ticks.0 / g, whole / g)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// A bar/beat position, 1-based, with an optional exact sub-beat offset:
/// `bar(1).beat(3).plus(Dur::eighth())`. Raw ticks convert in via
/// `Pos::from(Ticks(..))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos(PosKind);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PosKind {
    Grid {
        bar: u32,
        beat: u32,
        offset: Option<Dur>,
    },
    Raw(Ticks),
}

/// The start of 1-based bar `n` — the terse position constructor.
pub fn bar(n: u32) -> Pos {
    Pos(PosKind::Grid {
        bar: n,
        beat: 1,
        offset: None,
    })
}

impl Pos {
    /// Selects a 1-based beat within the bar.
    ///
    /// # Panics
    /// On a raw-tick position — beats only exist on the bar/beat grid.
    pub fn beat(self, b: u32) -> Pos {
        match self.0 {
            PosKind::Grid { bar, offset, .. } => Pos(PosKind::Grid {
                bar,
                beat: b,
                offset,
            }),
            PosKind::Raw(_) => panic!("Pos::beat on a raw-tick position"),
        }
    }

    /// Offsets the position by an exact duration.
    pub fn plus(self, d: Dur) -> Pos {
        match self.0 {
            PosKind::Grid { bar, beat, offset } => {
                assert!(
                    offset.is_none(),
                    "Pos::plus applied twice; combine the durations"
                );
                Pos(PosKind::Grid {
                    bar,
                    beat,
                    offset: Some(d),
                })
            }
            PosKind::Raw(_) => panic!("Pos::plus on a raw-tick position; add ticks directly"),
        }
    }

    /// Resolves to ticks. Exact or an error.
    pub fn resolve(self, ppq: Ppq, ts: TimeSignature) -> Result<Ticks, ScoreError> {
        match self.0 {
            PosKind::Raw(t) => Ok(t),
            PosKind::Grid { bar, beat, offset } => {
                if bar == 0 || beat == 0 {
                    return Err(ScoreError::ZeroBasedPosition { bar, beat });
                }
                if beat > ts.beats {
                    return Err(ScoreError::BeatOutOfSignature {
                        beat,
                        beats: ts.beats,
                        unit: ts.unit,
                    });
                }
                let tpb = ts.ticks_per_beat(ppq);
                let base = u64::from(bar - 1) * ts.ticks_per_bar(ppq) + u64::from(beat - 1) * tpb;
                let off = match offset {
                    Some(d) => d.resolve(ppq)?.0,
                    None => 0,
                };
                Ok(Ticks(base + off))
            }
        }
    }
}

impl From<Ticks> for Pos {
    fn from(t: Ticks) -> Pos {
        Pos(PosKind::Raw(t))
    }
}
