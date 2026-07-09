//! Library surface for `cochlea-mcp`, split out from `main.rs` so the pure
//! JSON-RPC dispatch ([`server::Server::handle_line`]) can be driven
//! in-process from integration tests (`crates/mcp/tests/`) — no subprocess,
//! no stdin/stdout framing, just strings in and strings out.
//!
//! `main.rs` is the only thing that isn't reachable from here: it's the
//! stdin/stdout loop that owns the real MCP transport.

pub mod protocol;
pub mod server;
pub mod tools;
