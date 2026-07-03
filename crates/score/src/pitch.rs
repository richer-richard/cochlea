//! MIDI pitch: a `u8` note number with musical names on both ends
//! (`Pitch::A4`, `"C#3".parse()`, `Display` in sharps).

use std::str::FromStr;

use crate::error::ScoreError;

/// A MIDI note number (0..=127). `Pitch::A4` is 69 (440 Hz); middle C
/// (`Pitch::C4`) is 60. Named constants cover octaves 0–8; anything else
/// parses from strings like `"Bb-1"` or constructs directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pitch(pub u8);

/// Declares the twelve pitch constants of one octave.
macro_rules! octave {
    ($base:literal : $c:ident $cs:ident $d:ident $ds:ident $e:ident $f:ident
     $fs:ident $g:ident $gs:ident $a:ident $as_:ident $b:ident) => {
        pub const $c: Pitch = Pitch($base);
        pub const $cs: Pitch = Pitch($base + 1);
        pub const $d: Pitch = Pitch($base + 2);
        pub const $ds: Pitch = Pitch($base + 3);
        pub const $e: Pitch = Pitch($base + 4);
        pub const $f: Pitch = Pitch($base + 5);
        pub const $fs: Pitch = Pitch($base + 6);
        pub const $g: Pitch = Pitch($base + 7);
        pub const $gs: Pitch = Pitch($base + 8);
        pub const $a: Pitch = Pitch($base + 9);
        pub const $as_: Pitch = Pitch($base + 10);
        pub const $b: Pitch = Pitch($base + 11);
    };
}

impl Pitch {
    octave!(12: C0 CS0 D0 DS0 E0 F0 FS0 G0 GS0 A0 AS0 B0);
    octave!(24: C1 CS1 D1 DS1 E1 F1 FS1 G1 GS1 A1 AS1 B1);
    octave!(36: C2 CS2 D2 DS2 E2 F2 FS2 G2 GS2 A2 AS2 B2);
    octave!(48: C3 CS3 D3 DS3 E3 F3 FS3 G3 GS3 A3 AS3 B3);
    octave!(60: C4 CS4 D4 DS4 E4 F4 FS4 G4 GS4 A4 AS4 B4);
    octave!(72: C5 CS5 D5 DS5 E5 F5 FS5 G5 GS5 A5 AS5 B5);
    octave!(84: C6 CS6 D6 DS6 E6 F6 FS6 G6 GS6 A6 AS6 B6);
    octave!(96: C7 CS7 D7 DS7 E7 F7 FS7 G7 GS7 A7 AS7 B7);
    octave!(108: C8 CS8 D8 DS8 E8 F8 FS8 G8 GS8 A8 AS8 B8);

    /// Equal-temperament frequency, A4 = 440 Hz. Uses libm (determinism
    /// contract), though this only runs at voice-construction time.
    pub fn hz(self) -> f64 {
        440.0 * libm::exp2((f64::from(self.0) - 69.0) / 12.0)
    }
}

impl FromStr for Pitch {
    type Err = ScoreError;

    /// Parses `"A4"`, `"C#3"`, `"Bb2"`, `"G-1"` (octave −1 is MIDI 0–11).
    fn from_str(s: &str) -> Result<Pitch, ScoreError> {
        let bad = || ScoreError::BadPitch(s.to_owned());
        let mut chars = s.chars();
        let letter = chars.next().ok_or_else(bad)?;
        let semitone: i32 = match letter.to_ascii_uppercase() {
            'C' => 0,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => return Err(bad()),
        };
        let rest = chars.as_str();
        let (accidental, octave_str) = match rest.chars().next() {
            Some('#') => (1, &rest[1..]),
            Some('b') => (-1, &rest[1..]),
            _ => (0, rest),
        };
        let octave: i32 = octave_str.parse().map_err(|_| bad())?;
        let midi = (octave + 1) * 12 + semitone + accidental;
        u8::try_from(midi)
            .ok()
            .filter(|&m| m <= 127)
            .map(Pitch)
            .ok_or(ScoreError::PitchOutOfRange(midi))
    }
}

impl std::fmt::Display for Pitch {
    /// Sharps-only spelling (`"C#4"`), octave −1 for MIDI 0–11 — always
    /// re-parseable by `FromStr`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const NAMES: [&str; 12] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        let octave = i32::from(self.0) / 12 - 1;
        write!(f, "{}{}", NAMES[usize::from(self.0 % 12)], octave)
    }
}
