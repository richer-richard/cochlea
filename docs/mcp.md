# `cochlea-mcp`

An MCP (Model Context Protocol) stdio server over the cochlea libraries.
Any MCP client — Claude Code, another agent, a script — gets `render` /
`probe` / `spectro` / `lint` / `digest` / `diff` as tool calls, so it can
compose, render, and "listen" to audio through numbers and images without
shelling out to the `cochlea` binary or reading raw PCM.

The protocol is hand-rolled JSON-RPC 2.0 over newline-delimited stdio: one
JSON object per line in, at most one JSON object per line out. No async
runtime — offline batch tools (render a score, extract a report, write a
PNG) need none. Every response is a pure function of its request: no wall
clock, no session state, so identical requests produce identical responses.

## Tools

Each tool mirrors the matching `cochlea` CLI subcommand's semantics exactly
(see `crates/cli/src/main.rs`) over the same library calls — this is a
second front end onto the same offline pipeline, not a reimplementation.

| Tool | Arguments | Returns |
| --- | --- | --- |
| `render_score` | `score_path` (string, required), `out_path` (string, required), `stems_dir` (string, optional), `verify` (bool, default `false`) | Text summary: frame count, duration, sample rate, peak dBFS, stems written; if `verify` is set, the full verify-report JSON is appended and the call reports `isError: true` on a failed verification. |
| `probe_audio` | `audio_path` (string, required) | The full feature report (loudness/LUFS, true peak, onsets, YIN pitch, chroma/key, silence, clipping) as pretty JSON. Works on any WAV or FLAC — no score required. |
| `spectrogram` | `audio_path` (string, required), `out_path` (string, required), `sheet` (bool, default `false`), `bars_per_tile` (integer, default `8`) | Text: the written PNG path and its pixel dimensions. Plain spectrogram or, with `sheet: true`, a tiled contact sheet — no score-aware bar markers yet (those need score context via `render_score`). |
| `lint_score` | `score_path` (string, required) | Text: `"ok: no lint findings"`, or the JSON list of findings. `isError: true` iff any finding is `Severity::Error`, matching `cochlea lint`'s exit-1 threshold. |
| `probe_digest` | `audio_path` (string, required), `window_ms` (number, default `1000`) | A ~40-line deterministic text digest (`cochlea_features::digest_text`) instead of a full JSON report — the token-cheap way to "listen" to a WAV or FLAC. Prefer this over `probe_audio` unless the caller needs exact numbers to assert against. |
| `audio_diff` | `audio_path_a` (string, required), `audio_path_b` (string, required), `window_ms` (number, default `1000`), `json` (bool, default `false`) | Feature-space comparison text (`cochlea_features::compare_text`): a verdict (`byte-identical` / `tier2-equivalent` / `different (dimensions...)`) plus per-dimension deltas. `json: true` appends the full `CompareReport` as pretty JSON. A `different` verdict is a normal, successful answer — not `isError`. |

Tool-level failures (a bad path, a render error, a failed verify or lint)
come back as a normal `tools/call` success response with `isError: true`
and the reason in the text content — never a JSON-RPC error. JSON-RPC
errors (`-32700`/`-32601`/`-32602`) are reserved for protocol problems:
malformed JSON, an unknown method, or missing/malformed arguments on a
known tool. `audio_diff`'s `different` verdict is *not* one of these
failures — see its row above.

## Client setup

Claude Code:

```
claude mcp add cochlea -- cargo run -p cochlea-mcp --release
```

or, against an already-built binary:

```
claude mcp add cochlea -- /path/to/target/release/cochlea-mcp
```

Any other stdio MCP client: launch `cochlea-mcp` (or `cargo run -p
cochlea-mcp`) as a subprocess and speak JSON-RPC 2.0 over its stdin/stdout,
one object per line. stdout carries only protocol responses; all logging
goes to stderr, so it's safe to leave stderr connected to a terminal or log
file without corrupting the transport.

## Example

Request (`tools/call` for `probe_audio`, sent as one line):

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"probe_audio","arguments":{"audio_path":"mix.wav"}}}
```

Response (one line back; the pretty-printed report is escaped into the
`text` field, shown here unescaped for readability):

```json
{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\n  \"schema_version\": 1,\n  \"source\": {\n    \"sample_rate\": 48000,\n    \"channels\": 2,\n    ...\n  },\n  ...\n}"}],"isError":false}}
```

## Testing this crate

`crates/mcp/src/lib.rs` exists so the dispatch logic (`server::Server`) can
be driven in-process from integration tests — `Server::handle_line(&str) ->
Option<String>` takes one request line and returns at most one response
line, with no stdin/stdout/subprocess involved. `crates/mcp/src/main.rs` is
just the framing loop around it. See `crates/mcp/tests/protocol.rs` for
JSON-RPC conformance and `crates/mcp/tests/tools_e2e.rs` for a real
render → probe → spectrogram round trip against
`examples/scores/first_light.ron`.
