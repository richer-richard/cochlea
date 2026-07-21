# cochlea-mcp

An MCP (Model Context Protocol) stdio server over the
[cochlea](https://github.com/richer-richard/cochlea) libraries, so any
MCP client — Claude Code, another agent, a script — gets
render/probe/spectrogram/diff/import as tool calls, without shelling
out to the `cochlea` binary or reading raw PCM.

```sh
cargo install cochlea-mcp
claude mcp add cochlea -- cochlea-mcp
```

Hand-rolled JSON-RPC 2.0 over newline-delimited stdio: one JSON object
per line in, at most one per line out. No async runtime — offline batch
tools (render a score, extract a report, write a PNG) need none. Every
response is a pure function of its request: no wall clock, no session
state.

Eight tools: `render_score`, `probe_audio`, `spectrogram`, `lint_score`,
`probe_digest`, `audio_diff`, `import_midi`, `score_reference` —
accepting WAV, FLAC, mp3, or ogg wherever audio input applies, with
`from_s`/`to_s` window parameters on the read tools for zooming into a
time range. `score_reference` makes the server
self-describing: the full RON score grammar, the live instrument-preset
catalog (generated from the validation registry, so it cannot go stale),
every verify assertion, and a worked example the test suite itself
renders — an agent connected cold can compose without ever seeing the
repo. `spectrogram` returns the image inline as MCP image content
(base64 PNG, size-capped) — optionally annotated with the detected
beats, onsets, and pitch — so clients without filesystem access still
get to *look* at audio, and `audio_diff` can return a signed A→B
difference heat map the same way. `import_midi` brings Standard MIDI
Files into the loop with timing intact. Launch with `--root DIR` to confine every read
and write to one directory (canonical-path checked before any
filesystem work). Tool-level failures (a bad path, a failed verify) come
back as a normal success response with `isError: true`; JSON-RPC errors
are reserved for protocol problems (malformed JSON, unknown method,
missing arguments, confinement refusals).

Full tool table, schemas, and a request/response example live in
[`docs/mcp.md`] in the main repo.

[`docs/mcp.md`]: https://github.com/richer-richard/cochlea/blob/main/docs/mcp.md

## Links

- Repo: <https://github.com/richer-richard/cochlea>
- MCP Registry name: `mcp-name: io.github.richer-richard/cochlea-mcp`

License: MIT OR Apache-2.0, at your option.
