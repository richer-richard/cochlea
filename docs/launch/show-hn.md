# Show HN

Draft only — do not post without Richard's explicit go (`checklist.md`).
Use HN's "Show HN" submission form so it prefixes the title itself (don't
type "Show HN:" into the title field manually).

## Title (pick one)

```
A. Cochlea – a deterministic, headless audio engine built for AI agents
B. Cochlea: audio you can byte-diff, a headless engine built for agents
C. I built an audio engine with no speaker and no human ear in the loop
```

Char counts (own length / with the "Show HN: " prefix the form adds; HN's
field allows ~80 with the prefix):

- A: 70 / 79
- B: 68 / 77
- C: 68 / 77

A states the thing plainly — safest, does the least work to earn a
click. B leads with the determinism hook, which is the most defensible
and most discussion-worthy claim in the whole post. C is pure curiosity
bait in HN's classic "I built a weird thing" register — highest
click-through risk/reward, and it undersells the technical depth until
the body. My pick would be B if I had to choose one, but any of the
three works; A is the fallback if B reads as too cute.

URL: `https://github.com/richer-richard/cochlea`

## Body (submission text)

```
Cochlea is an audio engine with no realtime path, no audio device, and
no human ear in the loop — because its only user is an agent. It writes
a score as data, renders it offline to PCM, and lets the agent "listen"
through numbers and small images instead of raw samples.

The problem it's solving: an agent can't hear a WAV file, and it
shouldn't have to read one sample-by-sample either — a 3-minute render
at 48kHz/32-bit-float PCM is about 66MB, useless as context. A rendered
7-second demo in the repo is 2.7MB; its full JSON feature report is
2KB; a plain-text digest of the same file is well under 1KB and stays
that size even for much longer files (the digest is row-capped, not
duration-scaled). Here's what that digest actually looks like, real
output from `cochlea probe demo.wav --digest`:

    cochlea digest: 7.036s  2ch  48000Hz
    loudness: integrated=-22.70  momentary_max=-21.84  true_peak=-15.91  lra=10.61
    key: E major (conf 0.81)  pitch: voiced=98%  median=110.0Hz (A2 +0.0c)
    tempo: 56.0bpm (conf 0.00) clear_rhythm=false
    stereo: width=0.03 corr=1.00 bal=-0.00
    structure: 1 section
    onsets: count=6  rate=0.85/s
    silence: leading=0ms  trailing=2486ms
    clipping: clipped=0  over_0dbtp=false
    timeline: window=1000ms  bucket=1x  rows=6
       idx        t(s)     rms   peak  ons     f0  flags
         0   0.000-1.000   -23.57  -18.99    0   110.0  -
         1   1.000-2.000   -24.16  -17.97    1    80.5  -
         ...

An agent reads that, or looks at one small spectrogram PNG, and asserts
against either. So the loop is: compose -> render -> probe -> verify,
and the agent retries on a failed assertion without a human confirming
"yes, that sounds right."

The part I spent the most time on is determinism, because "run it again,
get the same bytes" turns out to be genuinely hard for audio — filters
and delays carry state, so per-sample purity isn't free the way it is in
a lot of other domains. The contract is three tiers: byte-identical PCM
on a pinned CI target (enforced by banning std transcendentals in DSP
code via clippy config — libm only — plus no fast-math, no implicit FMA,
denormals honored rather than flushed, fixed summation order), feature
tolerances across platforms (LUFS +-0.1 LU, onsets +-2ms, pitch +-5
cents), and spectrogram sentinels via image diff. All three are CI-
tested, not just claimed.

Lossless input works too — WAV and now FLAC (pure-Rust decode via
symphonia), verified bit-exact against WAV twins of the same content.

There's also an MCP stdio server (hand-rolled JSON-RPC, no async
runtime) so Claude Code or any other MCP client can render/probe/diff
audio as tool calls without shelling out to a binary or reading PCM
directly.

Pure Rust throughout — no ffmpeg, no unsafe (forbidden workspace-wide),
no audio device or GUI/GPU crate ever enters the dependency graph (a
`cargo-deny` ban enforced in CI, not just a README claim).

Honest about what's not here yet: v1 renders from synthesized
instruments only (no sample playback), there's no realtime path by
design (this is an offline batch tool, not something you'd plug into a
DAW), and lossy formats (mp3/ogg) aren't decoded yet — FLAC and WAV
only for now.

README has the full workflow with a real probe JSON excerpt and rendered
spectrograms: https://github.com/richer-richard/cochlea

Happy to answer questions about the determinism approach specifically —
it's the part I'd most want pushback on.
```

Notes for whoever posts this:

- The digest excerpt is real output (`cochlea probe` against
  `examples/scores/first_light.ron` rendered to WAV, rerun 2026-07-09 after
  the schema-v2 digest lines landed) —
  re-run it before posting if the score, presets, or digest format have
  changed since, rather than trusting this copy verbatim.
- The closing line is deliberate: HN rewards a specific, answerable
  invitation over a generic "let me know what you think." Determinism is
  the part most likely to draw genuinely interesting technical
  disagreement (someone will ask about denormal handling, or why libm
  over std, or whether "byte-identical" is actually achievable
  cross-platform for anything beyond the pinned target — all good
  questions with real answers already in `docs/determinism.md`).
- The "honest about what's not here yet" paragraph is load-bearing, not
  filler — HN punishes posts that oversell, and this project's own
  design docs are explicit that realtime/MIDI/sampled-instruments/lossy-
  decode are deliberately out of scope, not forgotten. Stating that
  plainly heads off the "so it can't actually..." comment before it's
  written.
- If stereo imaging or structure detection (wave 2, still in flight as
  of this draft) has landed by post time, they're reasonable additions
  to the tech-notes paragraph — but only with real, verified specifics,
  not this draft's guess at what they'll look like.
