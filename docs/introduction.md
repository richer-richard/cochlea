# cochlea

A headless, deterministic audio engine for AI agents. Write a score as
data, render it offline to byte-identical PCM, then listen through
numbers — loudness, onsets, pitch, key, spectrograms — and assert what
you heard. Compose → render → probe → verify, with no human ear (and no
audio device) anywhere in the loop.

There is no realtime path in this project, and there never will be one.
Offline render is ground truth.

![Mel spectrogram of first_light.ron: six note onsets followed by a reverb tail decaying to silence](./assets/first_light_spectro.png)

*What the agent sees instead of hearing a WAV file.*

## Why

An agent can't listen to audio, and it shouldn't have to read raw PCM
either — a few minutes of 48kHz float audio is tens of megabytes, which
is a bad way to spend a context window. Cochlea's answer is a small set
of primitives an agent can compose:

- **Score IR** — ticks, tempo, tracks, notes, automation, and an
  optional master bus (gain + brick-wall limiter), expressed as data
  (Rust builder or RON). Standard MIDI Files import and export with
  timing intact.
- **Deterministic render** — the same score renders to the same PCM
  bytes every time on the pinned CI target, enforced at the toolchain
  level, not by convention. See [Determinism Contract](./determinism.md).
- **Feature reports** — loudness, true peak, onsets, pitch plus a
  quantized melody (note events an agent can diff against what it
  wrote), an MFCC timbre digest, key, tempo (with octave-alternative
  candidates and stability), rhythm (grid alignment with a
  straight-vs-triplet hypothesis test, syncopation, a trustable
  `clear_rhythm`), a chord timeline and per-section key (harmony), stereo
  width, structure — as a few kilobytes of JSON, or a sub-kilobyte text
  digest sized for an LLM's context window.
  Every read tool takes a `--from/--to` window, so a long file can be
  probed a few bars at a time.
- **Spectrograms** — a small PNG when a report alone doesn't answer the
  question; optionally annotated with the detected beats, onsets, and
  pitch, and diffable as a signed A→B heat map.
- **Verify** — an assertion DSL over a render (`true_peak_below`,
  `pitch_matches_score`, `tempo_is`, ...), so an agent can retry on a
  failed assertion instead of asking a human "does that sound right?"

Every crate here works standalone. `cochlea probe` runs on any WAV,
FLAC, mp3, or ogg file with no score in sight — that's the adoption
wedge, and it's enforced structurally (`features`/`spectro` depend on
neither `score` nor `synth`, checked via `cargo tree` in CI).

## Where to start

- Writing a score — the RON grammar, every preset and parameter, the
  verify assertions, a worked example: [Score Format
  Reference](./score-format.md). The same text is served in-band by
  `cochlea reference` and the MCP `score_reference` tool, and a test
  pins all three to the same generator.
- Building instruments and scores, or wiring cochlea into a bigger
  system: [Design & API Surface](./plan.md).
- The determinism contract, and the fundsp/rustfft audits behind it:
  [Determinism Contract](./determinism.md).
- Running cochlea as an MCP server so an agent calls render/probe/diff
  as tools: [MCP Server](./mcp.md).

## Install

```sh
cargo install cochlea       # the CLI: render / probe / lint / spectro
cargo install cochlea-mcp   # the MCP stdio server
cargo add cochlea-features  # or any crate, as a library dependency
```

All 9 crates are on [crates.io](https://crates.io/crates/cochlea).
Source, issues, and the workspace layout live at
[github.com/richer-richard/cochlea](https://github.com/richer-richard/cochlea).
