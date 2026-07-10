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
- **Submitted 2026-07-10**: https://github.com/punkpeye/awesome-mcp-servers/pull/9787

**Update 2026-07-10/11**: the repo's automation commented on the PR requiring
a Glama listing (glama.ai/mcp/servers) that passes their automated
build+introspection check, with the resulting score badge added to the
entry. Done:

- Submitted `richer-richard/cochlea` via the Glama "Add Server" form
  (already-authenticated session, no separate signup needed).
- Claimed the server (auto-claimed on submission, matching GitHub
  ownership) and configured its Dockerfile via the server's own admin
  page — **not** committed to our repo; Glama runs its own Dockerfile
  against the source for their checks.
- First build attempt failed: `rustup --profile minimal` has no C
  linker, and `debian:trixie-slim` ships neither `gcc` nor
  `build-essential` — `error: linker `cc` not found`. Fixed by adding
  `apt-get install -y gcc libc6-dev` ahead of the rustup install in the
  build step.
- Second attempt (a real cold compile of the full workspace: rustup
  install + `cargo build --release -p cochlea-mcp` + the
  build→introspection test phase) took **8m45s** end to end and passed.
- Added the score badge to the actual README entry (matching the file's
  real convention: link → badge → icons → description, not just prose
  in the PR body):
  ```
  [![richer-richard/cochlea MCP server](https://glama.ai/mcp/servers/richer-richard/cochlea/badges/score.svg)](https://glama.ai/mcp/servers/richer-richard/cochlea)
  ```
- Replied on the PR confirming completion, linking the live Glama page.

---

## 3. MCP community registry (modelcontextprotocol/registry)

**This is not a PR** — the checklist's original assumption was wrong.
The official registry publishes via a CLI tool (`mcp-publisher`) against
`server.json`, not a pull request to their repo (their own
`CONTRIBUTING.md`: *"Do NOT open a pull request to add your server to
`data/seed.json`... it will not publish your server to the registry"*).

**Update 2026-07-10, `registryType: cargo` doesn't actually work yet**:
the registry's own schema and docs describe Cargo/crates.io support, and
`mcp-publisher validate` passes against it, but the production server
rejects it: `registry validation failed for package 0 (cochlea-mcp):
unsupported registry type: cargo`. Turns out the work is merged upstream
(`modelcontextprotocol/registry` #1055, #1207, #1330) but not yet
released to production — tracked in an open, unanswered issue:
[#1423](https://github.com/modelcontextprotocol/registry/issues/1423),
where I left a confirming comment (second real-world report, deliberately
didn't mention the MCPB workaround below to keep the ask focused).

**Switched to MCPB instead** (Richard approved 2026-07-10): the registry
does support `registryType: mcpb` today — a prebuilt binary bundle
hosted as a GitHub Release asset. Built:

- `crates/mcp/mcpb/manifest.json` — the MCPB manifest (binary server
  type, `platform_overrides` picking the right native binary per OS),
  validated against the official schema.
- `.github/workflows/release-mcp.yml` — tag-triggered
  (`cochlea-mcp-v*`): builds `cochlea-mcp` natively on
  ubuntu/macos/windows-latest (same runners as `ci.yml`, no
  cross-compilation needed), assembles the three binaries into the
  bundle, packs it with the official `@anthropic-ai/mcpb` CLI, and
  publishes it as a GitHub Release with a sha256 checksum.
- Tagged and released: [`cochlea-mcp-v0.1.0`](https://github.com/richer-richard/cochlea/releases/tag/cochlea-mcp-v0.1.0)
  — build, package, and release all succeeded on the first run.
- `crates/mcp/server.json` now points its one `packages` entry at that
  release asset (`registryType: mcpb`) with the real download URL and
  sha256. The dormant `cargo` entry was removed for now — keeping both
  would fail the whole publish, since the registry rejects the entire
  `server.json` if any one package entry is invalid, not just that
  entry. Re-add `cargo` once #1423 ships.

**Publish attempt**: `mcp-publisher validate` passes. `mcp-publisher
publish` first caught a real bug (description exceeded the registry's
100-char limit — fixed), then hit an expired JWT from Richard's earlier
login. Re-triggered `mcp-publisher login github` — waiting on Richard to
complete the device-flow prompt, then will re-run `publish`.

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

- **Section**: `### Project/Tooling Updates`
- **Line added** (colon instead of em dash, per house style):
  ```
  * [cochlea 0.1.0: a headless, deterministic audio engine for AI agents](https://richer-richard.github.io/cochlea/)
  ```
- **Submitted 2026-07-10**: https://github.com/rust-lang/this-week-in-rust/pull/8370
  (draft `2026-07-15-this-week-in-rust.md`). Uses Richard's one
  submission slot for that section this week.
