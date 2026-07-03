//! Key estimation: a 12-bin chroma vector built from STFT magnitude,
//! correlated against the 24 rotated Krumhansl-Kessler major/minor
//! profiles (Krumhansl & Schmuckler's key-finding algorithm).
//!
//! Chroma needs frequency resolution the onset detector's 1024-point STFT
//! doesn't have: at 48 kHz that's ~46.9 Hz/bin, and a Hann window's main
//! lobe is about 4 bins wide, i.e. ~187 Hz — at C4 (261.6 Hz) that's *more
//! than half an octave* of smearing, easily enough to correlate the wrong
//! tonic. Semitone width in Hz grows with frequency (pitch is logarithmic,
//! FFT bins are linear), so this hurts low octaves most. Chroma therefore
//! runs its own larger-window STFT (`FFT_SIZE = 8192`, ~171 ms, ~5.9 Hz/bin
//! — under a quarter-semitone even at C2) rather than reusing the onset
//! transform.

use crate::report::{KeyReport, Mode, PitchClass};
use crate::stft::Stft;

/// FFT size in samples: large, for low-octave frequency resolution (see
/// the module docs). At 48 kHz, ~171 ms / ~5.9 Hz per bin.
const FFT_SIZE: usize = 8192;
/// Hop size in samples (75% overlap at `FFT_SIZE = 8192`).
const HOP: usize = 2048;

/// Ignore STFT bins outside this range when building chroma: below is
/// mostly sub-bass/DC leakage, above is thin, aliased-into-pitch-class
/// harmonic content that mostly adds noise to the chroma weighting.
const CHROMA_MIN_HZ: f64 = 55.0;
const CHROMA_MAX_HZ: f64 = 5000.0;

/// Krumhansl-Kessler major-key profile, index 0 = tonic.
const MAJOR_PROFILE: [f64; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
/// Krumhansl-Kessler minor-key profile, index 0 = tonic.
const MINOR_PROFILE: [f64; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

pub(crate) fn analyze(mono: &[f32], sample_rate: u32) -> KeyReport {
    let stft = Stft::compute(mono, sample_rate, FFT_SIZE, HOP);
    let chroma = chroma_vector(&stft);

    let mut best_tonic = PitchClass::C;
    let mut best_mode = Mode::Major;
    let mut best_corr = f64::MIN;
    for (t, &tonic) in PitchClass::ALL.iter().enumerate() {
        let major_corr = pearson(&chroma, &rotate(&MAJOR_PROFILE, t));
        if major_corr > best_corr {
            best_corr = major_corr;
            best_tonic = tonic;
            best_mode = Mode::Major;
        }
        let minor_corr = pearson(&chroma, &rotate(&MINOR_PROFILE, t));
        if minor_corr > best_corr {
            best_corr = minor_corr;
            best_tonic = tonic;
            best_mode = Mode::Minor;
        }
    }

    KeyReport {
        tonic: best_tonic,
        mode: best_mode,
        confidence: best_corr,
        chroma,
    }
}

/// Accumulate STFT-magnitude energy into 12 pitch-class bins by mapping
/// each in-range bin's center frequency to the nearest equal-tempered
/// pitch class (`libm::log2`), then normalize so the max bin is `1.0`.
fn chroma_vector(stft: &Stft) -> [f64; 12] {
    let mut raw = [0.0f64; 12];
    for frame in &stft.magnitudes {
        for (bin, &mag) in frame.iter().enumerate() {
            let hz = stft.bin_hz(bin);
            if !(CHROMA_MIN_HZ..=CHROMA_MAX_HZ).contains(&hz) {
                continue;
            }
            let midi_float = 69.0 + 12.0 * libm::log2(hz / 440.0);
            let pitch_class = midi_float.round().rem_euclid(12.0) as usize;
            raw[pitch_class] += f64::from(mag);
        }
    }
    let max = raw.iter().copied().fold(0.0f64, f64::max);
    if max > 0.0 {
        for v in &mut raw {
            *v /= max;
        }
    }
    raw
}

/// `rotate(profile, t)[i] = profile[(i - t) mod 12]`: the profile as it
/// would read if the tonic were pitch class `t` (chroma index `t`)
/// instead of `profile`'s native tonic at index 0.
fn rotate(profile: &[f64; 12], t: usize) -> [f64; 12] {
    let mut out = [0.0; 12];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = profile[(i + 12 - t) % 12];
    }
    out
}

/// Pearson correlation coefficient of two 12-element vectors. `0.0` if
/// either has zero variance (undefined correlation, but a safe default —
/// never triggered for the fixed, non-constant KK profiles, only possible
/// for an all-zero chroma vector).
fn pearson(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let n = a.len() as f64;
    let mean_a = a.iter().sum::<f64>() / n;
    let mean_b = b.iter().sum::<f64>() / n;

    let mut cov = 0.0;
    let mut var_a = 0.0;
    let mut var_b = 0.0;
    for i in 0..a.len() {
        let da = a[i] - mean_a;
        let db = b[i] - mean_b;
        cov += da * db;
        var_a += da * da;
        var_b += db * db;
    }

    if var_a <= 0.0 || var_b <= 0.0 {
        return 0.0;
    }
    cov / (var_a.sqrt() * var_b.sqrt())
}
