//! The `cochlea` binary: `render`, `probe`, `lint`, `spectro`.
//!
//! Exit codes: 0 ok, 1 verify/lint failures, 2 usage/IO/render errors
//! (clap and anyhow errors both land on 2 via the wrapper in `main`).

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use cochlea_score::{Score, Severity};
use cochlea_synth::PatchBank;
use cochlea_verify::VerifyExt;

#[derive(Parser)]
#[command(
    name = "cochlea",
    version,
    about = "Headless audio engine for agents: compose, render, listen through numbers"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render a RON score to a WAV mix (and optionally per-track stems).
    Render {
        /// The score (RON data form, version 1).
        score: PathBuf,
        /// Output WAV path (32-bit float stereo).
        #[arg(long)]
        out: PathBuf,
        /// Also write one WAV per track into this directory.
        #[arg(long)]
        stems: Option<PathBuf>,
        /// Run the score's embedded `verify:` assertions; exit 1 on failure.
        #[arg(long)]
        verify: bool,
        /// Write the verify report JSON here instead of stdout.
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Extract the feature report (and optionally a spectrogram) from a WAV
    /// — works on arbitrary WAVs, no score needed.
    Probe {
        /// Input WAV (f32 or 16/24/32-bit PCM).
        input: PathBuf,
        /// Write the JSON report here instead of stdout.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Also render a mel spectrogram PNG here.
        #[arg(long)]
        spectro: Option<PathBuf>,
    },
    /// Statically validate a score against the preset catalog.
    Lint {
        /// The score (RON data form, version 1).
        score: PathBuf,
    },
    /// Render a mel spectrogram (or tiled contact sheet) from a WAV.
    Spectro {
        /// Input WAV.
        input: PathBuf,
        /// Output PNG path.
        #[arg(long)]
        out: PathBuf,
        /// Tile the piece into a contact sheet instead of one long strip.
        #[arg(long)]
        sheet: bool,
        /// Sections per tile when `--sheet` is set (time slices; bar-aware
        /// tiling uses markers, which need score context via `render`).
        #[arg(long, default_value_t = 8)]
        bars_per_tile: usize,
    },
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("cochlea: {err:#}");
            std::process::ExitCode::from(2)
        }
    }
}

fn load_score(path: &Path) -> anyhow::Result<Score> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Score::from_ron(&text).with_context(|| format!("parsing {}", path.display()))
}

fn run() -> anyhow::Result<std::process::ExitCode> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Render {
            score,
            out,
            stems,
            verify,
            report,
        } => {
            let score = load_score(&score)?;
            let rendered = cochlea_render::render(&score)?;
            rendered
                .write_wav(&out)
                .with_context(|| format!("writing {}", out.display()))?;
            eprintln!(
                "rendered {} frames at {} Hz -> {}",
                rendered.frames(),
                rendered.sample_rate().0,
                out.display()
            );
            if let Some(dir) = stems {
                rendered
                    .write_stems(&dir)
                    .with_context(|| format!("writing stems to {}", dir.display()))?;
            }
            if verify {
                let result = rendered
                    .verify(&score)
                    .with_specs(score.verify_specs())
                    .run();
                let text = serde_json::to_string_pretty(&result)?;
                match report {
                    Some(path) => std::fs::write(&path, text)
                        .with_context(|| format!("writing {}", path.display()))?,
                    None => println!("{text}"),
                }
                if !result.passed {
                    return Ok(std::process::ExitCode::from(1));
                }
            }
            Ok(std::process::ExitCode::SUCCESS)
        }

        Cmd::Probe {
            input,
            json,
            spectro,
        } => {
            let audio = cochlea_features::Audio::from_wav(&input)
                .with_context(|| format!("reading {}", input.display()))?;
            let report = cochlea_features::probe(&audio, &cochlea_features::ProbeOpts::default());
            let text = serde_json::to_string_pretty(&report)?;
            match json {
                Some(path) => std::fs::write(&path, text)
                    .with_context(|| format!("writing {}", path.display()))?,
                None => println!("{text}"),
            }
            if let Some(path) = spectro {
                write_spectro(&audio, &path, false, 0)?;
            }
            Ok(std::process::ExitCode::SUCCESS)
        }

        Cmd::Lint { score } => {
            let score = load_score(&score)?;
            let findings = score.validate(&PatchBank::presets());
            println!("{}", serde_json::to_string_pretty(&findings)?);
            let errors = findings
                .iter()
                .filter(|f| f.severity == Severity::Error)
                .count();
            if errors > 0 {
                eprintln!("{errors} error(s), {} finding(s) total", findings.len());
                return Ok(std::process::ExitCode::from(1));
            }
            Ok(std::process::ExitCode::SUCCESS)
        }

        Cmd::Spectro {
            input,
            out,
            sheet,
            bars_per_tile,
        } => {
            let audio = cochlea_features::Audio::from_wav(&input)
                .with_context(|| format!("reading {}", input.display()))?;
            write_spectro(&audio, &out, sheet, bars_per_tile)?;
            Ok(std::process::ExitCode::SUCCESS)
        }
    }
}

fn write_spectro(
    audio: &cochlea_features::Audio,
    path: &Path,
    sheet: bool,
    per_tile: usize,
) -> anyhow::Result<()> {
    let spec = cochlea_spectro::mel_spectrogram(
        &audio.samples,
        audio.channels,
        audio.sample_rate,
        &cochlea_spectro::SpectroOpts::new(),
    );
    let img = if sheet {
        cochlea_spectro::contact_sheet(&spec, &[], per_tile)
    } else {
        cochlea_spectro::render_png(&spec, &[])
    };
    cochlea_spectro::write_png(&img, path)
        .with_context(|| format!("writing {}", path.display()))?;
    eprintln!("spectrogram -> {}", path.display());
    Ok(())
}
