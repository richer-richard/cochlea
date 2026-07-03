//! Mel spectrogram rendering: STFT (rustfft), hand-rolled mel filterbank,
//! log magnitude with dB floor, viridis LUT, time ruler, optional bar-marker
//! grid, PNG out; tiled section contact sheets for whole-piece review in one
//! vision call. Depends on neither the score IR nor the synth.
