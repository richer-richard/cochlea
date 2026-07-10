# Builds cochlea-mcp: a stdio MCP server, no ports, no network I/O — talks
# JSON-RPC over stdin/stdout only. Mount audio/score files into the
# container and pass their in-container paths as tool arguments.
FROM rust:1.95.0-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --locked -p cochlea-mcp

FROM debian:bookworm-slim
COPY --from=builder /build/target/release/cochlea-mcp /usr/local/bin/cochlea-mcp
ENTRYPOINT ["/usr/local/bin/cochlea-mcp"]
