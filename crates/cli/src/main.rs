//! The `cochlea` binary. `probe` ships in P3; `render`, `lint`, and
//! `spectro` complete the set in P4 (docs/plan.md).
//!
//! Exit codes: 0 ok, 1 verify/lint failures, 2 usage/IO errors (clap and
//! anyhow errors both land on 2 via the wrapper in `main`).

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};

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

fn run() -> anyhow::Result<std::process::ExitCode> {
    let cli = Cli::parse();
    match cli.cmd {
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
                Some(path) => {
                    std::fs::write(&path, text)
                        .with_context(|| format!("writing {}", path.display()))?;
                }
                None => println!("{text}"),
            }
            if let Some(path) = spectro {
                let spec = cochlea_spectro::mel_spectrogram(
                    &audio.samples,
                    audio.channels,
                    audio.sample_rate,
                    &cochlea_spectro::SpectroOpts::new(),
                );
                let img = cochlea_spectro::render_png(&spec, &[]);
                cochlea_spectro::write_png(&img, &path)
                    .with_context(|| format!("writing {}", path.display()))?;
            }
            Ok(std::process::ExitCode::SUCCESS)
        }
    }
}
