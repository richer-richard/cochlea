# X thread

Draft only — do not post without Richard's explicit go (`checklist.md`).
Post as a reply-to-self thread (not standalone tweets) — numbering below
is the reply order. Every tweet's character count below was measured
with a script (Python `len()` on the exact tweet text, no markdown), not
eyeballed — all six land well under 280, with 21-95 characters of
margin. X's own counter weights some characters slightly differently
(the URL in tweet 6 collapses to a fixed 23 chars via t.co regardless of
its real length, and emoji/CJK can count as more than one), so treat
these as close-but-verify-at-post-time, not exact.

---

**1/6** (attach `docs/assets/first_light_spectro.png`)

I built an audio engine with no speaker, no realtime path, and no human
ear in the loop.

Its only listener is an AI agent. This image is what it "sees" instead
of hearing a WAV file. 🧵

*(185 chars)*

---

**2/6**

An agent can't hear audio, and it shouldn't read raw samples either — a
3-min 48kHz float render is ~66MB. Useless as context.

But it can read a 2KB JSON report, or a <1KB text digest, or look at
one small spectrogram. And it can assert against any of those.

*(259 chars)*

---

**3/6**

So the loop is: compose (data) → render (deterministic PCM) → probe
(JSON/digest) → spectrogram (one vision call) → verify (assertions,
nonzero exit on fail).

The agent retries on a failed assertion — no human has to say "yes,
that sounds right."

*(247 chars)*

---

**4/6**

The hard part: byte-identical PCM, same input in, same bytes out, on a
pinned CI target. Enforced by clippy config, not convention — std
transcendentals are banned in DSP code, libm only. No fast-math, no
implicit FMA, denormals honored not flushed.

*(249 chars)*

---

**5/6**

Reads WAV and now FLAC (pure-Rust decode, verified bit-exact against
WAV twins). There's also an MCP stdio server — point Claude Code at it
and an agent calls render/probe/spectrogram/diff as tools, no shelling
out, no reading raw PCM.

*(235 chars)*

---

**6/6**

Pure Rust throughout. No ffmpeg, no unsafe, no audio device or GUI/GPU
crate can even enter the dependency graph — a cargo-deny ban checked in
CI, not a README claim.

Repo, would love feedback: https://github.com/richer-richard/cochlea

*(236 chars, link included; t.co shortens the URL further)*

---

Notes for whoever posts this:

- Tweet 1 needs the actual image attached, not just referenced — attach
  `docs/assets/first_light_spectro.png` (or re-render a fresh one if the
  demo score has changed) when composing the tweet in X's UI. A thread
  that opens with an image outperforms a thread that opens with text
  alone.
- All six char counts above were verified with a script counting the
  tweet text as it would actually post (no markdown, no code fences) —
  re-count after any edit, don't eyeball it; X truncates silently past
  280 in some clients.
- Cut from an earlier 8-tweet draft: a Rust code snippet (its own tweet)
  and a "voices tick sample-by-sample" DSP detail. X doesn't render code
  fences well and the sample-by-sample point is a nice-to-have next to
  the byte-identical claim already in tweet 4, not essential — cut for
  thread length, not because it's wrong.
- If stereo imaging or structure detection (wave 2, still in flight as
  of this draft) has landed by post time, it's a reasonable addition
  between 5 and 6 — but only with real, verified specifics.
