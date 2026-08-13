# `cochlea-mcp`

An MCP (Model Context Protocol) stdio server over the cochlea libraries.
Any MCP client — Claude Code, another agent, a script — gets `render` /
`probe` / `spectro` / `lint` / `digest` / `loudness` / `beats` / `diff` /
`import` / `export` / `transcribe` / `reference` as tool calls, so it can
compose, render, and "listen" to audio through
numbers and images without shelling out to the `cochlea` binary or
reading raw PCM.

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
| `probe_audio` | `audio_path` (string, required), `from_s`/`to_s` (numbers, optional) | The full feature report, schema v5 (loudness/LUFS/true peak/LRA + short-term max, onsets, YIN pitch + quantized melody notes, MFCC timbre digest, chroma/key, a chord timeline and per-section key (harmony), tempo with candidates + stability, rhythm with grid alignment + straight-vs-triplet grid + `clear_rhythm`, stereo image, structural sections, silence, clipping) as pretty JSON. Works on any WAV, FLAC, mp3, or ogg — no score required. `from_s`/`to_s` zoom into a time window: report times become relative to the cut, `source.start_ms` anchors them. |
| `spectrogram` | `audio_path` (string, required), `out_path` (string, optional), `sheet` (bool, default `false`), `bars_per_tile` (integer, default `8`), `annotate` (bool, default `false`), `from_s`/`to_s` (numbers, optional) | The image itself, inline, as an MCP image content block (base64 PNG) whenever it fits the ~700 KB cap — a client with no filesystem access still gets to look at the audio — plus a text summary with pixel dimensions. `out_path` additionally (or, over the cap, instead) writes the PNG to disk. `annotate: true` draws the detected beat grid (orange, top), onsets (cyan, bottom), and pitch segments (magenta) on the image; `sheet: true` tiles a contact sheet instead (the two are mutually exclusive). |
| `lint_score` | `score_path` (string, required) | Text: `"ok: no lint findings"`, or the JSON list of findings. `isError: true` iff any finding is `Severity::Error`, matching `cochlea lint`'s exit-1 threshold. |
| `probe_digest` | `audio_path` (string, required), `window_ms` (number, default `1000`) | A ~40-line deterministic text digest (`cochlea_features::digest_text`) instead of a full JSON report — the token-cheap way to "listen" to an audio file. Prefer this over `probe_audio` unless the caller needs exact numbers to assert against. |
| `loudness_timeline` | `audio_path` (string, required), `hop_ms` (number, default `100`) | The loudness-over-time curve of the whole file as JSON: momentary (400 ms) and short-term (3 s) LUFS sampled every `hop_ms`, each point's time measured from the start of the file. The dynamics view the single integrated-LUFS / LRA summary in `probe_audio` can't give — where a mix gets loud, where a gate opens, how the level moves. (For a windowed, anchored analysis use `probe_audio` with `from_s`/`to_s`.) |
| `beat_grid` | `audio_path` (string, required) | The full beat grid of the whole file as JSON (a `TempoReport`): every detected beat time (ms, from the start of the file), the estimated downbeats, the tempo with its octave-alternative candidates, and a windowed stability score — the per-beat detail the compact `tempo` field inside `probe_audio` drops. |
| `score_reference` | *(none)* | The complete score-authoring reference as Markdown: the RON grammar (including the `master:` gain/limiter section), the live instrument-preset catalog (names, polyphony, every automatable param with unit/range/default — generated from the same registry that validates scores, so it cannot go stale), all embeddable `verify:` assertions, and a worked example that the test suite itself parses and renders. An agent should call this before its first `render_score`. |
| `audio_diff` | `audio_path_a` (string, required), `audio_path_b` (string, required), `window_ms` (number, default `1000`), `json` (bool, default `false`), `spectrogram` (bool, default `false`) | Feature-space comparison text (`cochlea_features::compare_text`): a verdict (`byte-identical` / `tier2-equivalent` / `different (dimensions...)`) plus per-dimension deltas, now including a timbre (MFCC) distance. `json: true` appends the full `CompareReport`; `spectrogram: true` also returns the signed A→B difference heat map inline (red = louder in B, blue = quieter, black = unchanged). A `different` verdict is a normal, successful answer — not `isError`. |
| `import_midi` | `midi_path` (string, required), `out_path` (string, required), `sample_rate` (integer, default `48000`) | Converts a Standard MIDI File (format 0/1, metrical division) to a RON score at `out_path`. Timing imports exactly; GM programs map to rough preset families and channel-10 percussion to kick/snare/hat tracks, with every mapping guess listed in the response for re-voicing. |
| `export_midi` | `score_path` (string, required), `out_path` (string, required) | The inverse of `import_midi`: converts a RON score to a Standard MIDI File (format 1) at `out_path`. Timing exports exactly (score ticks → SMF ticks, tempo map and time signature carry over); presets become rough General MIDI program labels, since a synth voice isn't a GM instrument. Use it to hand a composed score to a DAW or notation tool. |
| `transcribe_audio` | `audio_path` (string, required), `out_path` (string, required), `bpm` (number, optional), `grid` (string, default `"1/16"`), `preset` (string, default `"sine"`), `track_name` (string, default `"lead"`), `ppq` (integer, default `960`) | The inverse of `render_score`, and the arrow that closes the compose loop: audio in, an **editable RON score** out. Pitch-tracks the melody, reads its timing against `bpm` (detected from the audio when omitted), quantizes to `grid` (`"none"` keeps raw analyzer timing), and estimates each note's velocity from its peak level. Deliberately monophonic — chords, drums, and dense mixes come back as whichever single line the tracker locked onto. Every assumption (tempo and where it came from, the grid, the preset, clamped/repaired/dropped notes) is in the response text; treat the result as a draft to re-voice. |

Tool-level failures (a bad path, a render error, a failed verify or lint)
come back as a normal `tools/call` success response with `isError: true`
and the reason in the text content — never a JSON-RPC error. JSON-RPC
errors (`-32700`/`-32601`/`-32602`) are reserved for protocol problems:
malformed JSON, an unknown method, or missing/malformed arguments on a
known tool. `audio_diff`'s `different` verdict is *not* one of these
failures — see its row above.

## Confinement (`--root`)

By default the server reads and writes wherever the caller points it —
appropriate for a personal, local loop. For anything less trusted, launch
with `--root DIR`: every path argument on every tool, reads and writes
alike, must then resolve (canonically — symlinks and `..` are resolved
first) inside `DIR`, and anything else is refused as an Invalid Params
error before the filesystem is touched. This is defense against a
confused or prompt-injected *client*, not a sandbox against hostile local
processes.

Confinement covers paths the *score* implies, not just the ones the caller
types. `render_score` with a `stems_dir` derives one file per track from
the track name, and a track name is free-form score data — it can arrive from a
hand-authored RON file or, through `import_midi`, from a MIDI track-name
meta event. A name spelled as a path (`/etc/x`, `../../x`) would otherwise
escape both the stems directory and `DIR` while every path argument stayed
legitimately inside it. Two checks close that:

- **The name**, against `cochlea_render::stem_file_name` before the render
  starts — one ordinary, portable file name, with separators, `:`,
  `<>"|?*`, control characters and the Win32 device names refused on every
  platform so a score means the same thing on every host.
- **The path it lands on**, when the stem is written. A well-formed name is
  not enough: a symlink already sitting at `<stems>/<track>.wav` would be
  followed straight out of the directory, so anything already at that path
  is resolved and checked for containment. Presence is tested with
  `symlink_metadata`, not `exists()` — `exists()` follows the link, so a
  link pointing at a file that does not exist *yet* used to read as
  "nothing here" and skip the check while `File::create` followed it anyway
  (fixed in 0.7.1). A link that cannot be resolved is refused — including a
  dangling one whose target *would* have been inside the directory, since a
  lexical check of an unresolvable target is exactly the check that cannot
  see a redirect. One that resolves and stays inside the stems directory is
  ordinary and still works.

A stem that would land on the mix (`out_path`) or the score (`score_path`)
is refused too, so a `stems_dir` overlapping either cannot destroy them.
"Same file" here folds case (`Lead.wav` and `lead.wav` are one file on
macOS and Windows), on every platform — the same rule the CLI uses, and the
same rule that governs every `out_path`-versus-input check on this server.

```
claude mcp add cochlea -- cochlea-mcp --root ~/music-workspace
```

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
{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\n  \"schema_version\": 5,\n  \"source\": {\n    \"sample_rate\": 48000,\n    \"channels\": 2,\n    ...\n  },\n  ...\n}"}],"isError":false}}
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
