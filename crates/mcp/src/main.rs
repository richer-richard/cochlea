//! `cochlea-mcp`: an MCP (Model Context Protocol) stdio server exposing
//! cochlea's render/probe/spectro/lint pipeline as agent tools, so any MCP
//! client can compose, render, and "listen" to audio without shelling out
//! or reading PCM directly. Hand-rolled JSON-RPC 2.0 over newline-delimited
//! stdio — no async runtime; offline batch tools need none.
//!
//! stdout carries only protocol response lines; everything else (this
//! banner included) goes to stderr. EOF on stdin is a clean exit.
//!
//! This file is deliberately thin: framing only. The dispatch logic lives
//! in the library's `server` module as a pure function of strings
//! (`Server::handle_line`), so it's tested without a subprocess
//! (`crates/mcp/tests/`).

use std::io::{BufRead, Write};

use cochlea_mcp::server::Server;

fn main() {
    eprintln!(
        "cochlea-mcp {} starting on stdio",
        env!("CARGO_PKG_VERSION")
    );
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let server = Server::new();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                eprintln!("cochlea-mcp: stdin read error: {err}");
                break;
            }
        };
        let Some(response) = server.handle_line(&line) else {
            continue;
        };
        if writeln!(stdout, "{response}").is_err() || stdout.flush().is_err() {
            eprintln!("cochlea-mcp: stdout write failed, exiting");
            break;
        }
    }
}
