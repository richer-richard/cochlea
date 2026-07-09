# cochlea-mcp

An MCP (Model Context Protocol) stdio server over the
[cochlea](https://github.com/richer-richard/cochlea) libraries, so any
MCP client — Claude Code, another agent, a script — gets
render/probe/spectrogram/diff as tool calls, without shelling out to the
`cochlea` binary or reading raw PCM.

```sh
cargo install cochlea-mcp
claude mcp add cochlea -- cochlea-mcp
```

Hand-rolled JSON-RPC 2.0 over newline-delimited stdio: one JSON object
per line in, at most one per line out. No async runtime — offline batch
tools (render a score, extract a report, write a PNG) need none. Every
response is a pure function of its request: no wall clock, no session
state.

Six tools: `render_score`, `probe_audio`, `spectrogram`, `lint_score`,
`probe_digest`, `audio_diff` — accepting WAV or FLAC wherever audio
input applies. Tool-level failures (a bad path, a failed verify) come
back as a normal success response with `isError: true`; JSON-RPC errors
are reserved for protocol problems (malformed JSON, unknown method,
missing arguments).

Full tool table, schemas, and a request/response example live in
[`docs/mcp.md`] in the main repo.

[`docs/mcp.md`]: https://github.com/richer-richard/cochlea/blob/main/docs/mcp.md

License: MIT OR Apache-2.0, at your option.
