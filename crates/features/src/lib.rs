//! Feature extraction over PCM: loudness (ebur128), onsets (spectral flux),
//! pitch (YIN), chroma/key (Krumhansl-Schmuckler), silence/tail, clipping.
//! One schema-versioned JSON report. Works on arbitrary WAVs — this crate
//! depends on neither the score IR nor the synth.
