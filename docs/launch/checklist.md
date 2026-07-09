# Launch checklist

Drafts only. Nothing in this directory gets posted, published, or flipped
public without Richard's explicit go — that includes each individual step
below, not just the batch as a whole (`docs/superpowers/specs/2026-07-09
-agent-audio-v2-design.md` §5). This file is the order of operations;
`show-hn.md`, `r-rust.md`, and `x-thread.md` are the copy-paste-ready post
text.

## 0. Before any of this: confirm the release is actually done

- [ ] Both build waves gates-green: `cargo fmt --all --check`, `cargo
      clippy --workspace --all-targets -- -D warnings`, `cargo test
      --workspace`, `cargo deny check`.
- [ ] Wave 2 (`docs/superpowers/specs/2026-07-09-agent-audio-v2-design.md`
      §2) actually landed: `cochlea-decode` (done), FLAC wired into the
      CLI and MCP server (in flight as of this writing — verified working
      via `cochlea probe *.flac --digest` locally, but confirm it's
      actually merged before claiming it in posts), tempo/beat +
      `clear_rhythm` (done), stereo/LRA, structure detection, `Report`
      schema v2, the new `VerifySpec` variants, the drum-groove demo +
      golden hash.
- [ ] **Re-read `show-hn.md`/`r-rust.md`/`x-thread.md` against the
      *actual* shipped feature set before posting them.** They're
      grounded in real CLI output as of this writing (see each file's
      "grounding" note for the exact commands run), but if stereo/
      structure work lands with different specifics than the design doc
      sketched, the posts need a pass, not a blind copy-paste.
- [ ] Demo suite renders clean, spectrogram sentinels match, golden PCM
      hash still confirmed on the Tier-1 CI target.
- [ ] `CHANGELOG.md` — **doesn't exist yet.** Not in this task's scope to
      create (docs/launch/** only), but it's a real gap: a launch-day
      audience checking crates.io or the repo root for "what's actually
      in this release" will look for one. Create it before flipping
      public, not after.
- [ ] README screenshots actually render on GitHub — they're relative
      paths to `docs/assets/*.png`, committed, so this should just work
      once the repo is public, but view the rendered README on GitHub
      once (not just locally) before posting anywhere that links to it.
- [ ] `docs/mcp.md` is accurate — it's been edited alongside the FLAC-
      wiring work (tool arg names changed, e.g. `wav_path` → `audio_path`
      on `probe_audio`/`spectrogram`/`audio_diff` as of this writing);
      confirm it matches `crates/mcp/src/tools.rs`'s actual schemas
      before any post that links to it.
- [ ] LICENSE files: `LICENSE-MIT` and `LICENSE-APACHE` are present at
      the repo root and every crate inherits `license.workspace = true`
      — already true, just a final look.
- [ ] 3-5 good-first-issue candidates, filed and labeled, so Show HN /
      r/rust traffic that wants to contribute has somewhere to land
      instead of bouncing. Candidates (not yet filed — pick from these or
      substitute, then actually open the issues):
      - Shell completions for the `cochlea` CLI (`clap_complete` — small,
        self-contained, exercises the existing `Cli` derive).
      - One more synth preset following the existing `Patch` trait
        pattern (`crates/synth`) — the six shipped presets are a
        reasonable but small set.
      - A `CONTRIBUTING.md` (genuinely useful and a fine first PR for
        someone who just read the codebase to write it).
      - Expand the assertion cookbook in `README.md` with a couple more
        worked `VerifySpec` examples once schema v2's new checks land.
      - A property test for a `cochlea-features` extractor that doesn't
        have one yet (check coverage against `crates/features/tests/`).

## 1. Repo — flip public

- [ ] `richer-richard/cochlea` is currently **private** (confirmed via
      `gh repo view --json isPrivate`). Flip with:
      ```
      gh repo edit richer-richard/cochlea --visibility public
      ```
      Explicit, separate ask — don't infer it from "the code is ready."
- [ ] Once flipped: confirm the CI badge in `README.md` goes green on the
      public repo (Actions need to actually run publicly, not just have
      run privately).
- [ ] Set a social preview image (repo Settings → General → Social
      preview, 1280×640 recommended). Reuse a rendered spectrogram —
      `docs/assets/first_light_spectro.png` or `title_cue_spectro.png` are
      already committed and make the point ("this is what an agent sees")
      without any design work. Crop/pad to 1280×640 first.
- [ ] Description and topics are already set via `gh` (14 topics,
      `agent-tools`/`ai-agents`/`mcp`/etc. — see `[[cochlea-v1-status]]`
      memory) — confirm they still read accurately once wave 2 lands
      (e.g. add a `flac` topic now that FLAC decode is real).

## 2. crates.io — publish in dependency order

Nothing here happens until the repo is public (crates.io links back to
the repo; publishing from a private repo is a bad look and breaks the
"no ffmpeg, pure Rust, verify it yourself" pitch these posts make).

Order (respects the acyclic dependency graph in `docs/plan.md`):

1. `cochlea-score`
2. `cochlea-synth` (needs `cochlea-score`)
3. `cochlea-render` (needs `cochlea-score`, `cochlea-synth`)
4. `cochlea-features`
5. `cochlea-spectro`
6. `cochlea-decode` (needs `cochlea-features`)
7. `cochlea-verify` (needs `cochlea-score`, `cochlea-features`, `cochlea-render`)
8. `cochlea` (the CLI binary — needs everything above)
9. `cochlea-mcp` (needs everything above)

Steps:

- [ ] `cargo publish --dry-run -p <crate>` for every crate above, in this
      order, on a clean checkout — catches missing metadata / bad
      `include`/`exclude` / version-pin drift before it's irreversible.
- [ ] Check every crate's `Cargo.toml` has `description`, `repository`,
      `license`, `keywords`, `categories` filled in (workspace-inherited
      already — spot check `cochlea-decode` and `cochlea-mcp`
      specifically since they're the newest members).
- [ ] `fenestra-anim` is already on crates.io (per the workspace manifest
      comment) — nothing to do there, just confirm the pinned version
      still resolves.
- [ ] `cargo publish -p <crate>` for real, same order, waiting for each to
      index before publishing the next dependent (crates.io propagation
      isn't always instant).
- [ ] Confirm docs.rs builds clean for each crate after publish (docs.rs
      builds automatically on publish; check the build log, not just that
      a page eventually renders).

## 3. Ecosystem listings

Each entry below is the actual repo URL plus a one-line blurb ready to
paste into that listing's PR/submission — adjust only for each project's
specific required format.

- [ ] **awesome-rust** — PR adding a line under the Audio/Music (or
      Text-to-Speech-adjacent Audio) section, matching the existing
      entries' format:
      ```
      * [cochlea](https://github.com/richer-richard/cochlea) — A headless, deterministic audio engine for AI agents: compose scores as data, render byte-reproducible PCM offline, then listen through feature reports, spectrograms, and assertions.
      ```
- [ ] **awesome-mcp-servers** — PR adding `cochlea-mcp`, format depends
      on the specific list's convention (check current `README.md`
      there first), blurb:
      ```
      **[Cochlea](https://github.com/richer-richard/cochlea)** - Render, analyze, and verify audio (WAV/FLAC) through a deterministic offline engine — compose scores, extract loudness/pitch/onset/key reports, generate spectrograms, and diff renders, all as MCP tools.
      ```
- [ ] **MCP community registry** (modelcontextprotocol.io's official
      servers list, if one exists at launch time — check current state,
      this ecosystem moves fast) — submit `cochlea-mcp` per their
      current submission process, same blurb as above.
- [ ] **This Week in Rust** — submit via their normal PR-to-content-repo
      process (check the `this-week-in-rust` repo's `CONTRIBUTING.md` for
      the current mechanism — historically a PR adding a line to the next
      unpublished issue under "Crate of the Week" nominations or the
      general "News" section):
      ```
      [cochlea](https://github.com/richer-richard/cochlea) — a headless, deterministic audio engine for AI agents, with an MCP server so agents can render, probe, and diff audio as tool calls.
      ```

## 4. Launch posts

Order matters less than *not all at once* — stagger by a few hours so
each platform's discussion doesn't compete with the others for Richard's
attention answering comments.

- [ ] **Show HN** (`show-hn.md`) — pick one of the three title options,
      post via HN's actual "Show HN" submission form (it prefixes the
      title itself, don't type "Show HN:" manually). Best posted weekday
      morning US Eastern for visibility. Richard should plan to answer
      comments for the first few hours — that's most of what makes or
      breaks a Show HN thread's ranking.
- [ ] **r/rust** (`r-rust.md`) — post as a text post (not a link post) so
      the technical framing shows up in-feed, with the repo link in the
      body. Flair as appropriate if the sub uses flairs.
- [ ] **X thread** (`x-thread.md`) — post as a thread (reply-to-self
      chain), not a wall of separate unconnected tweets, with the first
      tweet's spectrogram image attached. First tweet is the hook; it's
      the only one most people will see, so it has to stand alone.

## 5. Later, explicit go required (not part of this launch batch)

- [ ] Claude-in-Chrome screenshots of the MCP tools running inside Claude
      Code, for docs and follow-up posts. Deliberately deferred — needs
      its own explicit go from Richard when he wants it, not bundled into
      the initial launch (`docs/superpowers/specs/2026-07-09-agent-audio
      -v2-design.md` §5.6).
