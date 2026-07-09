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

fn main() -> std::process::ExitCode {
    eprintln!(
        "cochlea-mcp {} starting on stdio",
        env!("CARGO_PKG_VERSION")
    );
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    match serve(&Server::new(), stdin.lock(), stdout.lock()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("cochlea-mcp: stdio error: {err}");
            std::process::ExitCode::from(1)
        }
    }
}
