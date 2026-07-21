//! Offline block render engine: 64-sample blocks split at event boundaries,
//! pure voice scheduling (allocation and oldest-note stealing are functions
//! of the event schedule alone), per-track rendering as the parallelism
//! unit and free stems export, f64 master sum in fixed track order, and
//! 32-bit float WAV output via hound.
//!
//! ```
//! use cochlea_score::*;
//! let score = Score::new(SampleRate(48_000), Ppq(960))
//!     .track("lead", Instrument::preset("sine"))
//!     .note("lead", bar(1), Dur::quarter(), Pitch::A4, Vel(96));
//! let rendered = cochlea_render::render(&score).unwrap();
//! assert!(rendered.mix().iter().any(|&s| s != 0.0));
//! ```

mod engine;
mod error;
mod master;
mod schedule;

use std::path::Path;

use cochlea_score::{SampleRate, Score};
use cochlea_synth::PatchBank;

pub use error::RenderError;

/// A completed render: per-track stems plus the mix, all interleaved
/// stereo f32 at the score's sample rate.
///
/// The mix is *defined* as the f64 sum of the stems' stored f32 values in
/// fixed track order, passed through the score's master stage (gain +
/// limiter — a no-op for the default master), converted back to f32. For
/// a score without a master section the invariant "mix == sum of stems"
/// therefore holds byte-for-byte exactly as before, and is tested; with a
/// master, the mix is `master(Σ stems)` and the stems stay pre-master.
pub struct Rendered {
    sample_rate: SampleRate,
    stems: Vec<(String, Vec<f32>)>,
    mix: Vec<f32>,
}

impl Rendered {
    /// The stereo mix, interleaved L/R.
    pub fn mix(&self) -> &[f32] {
        &self.mix
    }

    /// Per-track stems in score track order, interleaved L/R, all the same
    /// length as the mix.
    pub fn stems(&self) -> impl Iterator<Item = (&str, &[f32])> {
        self.stems.iter().map(|(n, s)| (n.as_str(), s.as_slice()))
    }

    /// One track's stem by name.
    pub fn stem(&self, track: &str) -> Option<&[f32]> {
        self.stems
            .iter()
            .find(|(n, _)| n == track)
            .map(|(_, s)| s.as_slice())
    }

    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// Render length in frames (samples per channel).
    pub fn frames(&self) -> u64 {
        (self.mix.len() / 2) as u64
    }

    /// Writes the mix as a 32-bit float stereo WAV.
    pub fn write_wav(&self, path: impl AsRef<Path>) -> Result<(), RenderError> {
        write_wav(path.as_ref(), self.sample_rate, &self.mix)
    }

    /// Writes one `<track>.wav` per stem into `dir` (created if missing).
    pub fn write_stems(&self, dir: impl AsRef<Path>) -> Result<(), RenderError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        for (name, stem) in &self.stems {
            write_wav(&dir.join(format!("{name}.wav")), self.sample_rate, stem)?;
        }
        Ok(())
    }
}

fn write_wav(path: &Path, sample_rate: SampleRate, samples: &[f32]) -> Result<(), RenderError> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: sample_rate.0,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    Ok(())
}

fn render_inner(score: &Score, bank: &PatchBank, parallel: bool) -> Result<Rendered, RenderError> {
    let schedule = schedule::compile(score, bank)?;
    let stems = engine::render_stems(&schedule, score, parallel);
    let mut mix64 = engine::sum_stems_f64(&stems, schedule.total_samples);
    master::process(&mut mix64, schedule.sample_rate, score.master());
    let mix = engine::quantize(&mix64);
    Ok(Rendered {
        sample_rate: schedule.sample_rate,
        stems: schedule
            .tracks
            .iter()
            .map(|t| t.name.clone())
            .zip(stems)
            .collect(),
        mix,
    })
}

/// Renders a score with the shipped presets, tracks in parallel.
pub fn render(score: &Score) -> Result<Rendered, RenderError> {
    render_inner(score, &PatchBank::presets(), true)
}

/// Renders with a custom [`PatchBank`] (the `Instrument::custom` path).
pub fn render_with(score: &Score, bank: &PatchBank) -> Result<Rendered, RenderError> {
    render_inner(score, bank, true)
}

/// Single-threaded render — exists so the determinism test can assert
/// `parallel == serial` byte-for-byte, and for callers that want to bound
/// CPU use.
pub fn render_serial(score: &Score) -> Result<Rendered, RenderError> {
    render_inner(score, &PatchBank::presets(), false)
}
