//! `cochlea-mcp`: an MCP (Model Context Protocol) stdio server exposing
//! cochlea's render/probe/spectro/lint pipeline as agent tools, so any MCP
//! client can compose, render, and "listen" to audio without shelling out
//! or reading PCM directly. Hand-rolled JSON-RPC 2.0 over newline-delimited
//! stdio — no async runtime; offline batch tools need none.
//!
//! stdout carries only protocol response lines; everything else (this
//! banner included) goes to stderr. EOF on stdin is a clean exit.
//!
//! This file is deliberately thin: the framing loop (with its per-line
//! size cap) is `server::serve`, and dispatch is a pure function of
//! strings (`Server::handle_line`) — both tested without a subprocess
//! (`crates/mcp/tests/`).

use cochlea_mcp::server::{Server, serve};

/// The only flag: `--root <dir>` confines every tool's file access to
/// `dir`. No clap — one optional flag doesn't justify the dependency in a
/// binary whose whole interface is JSON-RPC on stdio.
fn parse_args() -> Result<Option<std::path::PathBuf>, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => Ok(None),
        [flag, dir] if flag == "--root" => Ok(Some(std::path::PathBuf::from(dir))),
        _ => Err(format!(
            "usage: cochlea-mcp [--root DIR]   (got: {})",
            args.join(" ")
        )),
    }
}

fn main() -> std::process::ExitCode {
    let root = match parse_args() {
        Ok(root) => root,
        Err(usage) => {
            eprintln!("cochlea-mcp: {usage}");
            return std::process::ExitCode::from(2);
        }
    };
    let server = match &root {
        Some(dir) => match Server::with_root(dir) {
            Ok(server) => server,
            Err(err) => {
                eprintln!("cochlea-mcp: --root {}: {err}", dir.display());
                return std::process::ExitCode::from(2);
            }
        },
        None => Server::new(),
    };
    eprintln!(
        "cochlea-mcp {} starting on stdio{}",
        env!("CARGO_PKG_VERSION"),
        root.as_ref()
            .map(|r| format!(" (confined to {})", r.display()))
            .unwrap_or_default()
    );
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    match serve(&server, stdin.lock(), stdout.lock()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("cochlea-mcp: stdio error: {err}");
            std::process::ExitCode::from(1)
        }
    }
}
