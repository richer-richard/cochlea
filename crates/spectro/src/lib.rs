//! Mel spectrogram rendering: STFT (rustfft), hand-rolled mel filterbank,
//! log magnitude with dB floor, viridis LUT, time ruler, optional bar-marker
//! grid, PNG out; tiled section contact sheets for whole-piece review in one
//! vision call. Depends on neither the score IR nor the synth.
//!
//! ## Determinism
//!
//! Every function here is a pure function of its in-memory arguments (the
//! only fallible, side-effecting operation is [`write_png`], which touches
//! the filesystem). Transcendentals in the analysis path go through `libm`
//! exclusively (`cochlea-spectro` mirrors the workspace-wide rule in
//! `docs/determinism.md`); `rustfft::FftPlannerScalar` is used explicitly so
//! FFT codegen never depends on runtime CPU-feature dispatch
//! (`rustfft::FftPlanner::new` is banned via `clippy.toml`).
//!
//! ## Design choices, documented once here
//!
//! - **Mel formula**: HTK (`mel = 2595 * log10(1 + f/700)`), not Slaney.
//!   Filters are peak-normalized triangles (weight 1.0 at each band's
//!   center bin), not Slaney's area-normalized ones. See
//!   [`mel_spectrogram`] for the full rationale.
//! - **Window**: periodic (DFT-even) Hann, `0.5 - 0.5*cos(2*pi*n/fft)` —
//!   the STFT here is analysis-only (no overlap-add reconstruction), so
//!   periodic vs symmetric doesn't matter for correctness, but periodic is
//!   the conventional choice (matches librosa/scipy `periodic=True`).
//! - **Framing**: frame `i` covers samples `[i*hop, i*hop+fft)` of the mono
//!   downmix, zero-padded past the end — no reflection/centering padding.
//!   Enough trailing all-zero frames are appended past the last real sample
//!   that some frames are *guaranteed* to be pure padding, so "silence
//!   after the signal ends" is a deterministic, testable floor rather than
//!   an edge-effect artifact. See [`mel_spectrogram`].
//! - **Viridis LUT**: the standard 256-entry matplotlib viridis anchor
//!   table (`lib/matplotlib/_cm_listed.py::_viridis_data`), not
//!   reconstructed or approximated. See the `viridis` module (crate-private,
//!   backs [`render_png`]/[`contact_sheet`]).
//! - **Ruler labels**: a hand-designed 3x5 pixel bitmap digit font (`0`-`9`
//!   only; not derived from any standard typeface) rather than dropping
//!   text entirely, so contact sheets/spectrograms are self-describing in a
//!   single vision call without an accompanying legend. See [`render_png`].

mod diff;
mod error;
mod font;
mod marker;
mod mel;
mod opts;
mod render;
mod viridis;

pub use diff::{DiffResult, diff_images};
pub use error::SpectroError;
pub use marker::Marker;
pub use mel::{MelSpec, mel_spectrogram};
pub use opts::SpectroOpts;
pub use render::{CONTACT_SHEET_GUTTER_PX, RULER_HEIGHT_PX, contact_sheet, render_png, write_png};
