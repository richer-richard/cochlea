# Ecosystem listings

Draft only — do not submit without Richard's explicit go (`checklist.md`
principle applies here too, even though this file didn't exist when that
checklist was written). Researched against each target's *actual current*
contribution process on 2026-07-10 — the original checklist's blurbs were
placeholders written before wave 2 landed and before any of these repos'
real submission mechanics were checked; this file replaces that section
of `checklist.md` §3.

---

## 1. awesome-rust (rust-unofficial/awesome-rust)

**Not eligible yet — do not submit this PR now.** Their `CONTRIBUTING.md`
sets a hard bar: `(stars > 50 | downloads > 2000)`. Cochlea has neither
yet (repo went public today). Submitting now will very likely get closed
without review. Revisit once either threshold is crossed — worth checking
back after the launch posts have had a few days to run.

Content is drafted below so it's ready the moment the bar is cleared —
just needs `git log --oneline -1` re-confirmed current and the star count
re-checked before opening.

- **Section**: `### Audio and Music` (currently ~13 entries, alphabetical
  by link text)
- **Insertion point**: between `pdeljanov/Symphonia` and the `RustAudio`
  org entry
- **Template** (from their `CONTRIBUTING.md`, exact): `[ACCOUNT/REPO](https://github.com/ACCOUNT/REPO) [[CRATE](https://crates.io/crates/CRATE)] - DESCRIPTION`
- **Line to add**:
  ```
  * [richer-richard/cochlea](https://github.com/richer-richard/cochlea) [[cochlea](https://crates.io/crates/cochlea)] - A headless, deterministic audio engine for AI agents: compose scores as data, render byte-identical PCM offline, then verify against feature-report and spectrogram assertions.
  ```
- **How to submit**: their `CONTRIBUTING.md` literally says to use the
  GitHub "pen" (edit) icon on `README.md` and follow the fork+PR prompt
  — no separate fork/clone needed.

---

## 2. awesome-mcp-servers — punkpeye/awesome-mcp-servers (90.5k stars)

Target this one, not `wong2/awesome-mcp-servers` (4.2k stars) — wong2's
repo doesn't take PRs at all; it redirects contributors to a submission
form at mcpservers.org/submit. Checked both `CONTRIBUTING.md` files
2026-07-10 to confirm. punkpeye's takes direct PRs, no popularity bar.

- **Section**: `### 🎥 <a name="multimedia-process"></a>Multimedia
  Process` — closest fit ("handle multimedia, such as audio and video
  editing, playback, format conversion"); there's no dedicated
  audio-analysis category. Entries in this section aren't strictly
  alphabetical in practice (community PRs append informally) — insert
  near the other `r`-prefixed entries (`realcrabcut`, `rendobar`,
  `runapi-ai`) for readability, exact position doesn't matter.
- **Icons** (per their `## Legend`): 🦀 (Rust codebase), 🏠 (local
  service — it operates on local files, calls no remote API), 🍎🪟🐧 (CI
  matrix covers all three — confirmed via the pinned `test (Tier 2,
  macos-latest)` / `windows-latest` jobs). Not 🎖️ (that marks official
  vendor implementations, e.g. a company's own API server).
- **Line to add**:
  ```
  - [richer-richard/cochlea](https://github.com/richer-richard/cochlea) 🦀 🏠 🍎 🪟 🐧 - Render, analyze, and verify audio (WAV or FLAC) through a fully offline, deterministic engine. Compose scores as data, render byte-identical PCM, pull loudness, pitch, tempo, key, and structure reports, generate spectrograms, and diff two renders against each other. No ffmpeg, no audio device, just numbers an agent can actually reason about. `cargo install cochlea-mcp`
  ```
- **Submitted 2026-07-10**: PR opened, see status below.

---

## 3. MCP community registry (modelcontextprotocol/registry)

**This is not a PR** — the checklist's original assumption was wrong.
The official registry publishes via a CLI tool (`mcp-publisher`) against
`server.json`, not a pull request to their repo (their own
`CONTRIBUTING.md`: *"Do NOT open a pull request to add your server to
`data/seed.json`... it will not publish your server to the registry"*).

Good news: it natively supports Cargo/crates.io as a package source
(`registryType: cargo`), so no npm/PyPI repackaging needed.

**Already done, ahead of this**: `crates/mcp/server.json` is written (this
session) with `cochlea-mcp`'s metadata, and `crates/mcp/README.md` now
carries the required ownership-verification marker (`mcp-name:
io.github.richer-richard/cochlea-mcp`) as *visible* markdown text —
crates.io strips HTML comments from its rendered README, so the
`<!-- mcp-name: ... -->` hidden-comment form other registries accept
silently fails for Cargo packages. Both are committed
(`crates/mcp/README.md`, `crates/mcp/server.json`).

**Remaining steps require Richard** (interactive GitHub device-flow auth
— not something I can complete non-interactively):

```sh
# once cochlea-mcp is live on crates.io (publishing now, see status):
brew install mcp-publisher   # or the curl one-liner in their quickstart
cd crates/mcp
mcp-publisher login github    # opens a device-flow prompt: visit a URL, enter a code
mcp-publisher publish         # reads ./server.json, publishes to registry.modelcontextprotocol.io
```

Verify after: `curl "https://registry.modelcontextprotocol.io/v0.1/servers?search=io.github.richer-richard/cochlea-mcp"`

---

## 4. This Week in Rust (rust-lang/this-week-in-rust)

This **is** a PR, but not the simple one-line-link the original checklist
sketched. Their own `README.md` is explicit: *"We do not include: Links
that are solely to a GitHub repo or crate on crates.io."* — entries in
the "Project/Tooling Updates" section need a version/description hook
and should link to something with actual release detail (a blog post,
release notes, or changelog), not a bare repo root.

**Update 2026-07-10**: rather than lean on `CHANGELOG.md` alone, there's
now a real docs site — `mdBook`, built from `docs/{plan,determinism,
mcp}.md`, deployed via GitHub Actions to
<https://richer-richard.github.io/cochlea/>. That's a stronger link
target than a changelog: it's the actual determinism-contract writeup
and design doc, not just a version diff list. Points at the site's
introduction page, which itself links onward to the deeper pages.

- **Target file**: `draft/2026-07-15-this-week-in-rust.md` (their next
  open, unpublished issue as of 2026-07-10 — **re-check this filename
  before opening the PR**, it rolls to a new date every week the
  `draft/` folder is checked)
- **Section**: `### Project/Tooling Updates`
- **Line to add**:
  ```
  * [cochlea 0.1.0 — a headless, deterministic audio engine for AI agents](https://richer-richard.github.io/cochlea/)
  ```
- **How to submit**: fork → edit the draft file → PR against
  `rust-lang/this-week-in-rust`, per their README's "PRs for next issue
  are now being accepted" section. One submission per contributor per
  week is their stated limit for this section, so this uses Richard's
  one slot for that week if he's submitting anything else too.
