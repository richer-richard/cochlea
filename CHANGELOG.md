# Changelog

All notable changes to the cochlea workspace. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the workspace
versions all crates together.

## [0.7.1] — 2026-08-13

An adversarial pass over 0.7.0, and it found the two things a security
release is most likely to leave behind: the hole it *thought* it closed,
reopened one presence-test away, and a comparison that was only ever right on
one kind of filesystem.

0.7.0 stopped a symlink at a stem's path from redirecting the write out of
the stems directory — but tested for the link with `Path::exists()`, which
follows it, so a link pointing at a file that did *not* exist yet read as
"nothing there", skipped the check entirely, and got followed by
`File::create` anyway. And the same release taught the stem *set* that two
names differing only by case are one file on macOS and Windows, without
teaching that to the guard between a stem and the mix, the report, or the
score — so `--out d/Lead.wav --stems d` with a track named `lead` still
destroyed the mix at exit 0.

Two more of the same shape turned up in the review of *this* release's own
fixes: `spectro` and `eval` were the two subcommands the 0.7.0 overwrite
sweep never reached, and both could destroy a file they had just read. A
third pass, over those fixes in turn, found the pattern one level up — the
rule had been shared but the *resolving* in front of it had not, which left
the Python `spectrogram` binding with no guard at all and two front ends
calling something spelled `same_file` and meaning different things. The whole
predicate now lives in one place, resolving included.

Alongside them, three arithmetic bugs on the position path: a bar/beat
position is a product of two numbers a score file picks, and nothing bounded
either one.

### Fixed

- **A *broken* symlink at a stem's path could still redirect the write out
  of the stems directory, and out of the MCP server's `--root`.** 0.7.0's
  containment check ran only `if path.exists()`, and `exists()` follows the
  link: a link whose target does not exist yet reports `false` and the check
  never ran, while `File::create` followed the link all the same and created
  the stem at the far end — reported at exit 0 on the CLI and `isError:
  false` over MCP. Reproduced against 0.7.0 on both front ends, which share
  the write sink.
  `write_stems_as` now tests with `symlink_metadata` (which does not follow),
  and refuses a link it cannot resolve rather than guessing where the write
  lands. A link that resolves *inside* the directory is still ordinary and
  still works.

- **A stem could overwrite the mix, the report, or the input score when the
  names differed only by case — at exit 0.** macOS and Windows are
  case-insensitive by default, so `d/Lead.wav` and `d/lead.wav` are two paths
  and one file there; every overwrite guard compared them exactly.
  Reproduced on macOS: `render score.ron --out d/Lead.wav --stems d` with a
  track named `lead` wrote the mix, then wrote one track over it, and exited
  0 having reported the mix's frame count. The same gap sat on the MCP side
  (`render_score`'s stem guard, and every `out_path` / input alias check —
  `spectrogram`, `import_midi`, `export_midi`, `transcribe_audio`).
  The comparison is now one shared function, `cochlea_render::same_file`,
  called by all three front ends.

- **The Python `spectrogram` binding could write its PNG over its own
  input** — the same bug as `cochlea spectro --out`, in the third front door
  onto the same call. It was missed because the first fix shared the
  *comparison* but left each front end to do its own path resolving, so
  "every write path is guarded" was a claim about the binary only.
  `cochlea_render::same_file` now does both halves, and the CLI, the MCP
  server and the bindings all call exactly it.

- **Two spellings of one not-yet-created path could pass the overwrite
  guard.** The resolver canonicalizes a path's parent when the path itself
  does not exist yet, and returned the path *untouched* when the parent did
  not exist either — which is precisely the case where a run creates its own
  output directory. `render s.ron --stems ./new --report new/lead.wav
  --verify` compared `./new/lead.wav` against `new/lead.wav`, found them
  different, wrote the `lead` stem and then wrote the report on top of it.
  The fallback now folds the path's components lexically (`.` dropped,
  `name/..` cancelled), which can only ever make two spellings look like one
  file — never one file look like two.

- **`cochlea spectro --out` could destroy the file it was rendering.** The
  audio is fully decoded before the PNG is written, and the subcommand had
  no aliasing guard at all — the MCP `spectrogram` tool it mirrors has had
  one since it shipped. A `.wav` output happened to be refused by the PNG
  encoder, which reads as safety but is luck: `cochlea_decode::load`
  identifies a file by its magic bytes when the extension is not an audio
  one, so `spectro audio.png --out audio.png` decoded 2.7 MB of WAV and
  wrote a 45 KB spectrogram over it, at exit 0. Reproduced.

- **`cochlea eval --json` could destroy a candidate or a reference.** The
  report is written after every pair has been compared, so
  `eval --candidates d --references r --json d/a.wav` printed `1/1 passed`,
  exited 0, and left a 208-byte JSON report where a golden WAV had been —
  the next run would then compare against a file that is no longer audio.
  Reproduced. `--json` is now checked against every candidate and every
  reference the run will read, before the first comparison.

  With these two, every write path in the CLI finally routes through the one
  `same_file` guard its own doc comment claimed for it — both were missed
  because the rule was applied per-subcommand as each was written.

- **A bar/beat position could overflow `u64`.** `Pos::resolve` computes
  `(bar - 1) · ticks_per_bar`, and *both* factors are caller-supplied: `bar`
  is a `u32` from the score, and `TimeSignature::validate` bounded only
  "beats is not zero". A score with `time_signature: (4294967295, 4)` and a
  note at bar 100000000 panicked the loader with "attempt to multiply with
  overflow" in debug, and wrapped to a silently wrong — possibly *accepted* —
  tick in the release profile every shipped binary is built with. Now
  checked, and refused with `PositionTooFar`. (The numerator itself is
  bounded too — see Changed.)

- **A raw-tick position past the ceiling still panicked the renderer.**
  Bounding the grid arm of `Pos::resolve` closed the RON `verify:` route,
  which always builds a `(bar, beat)` position, and left the identical panic
  reachable one line up in the Rust API: `Ticks` is a public newtype over a
  public `u64`, so `rendered.verify(&score).silent_after(Ticks(1 << 40))`
  handed an unchecked tick to `Score::resolve` and on into `mul_div`. Both
  arms are bounded now. A raw tick past the ceiling is the same authoring
  mistake as a bar past it, and gets the same `PositionTooFar`.

- **`Pos::resolve` divided by a caller-supplied zero.** It is a `pub fn`
  taking a `TimeSignature` whose fields are also `pub`, and it called
  `ticks_per_beat` — `whole_note_ticks / unit` — before any validation ran,
  so `unit: 0` was a division-by-zero panic in the very function this release
  hardened against untrusted numbers. The signature is validated before it is
  used. (Unreachable through the CLI or the MCP server, which validate at
  load; reachable by anyone using the crate directly.)

- **A far-future `verify:` position panicked the renderer, after the mix was
  written.** `Score::resolve` — which backs the RON `verify:` block and
  `cochlea-verify`'s own `Pos` resolution — never applied the `Ticks::MAX`
  bound the note/tempo/automation builders apply, so a verify assertion at
  bar 4294967295 sailed past load and reached the tempo map's exact rational
  arithmetic at render time: `mul_div ... overflows u64`, panicking *after*
  `--out` had already been written. The bound now lives in `Pos::resolve`
  itself, so every grid position passes through it once, whichever door it
  came in by.

- **A track name of 256 MB or more panicked `export_midi`.** A meta event's
  length is itself a variable-length quantity, and `write_vlq_into` writes
  into a four-byte buffer — a length past 28 bits indexed off the end of it.
  Unreachable with any real name, but a track name is score data, and score
  data does not get to pick the panic; it is now refused with the existing
  "track name too long for SMF" error.

- **`time_signature: (4, 3)` reported the wrong number.** The one validation
  error covered both a zero numerator and an unusable denominator, printing
  the *beats* value against the *unit*'s range — so a bad unit read as
  "time signature 4 out of range 1..=32". They are now two messages, each
  naming the value it is actually complaining about.

- **`cochlea eval` derived one reference path two ways.** The `--json`
  overwrite guard joined the candidate's raw file name onto `--references`;
  the comparison loop twenty lines below joined its `to_string_lossy`
  spelling. For a candidate whose name is not valid UTF-8 those are different
  files, so a `--json` aimed at the lossy one passed the guard and then landed
  on the reference the run had just read. One derivation now, used by both.

- **A stem-write failure blamed a symlink that wasn't there.** Every
  `canonicalize` error at a stem's path was reported as "a broken or looping
  link", including failures that have nothing to do with links (a permission
  wall on the target's path, a name the filesystem will not take). The
  message is now earned by looking at what is actually at the path; anything
  that is not a link propagates its own io error.

### Changed

- **Two output paths that differ only by case are now refused on every
  platform**, not just on the hosts where they collide. This is the same
  trade `stem_file_name` made in 0.7.0: a score is portable data, and a pair
  of outputs that would silently merge on a colleague's laptop is better
  refused here than accepted because this host happens to keep them apart.
  On Linux that newly refuses commands like
  `probe in.wav --json A.json --spectro a.json`. Renaming one output is the
  fix. (Unicode NFC/NFD spellings of the same name are still treated as
  distinct — the same documented limit the stem-set check carries.)

- **A time signature's numerator is bounded to 1..=255.** It was any nonzero
  `u32`, which no consumer could actually represent: `export_midi` clamped it
  into the SMF time-signature meta event's single byte, so a score that said
  `(300, 4)` exported a file that said 255/4 — wrong data, no error, and
  nothing for the reader to notice. Bounding the input beats truncating at
  the exit. Scores with a numerator above 255 (which could not round-trip
  through MIDI, and whose bar 2 was already unreachable) are now refused at
  load with the value they named.

- **A symlink at a stem's path that cannot be resolved is refused**, where
  0.7.0 followed it and 0.7.1's first cut aborted the export with a
  misleading reason. This includes a dangling link whose target would have
  been *inside* the stems directory (`stems/lead.wav -> stems/take2.wav`
  before `take2.wav` exists), which used to work by accident. Reading the
  link and checking its target lexically would mean trusting the one thing a
  lexical check cannot see through, which is how the hole this closes was
  opened; refusing is the honest answer. Delete the link, or write the stem
  under its own name.

## [0.7.0] — 2026-08-11

A security release, and an unusually honest one. A track name is score data,
and `--stems` turns it into a file path — so a name spelled as a path wrote
outside the stems directory, and outside the MCP server's `--root` while every
path *argument* stayed legitimately inside it. Fixing that took two passes:
the first validated the name and shipped documentation claiming a confinement
it did not have, because a symlink already sitting at the stem's path is
followed regardless of how well-formed the name is. Containment is now checked
where it belongs, at the resolved path. Around it, three ways a stem could
overwrite the mix, the report, or the score it was rendered from — all at exit
0 — are closed, and the name rule is enforced identically on every platform so
a score exports the same stems on every host.

Two breaking changes come with it, both called out under Changed: `RenderError`
is now `#[non_exhaustive]` with two new variants, and some track names that
previously exported stems are refused.

### Fixed

- **A track name could write a stem outside the stems directory — and
  outside the MCP server's `--root`.** Stems are written as
  `<dir>/<track>.wav`, and a track name is free-form score data, but it was
  handed to `Path::join` unchecked. `Path::join` *replaces* the base path
  when its argument is absolute, so a track named `/etc/x` wrote `/etc/x.wav`
  and one named `../../x` climbed out. Reproduced on both front ends against
  0.6.0. Over MCP it was a confinement bypass: with `score_path`, `out_path`,
  and `stems_dir` all legitimately inside `--root`, the escape rode in on the
  score's data and the tool still reported `isError: false`. The payload did
  not need a hand-written score either — `cochlea import` lifts a MIDI
  track-name meta event verbatim, so a downloaded `.mid` carried it.
  - The rule is now one public function, `cochlea_render::stem_file_name`: a
    stem name must be exactly one ordinary path component, with `/` and `\`
    both refused on every platform so a score means the same thing on every
    host. `write_stems_as` validates every name *before* creating the
    directory or writing a file, so a refused name leaves nothing behind, and
    both front ends pre-flight the same rule so the mix is never written
    first. A `..` on its own is still allowed and still safe: the appended
    extension makes it the ordinary file `...wav`.
  - Scope: writes were always suffixed `.wav` and could not create parent
    directories, so this was arbitrary-location write of `.wav` files into
    existing directories — destructive to any `.wav` the user could write,
    but not a path to code execution.

- **A symlink at a stem's path could still redirect the write out of the
  stems directory, and out of `--root`.** Validating the *name* is not
  enough: if `<stems>/lead.wav` already exists as a link pointing elsewhere,
  `File::create` follows it. Reproduced — a render truncated a file outside
  the server's root while every name and every argument was legitimate.
  `write_stems_as` now canonicalizes the stems directory and, for any stem
  path that already exists, checks where it actually lands before writing.
  A link that stays *inside* the directory is ordinary and still works: the
  rule is containment, not a ban on links. This is the same trap
  `resolve_write` was hardened against in 0.6.0, arriving by a path that
  never went through `resolve_write`.

- **A stem could overwrite the mix, the report, or the input score, at exit
  0.** Two separate holes. The CLI's `same_file` canonicalized both sides,
  which fails for a path that does not exist yet — so for two *outputs*, the
  pair it most needed to judge, it silently degraded to a raw string
  compare: `render --out d/stems/lead.wav --stems d/stems/../stems`
  destroyed the mix with the `lead` stem. It now resolves through the
  canonical parent plus file name, the shape the MCP server's
  `resolve_write` already used. Separately, the guard never compared a stem
  against the *score*, and `load_score` accepts any extension, so a score
  kept as `d/lead.wav` with `--stems d` was read, rendered, and then
  destroyed. And `render_score` (MCP) had no stem-collision guard at all —
  `out_path = <root>/lead.wav` with `stems_dir = <root>` wrote the mix and
  then overwrote it with one track, reporting success and the mix's peak.

- **Two tracks whose names differ only by case no longer silently lose a
  stem.** `Lead` and `lead` are distinct tracks and both valid names, but
  one file on macOS and Windows. The set is now checked before any stem is
  written. (Unicode NFC/NFD spellings of the same name still collide; that
  would need a normalization dependency this workspace does not carry, and
  it is documented as a known limit rather than half-solved.)

### Changed

- **Breaking — some track names that exported stems before are now
  refused.** The stem-name rule is enforced on every platform, not just the
  one where each check bites, because a score is portable data and should
  export the same stems everywhere rather than working on one host and
  failing (or writing somewhere unexpected) on another. On Unix that newly
  refuses names containing `\`, `:`, `<>"|?*`, or control characters, names
  longer than a file name can be, and the Win32 device names (`NUL`, `CON`,
  `COM1`, …) — the last of which used to write a stem to the null device on
  Windows and report success. Renaming the track is the fix; a score that
  does not use `--stems` / `stems_dir` is unaffected. A bare `..` is still
  allowed and still safe: with the extension appended it is the ordinary
  file `...wav`.

- **Breaking — `RenderError` is now `#[non_exhaustive]`,** and has two new
  variants (`UnwritableStemName`, `CollidingStemNames`). Adding a variant to
  a public enum is already breaking for any downstream exhaustive `match`,
  so the attribute lands in the same release rather than forcing a second
  break later. Downstream matches need a `_` arm.

- **The golden-audio composite action passes its inputs through the
  environment instead of `${{ }}` interpolation.** Interpolation pastes a
  value into the script text *before* bash parses it, so an input carrying a
  quote and `$(…)` would have run as a command; an environment variable is
  only ever data. Not exploitable as the action shipped — the inputs come
  from the calling workflow's own author — but this action exists to be
  copied into other repositories, and a consumer who wired an input to
  `github.event.*` data would have handed that data a shell. The action's
  inputs are unchanged, so callers need no edit.

## [0.6.0] — 2026-08-07

The compose loop closes. `render` made audio from a score and `probe` made
numbers from audio; `transcribe` makes a *score* from audio, so an agent can
now go around the circle instead of down a one-way street. Alongside it, an
adversarial pass over the new surface and the old one: three reachable panics
(a one-byte MIDI corruption, a hostile duration string, a note stack from
rounding), an MCP symlink write that destroyed the file it was reading, a
quadratic velocity pass, and a family of arguments that silently fell back to
their defaults instead of erroring.

### Added

- **`transcribe`: audio → an editable score, over the CLI and MCP.** The
  arrow the compose loop was missing. `render` turns a score into audio and
  `probe` turns audio into numbers; `cochlea transcribe solo.wav --out
  score.ron` (MCP: `transcribe_audio`) turns audio back into **RON you can
  edit and render again**. It pitch-tracks the melody, reads its timing
  against a tempo (detected from the audio unless you pass `--bpm`),
  quantizes to a note grid (`--grid 1/16`, `1/8t`, `none`, …), and estimates
  each note's velocity from its peak level. Deliberately monophonic and
  deliberately loud about it: chords and drums read as whichever single line
  the tracker locked onto, and every assumption — the tempo and where it came
  from, the grid, the preset, clamped or repaired or dropped notes — comes
  back as a warning, the same ethos as `import`'s mapping guesses. MCP is now
  twelve tools.
  - Architecture: the conversion is pure and score-side
    (`cochlea_score::transcribe`, taking plain `NoteObservation` data), so
    the dependency law holds — `features` still knows nothing about `score`,
    and the tick math is unit-testable with no audio in sight.
  - New public API in service of it: `cochlea_features::peak_dbfs_between`
    (plain numeric peak level over a time window) and its batched form
    `peak_dbfs_for_windows`, which downmixes once for many windows;
    `cochlea_score::Bpm::validate` (the tempo bounds as one public rule, so
    `--bpm` is rejected at the flag boundary); `Dur::MAX_FRACTION_TERM`; and
    `TranscribeOpts::grid_anchor_ms`, which phase-locks the quantization grid
    to a detected beat rather than to tick 0.

### Fixed

- **The quantization grid is phase-locked to the detected beat.** A grid is a
  phase as much as a spacing, and it was pinned at tick 0 — which assumes the
  audio begins exactly on a beat. Any recording with a count-in, room tone, or
  a pickup was therefore quantized against a grid offset from its real one,
  corrupting the rhythm rather than merely the tempo. Both front ends now
  estimate the beat grid (even when `--bpm` pins the tempo, since the tempo
  says how far apart the lines are and the first beat says *where* they fall)
  and pass it as `TranscribeOpts::grid_anchor_ms`. Ticks below the anchor snap
  symmetrically, so a pickup keeps its place instead of being dragged onto the
  downbeat.
- **An MCP write path can no longer destroy the file it is reading through a
  symlink.** `resolve_read` canonicalized fully while `resolve_write` stopped
  at the parent directory, so a symlinked *final* component made the two
  disagree about the same file: `audio_path = out_path = take.wav` (a link to
  `master.wav`) compared unequal, slipped past the aliasing guard, and the RON
  write followed the link and replaced the audio. Reproduced — a 249 KB WAV
  became 498 bytes of RON. Write paths now resolve an existing target the rest
  of the way, which also means `--root` confinement sees where a symlink
  actually lands. Shared by `transcribe_audio`, `import_midi`, and
  `export_midi`.
- **`Dur::parse` can no longer overflow on a hostile duration.** The
  dotted/triplet multipliers ran unchecked on freshly parsed `u32` terms, so
  `--grid "1/2147483648."` (and the MCP `grid` argument) overflowed `den * 2`
  — a panic in debug builds, a wrapped nonsense fraction in release. Terms are
  now bounded at parse (`Dur::MAX_FRACTION_TERM`, far finer than one tick at
  any PPQ) and the modifiers saturate, so direct library callers are safe too.
- **Transcription no longer emits simultaneous notes.** The input is a
  monophonic line, but rounding both ends of a short note to the grid could
  land its start and end on one tick while the next note snapped to that same
  tick — emitting a stack instead of a melody. Overlapping notes are now
  shortened to end where the next begins, a note landing on a taken tick is
  dropped, and both are reported.
- **Invalid flags fail before the audio is read, and nothing is written until
  the score lints.** `--preset`, `--ppq`, `--grid`-at-that-PPQ, and the input's
  sample rate are all checked at the boundary, and the lint moved ahead of the
  write — previously a bad `--preset` overwrote `--out` with an unrenderable
  score and only then exited non-zero. `transcribe_audio` gained the same
  checks plus the lint it never had (it used to report success for a score
  that would fail at `render_score`).
- **Velocity estimation is no longer quadratic.** `peak_dbfs_between`
  downmixes the whole buffer, and both front ends called it once per detected
  note — hundreds of full-length allocations on a few-minute file. New
  `peak_dbfs_for_windows` downmixes once for all windows.
- **An MCP integer argument no longer falls back to its default when the
  caller passed something invalid.** `usize_or` treated a negative or
  fractional JSON number the same as a missing key, so `{"ppq": -100}` (or
  `import_midi`'s `{"sample_rate": -1}`) silently proceeded at the default
  instead of erroring — while the CLI's clap rejects `--ppq -100` outright.
  That divergence broke the rule that each MCP tool mirrors its CLI
  subcommand exactly. Present-but-invalid integers are now Invalid Params
  errors, for both `transcribe_audio`'s `ppq` and `import_midi`'s
  `sample_rate`. String arguments (`grid`, `preset`, `track_name`) are strict
  the same way, an explicit JSON `null` reads as "not set" everywhere
  (clients commonly serialize unset optionals that way), and the one
  remaining silent-default helper was removed rather than left beside its
  replacement — so `spectrogram`'s `bars_per_tile` is validated too.
- **A corrupt MIDI file can no longer panic the importer.** A time-signature
  meta event stores its denominator as a power-of-two *exponent*, and that
  byte was shifted unvalidated (`1u32 << payload[1]`) — so any exponent past
  31 overflowed the shift: a panic in debug builds, a garbage denominator in
  release. It was reachable by a *single byte flip* in an otherwise-valid
  file, not just by a crafted one. The shift is now checked, an out-of-range
  denominator is reported as a warning and the default 4/4 kept, and the rest
  of the file still imports. Hardened with a malformed-input suite
  (`crates/score/tests/midi_malformed.rs`): hostile meta payloads, every
  truncation, every single-byte corruption, the header field space, and
  extreme delta times — all asserted to return a `Result`, never panic.

## [0.5.0] — 2026-08-06

Two more non-subtractive voices and a pair of over-time analysis surfaces, plus
a quiet data-loss fix. `marimba` and `organ` widen the palette to eleven
presets; `loudness_timeline` and `beat_grid` — CLI flags and MCP tools — expose
the dynamics curve and the full beat grid that the summary reports drop; and
every CLI write guard now canonicalizes before comparing, closing an
aliased-path hole that let a read subcommand overwrite its own input.

### Added

- **Two non-subtractive voices: `marimba` and `organ`** (the palette grows to
  eleven presets). `marimba` is modal synthesis — a struck bar as a fundamental
  plus tuned octave partials (1 : 4 : 8), each a sine under a fast squared
  rational-decay envelope, so the strike is wooden and the pitch reads straight
  back. `organ` is additive — a Hammond-ish drawbar registration (harmonics
  1, 2, 3, 4, 6, 8 at tapering levels) under a soft attack/release, sustained
  and filterless. With `fm_bell` that's now three non-subtractive timbres,
  widening the palette past "everything is a filtered saw". Pure arithmetic over
  exact harmonics — the golden PCM hashes are unchanged (neither flagship score
  uses them).
- **`loudness_timeline` and `beat_grid`, over the CLI and MCP.** `cochlea probe
  --loudness <path>` writes the momentary + short-term LUFS dynamics curve;
  `--beats <path>` writes the full beat grid (every beat time, downbeats, tempo
  candidates, stability) that the compact `tempo` summary in the main report
  drops. The same two are new MCP tools (`loudness_timeline`, `beat_grid`),
  returning JSON inline — the dynamics view and per-beat detail an agent
  couldn't previously get short of rendering a spectrogram. MCP is now eleven
  tools. Both describe the whole file, with times measured from its start (the
  MCP tools take no window; `probe --loudness/--beats` ignore `--from/--to`).

### Fixed

- **The read subcommands can no longer overwrite their own input through an
  aliased path.** `probe`/`diff`/`import` guarded against clobbering the input
  with a raw path compare, which a different spelling of the same file (`probe
  in.wav --json ./in.wav`) slipped past: the output write landed on the input
  and the command still exited 0. Every write guard — including a new one on
  `render` — now routes through one shared `same_file` helper that canonicalizes
  before comparing, the check only `export` previously had. Regression-tested.

## [0.4.0] — 2026-07-24

The voice-and-ears upgrade, and a hardening pass from an adversarial review:
close the two reproduced crashes and the read-path DoS, then answer harmony,
loudness-over-time, integer-PCM and MIDI export, a golden-audio eval harness,
and Python bindings.

### Hardening (adversarial review)

- **No tool can take the MCP server down.** `cochlea-mcp` now wraps every
  tool dispatch in `catch_unwind`, so a panic anywhere in the pipeline becomes
  a contained `isError` result instead of unwinding the single-threaded serve
  loop and killing the long-lived process (and every in-flight and future
  request in the session). Regression-tested with a poison call followed by a
  harmless one.
- **Far-future positions can't panic the renderer.** Authored ticks (tempo
  changes, note ends, automation keys) are bounded at load by `Ticks::MAX`
  (2³²) — chosen to keep the exact tick→sample/ns rational arithmetic from
  overflowing `u64` while still exceeding any one-hour render — so a crafted
  tempo tick that used to reach unchecked `mul_div` and panic now fails cleanly
  with `PositionTooFar`. Covers the programmatic builder, the RON loader, and
  MIDI import.
- **The read path is bounded.** `cochlea_decode::load` caps total decoded
  samples (`DEFAULT_MAX_SAMPLES`, overridable via `load_with_limit`),
  refusing a decompression bomb *as it accumulates* and an oversized WAV from
  its header — closing the unbounded-allocation / O(window²)-compute DoS that
  render's one-hour cap didn't cover.
- **Degenerate audio is refused at the door.** Zero channels, zero sample
  rate, or a ragged interleave are rejected by both `Audio::from_wav` and the
  decode boundary, so the mel spectrogram and per-frame analyzers can never be
  reached with a shape that would divide by zero or trip an assertion.

### Added — hearing (Report schema v5)

- **`harmony`** (`HarmonyReport`): a chord timeline plus per-section key — the
  two questions ("what's the progression", "what key is the bridge in") the
  single global `key` couldn't answer. Chords are template-matched from the
  same chroma the key estimate uses (major/minor/7ths/dim/aug/sus4 in all
  twelve roots), with a presence gate and a simplicity bias so a bare triad
  isn't over-read as a seventh or a lone note as a chord. Per-section key
  reuses the Krumhansl-Schmuckler correlation over structure-segmented chroma.
  Standalone `analyze_harmony`; a `harmony:` line in the digest.
- **`loudness.short_term_max_lufs`** and a standalone `loudness_timeline`
  (momentary + short-term LUFS sampled over time) — the dynamics view the
  single integrated/LRA summary can't give.
- **Downbeat / bar-relative reporting**: `TempoReport` gains `beats_per_bar`
  and `downbeats_ms` (the bar-opening beats, by onset-energy phase under an
  assumed meter), plus `TempoReport::bar_beat_at(ms)` — "beat 3 of bar 2"
  instead of a raw millisecond offset.

### Added — generation

- **`fm_bell` preset**: a single-operator FM voice (harmonic ratio so the
  pitch reads back on the played note, a decaying modulation index for the
  bright metallic strike) with an automatable `brightness` param — the
  palette's first non-subtractive voice, widening the timbral range and
  showing the IR carry a timbre knob an agent can sweep, not just a cutoff.

### Added — I/O

- **16- and 24-bit integer WAV output** alongside the lossless 32-bit float
  (`Rendered::write_wav_as`/`WavBitDepth`; CLI `render --bits 16|24|float`;
  MCP `render_score` `bits`). Deterministic round-to-nearest quantization,
  clamped, no dither.
- **MIDI export** (`export_midi`): a Score → Standard MIDI File (format 1)
  writer, the inverse of the importer — timing (ticks, tempo map, time
  signature) exports exactly, instruments become rough GM labels. CLI
  `export`, MCP `export_midi`; round-trip tested against the importer.
- **Python bindings** (`bindings/python`, pyo3 + maturin): `probe`, `diff`,
  `render`, `spectrogram`, `probe_digest`, `samples_identical`, plus an
  `assert_audio(...)` fluent API and a pytest `assert_audio` fixture. The
  deterministic core stays pure Rust; this is a thin reach layer (a detached
  crate, kept out of the determinism-critical build).

### Added — golden-audio harness

- **`cochlea eval`**: score a directory of candidate audio files against a
  directory of references (matched by filename), deterministically — a
  per-file verdict table, an aggregate pass rate, exit 1 on any regression or
  missing reference, optional JSON. The generative-model / reference-render
  regression oracle.
- A **GitHub composite action** (`.github/actions/golden-audio`) wrapping it,
  and a [golden-audio testing guide](docs/golden-audio.md).

## [0.3.0] — 2026-07-22

The hearing upgrade: melody as notes, timbre identity, triplet grids, a
zoom lens over every read tool, annotated and diff spectrograms, a master
bus with a real limiter, lossy-format probe input, and MIDI import.

### Added — reading audio (Report schema v4, CompareReport v3)

- **`pitch.melody`** (`MelodyNote`): the YIN track quantized to
  equal-tempered note events — start/end, note name, median f0, cents
  deviation. The compose loop's read-back half: an agent that wrote a
  melody can diff what the render *sounds like* as notes against what it
  wrote. Monophonic by construction (documented); standalone entry point
  `extract_melody`. The digest gains a `melody:` line.
- **`timbre`** (`TimbreReport`): a compact MFCC digest (26 mel filters,
  13 coefficients, per-coefficient mean and spread over the buffer) —
  the "did the re-render keep the instrument's character" axis. The
  compare report gains `timbre.mfcc_distance` (spectral shape only, `c0`
  excluded) and the diff text a `timbre` row.
- **`rhythm.grid`**: grid alignment now tests two subdivision hypotheses
  — straight sixteenths and eighth-note triplets — and reports whichever
  more onsets land on. A shuffle reads as an aligned *triplet* rhythm
  (alignment 1.0 on the calibration fixture) instead of being force-fit
  to sixteenths and scored sloppy; straight eighths stay decisively
  `straight` (their off-beats sit at 1/2 beat, not a triplet point).
  `CompareReport` gains `rhythm.grid_changed` — a feel change, distinct
  from a tightness change.
- **The zoom lens**: `Audio::window` (frame-exact `[from, to)` cuts) +
  `ProbeOpts::start_ms` + `source.start_ms` in the report; CLI
  `probe`/`spectro`/`diff` take `--from/--to`, and the MCP
  `probe_audio`/`spectrogram` tools take `from_s`/`to_s`. Report times
  are relative to the cut; `start_ms` anchors them in the file.
- **mp3 and ogg/vorbis probe input** (`cochlea-decode`): a lossy
  symphonia path with its contract stated where it lives — analysis
  input, never render ground truth (reproducible per build; no exactness
  claim once a codec has discarded the original samples). Extension and
  magic-byte dispatch (ID3/MPEG sync, `OggS`); the same tone probed from
  WAV, mp3, and ogg reads the same pitch within 5 cents (tested). The
  FLAC bit-exact module is untouched and separate.

### Added — looking at audio

- **Annotated spectrograms**: `render_annotated` draws analysis overlays
  on the image — detected beats (orange ticks, top edge), onsets (cyan
  ticks, bottom), pitch segments (magenta lines at their mel band) — via
  a plain-data `Overlay` (the spectro crate still never sees score or
  feature types). CLI `spectro --annotate`; MCP `spectrogram` with
  `annotate: true` (still inline image content).
- **Signed diff spectrograms**: `render_diff_png` — per-band `B − A` in
  dB, red = louder, blue = quieter, black = unchanged, saturating at
  ±24 dB. A moved onset is a blue/red vertical pair; a brightened sweep
  is a red wedge. Refuses mismatched analysis axes (`AxisMismatch`). CLI
  `diff --spectro delta.png`; MCP `audio_diff` with `spectrogram: true`
  (inline). `MelSpec` now carries its frequency axis (`fmin`/`fmax`) and
  maps Hz→band via `hz_band`.

### Added — making audio

- **Master bus** (`Master`/`Limiter` in the score IR, RON `master:`
  section): an output gain (−40..+24 dB) and a brick-wall lookahead
  limiter (ceiling −40..0 dBFS, lookahead 0..50 ms, release 1..1000 ms),
  applied to the f64 stem sum before the single f32 rounding. Offline
  lookahead is a forward sliding-window maximum — no delay line, no
  latency — and the *sample*-peak ceiling holds exactly by construction
  (leave ~1 dB under a `TruePeakBelow` target; true peaks are
  inter-sample). The do-nothing default master skips the stage entirely:
  master-less scores render byte-identically to 0.2.0 (both golden
  hashes unchanged), and `mix == Σ stems` still holds for them. Stems
  stay pre-master. This is the tool the loop kept asking for: push with
  `gain_db`, hold the ceiling, assert `IntegratedLufs` + `TruePeakBelow`.
- **MIDI import** (`cochlea_score::import_midi`, `cochlea import`, MCP
  `import_midi`): hand-rolled SMF format 0/1 parser (chunks, VLQs,
  running status). Timing maps exactly — the file's division becomes the
  score's PPQ verbatim, tempo metas become tempo-map steps, notes land
  unquantized on the integer grid. GM programs map to rough preset
  families and channel-10 percussion splits into kick/snare/hat tracks;
  every guess is returned as a warning for re-voicing. SMPTE division
  and format 2 are refused with reasons; CCs/bends/aftertouch are
  skipped loudly, never silently.

### Changed

- `probe()` runs one shared YIN pass for the pitch summary and melody.
- MCP tool descriptions updated to schema v4 and the wider format
  support; the tool list is eight (`import_midi` added).

## [0.2.0] — 2026-07-21

> **Versioning note.** The crates published to crates.io
> as `0.1.0` on 2026-07-10 were built from a tree that already contained
> the entire "agent read stack" below — the workspace was published
> mid-cycle without a version bump, so the published `0.1.0` does not
> match this changelog's `0.1.0` entry. `0.2.0` is the first release
> where the version number, the tag, and this changelog agree. Sorry;
> won't happen again (releases now cut from a tagged, changelog-matched
> commit only).

### Changed — the tempo/rhythm split (Report schema v3)

Tempo (speed) and rhythm (pattern) are separate axes that change
independently — a drum solo holds a steady pulse while its pattern turns
confusing — and are now separate detectors and report sections:

- **`tempo.confidence` is now pulse clarity** — mean-removed,
  length-unbiased normalized autocorrelation — replacing the v2
  mass-fraction score, which structurally punished music for carrying
  several metrical levels at once (the `drum_groove` demo read 0.01
  despite a spot-on BPM; it now reads 0.79). Not comparable to v2 values.
- **`tempo.candidates`**: the winner plus the strongest distinct octave
  alternatives with saliences — metrical ambiguity surfaced as data for
  the caller to weigh, instead of a coin flip hidden behind the prior.
- **`tempo.stability`**: windowed tempo agreement (mod octave) — the
  axis that separates "the rhythm got confusing" from "the speed moved".
- **New top-level `rhythm` section** (`cochlea_features::RhythmReport`):
  `grid_alignment` (fraction of onsets on the beat-subdivision grid),
  `offbeat_ratio` (syncopation as a number), `onset_rate_per_s`, and the
  grid-based **`clear_rhythm`** that replaces the old confidence rule
  (moved here from `tempo`).
- Tempo with fewer than two detected onsets is now `null` — a
  sustained tone's window-sliding flux ripple is genuinely periodic and
  used to read confidence 0.99.
- The Ellis beat-DP envelope is normalized to unit std with the penalty
  rescaled accordingly; the unnormalized envelope made the period
  penalty negligible, so the "beat grid" snapped to every onset.
- Robustness, measured (`crates/features/tests/rhythm.rs`): ±10 ms
  humanized timing jitter keeps the exact BPM and `clear_rhythm`;
  ±20–30 ms octave-folds the BPM but keeps alignment 1.0 and
  `clear_rhythm`; a dropped plus an extra hit in 22 changes nothing;
  uniformly random onsets are rejected on two independent gates.
- `CompareReport` schema v2: `rhythm` delta section; `tempo` delta
  carries `stability_delta`.

### Added

- **Verify**: `GridAlignmentAtLeast(min)`, and the render-side sweep
  checks `BrightnessRises`/`BrightnessFalls(track, from, to, min_ratio)`
  — `Monotone` deliberately validates only the *authored* automation
  curve, so nothing previously verified a sweep audibly happened in the
  output; these listen to the stem's spectral centroid (new public
  `cochlea_features::spectral_centroid_curve`). Builder forms:
  `grid_alignment_at_least`, `brightness_rises`, `brightness_falls`.
- **Synth**: `kick` (pitch-drop sine) and `snare` (tonal body +
  counter-RNG noise burst) presets — eight total; `chord_pad` is now
  genuinely stereo (detuned saws pan ±0.35, constant-power).
  `drum_groove` uses the real kit, pans hats/snare via the engine's
  (previously never-exercised) `pan` automation, and asserts
  `HasClearRhythm(true)` with grid alignment ≥ 0.9. Golden re-blessed.
- **Self-describing authoring reference**
  (`cochlea_score::authoring_reference`): the RON grammar, the live
  preset catalog (generated from the registry, cannot go stale), every
  verify assertion, and a worked example that the test suite parses and
  renders. Served by the new `cochlea reference` subcommand, the new
  `score_reference` MCP tool, and the book's Score Format page (all
  three pinned to one generator by tests).
- **MCP**: `spectrogram` returns the image inline as MCP image content
  (base64 PNG, size-capped; `out_path` now optional) — clients without
  filesystem access get the one-vision-call review. `--root DIR`
  confinement: every path must canonically resolve inside the root or
  the call is refused before any filesystem work; the input-clobber
  guards now compare canonical paths, not strings.
- `cochlea_features::estimate_tempo_and_rhythm` (one shared analysis
  pass) and `cochlea_spectro::encode_png` (PNG to memory).

### Fixed

- Structure detection computes a *banded* self-similarity matrix — the
  novelty kernel never reads beyond ±16 frames, so the full O(n²) matrix
  (~415 MB for two hours of audio) was almost entirely waste — plus a
  deterministic frame-count cap; the old 1 ms `frame_ms` floor never
  actually bounded long files, despite its comment claiming so.
- `UnknownInstrument` errors list the bank's actual patch names instead
  of a hard-coded six-name string.

## [0.1.0-unversioned] — published to crates.io 2026-07-10 (see the 0.2.0 versioning note)

### Added — the agent read stack (v2, wave 1 + wave 2)

- **Segment timeline** (`cochlea-features`): windowed per-second feature
  timeline (RMS/peak dBFS, onset counts, silence flags, YIN f0 + nearest
  note, low/mid/high band energy) so an agent seeks inside audio by index,
  never by PCM.
- **Text digest** (`cochlea-features`, `cochlea probe --digest`): a
  deterministic plain-text digest of any audio file — a 3-minute WAV
  (~66 MB of PCM) reads as ~2.3 KB of text (measured, not estimated:
  36 bucketed timeline rows, well under the 40-row cap); the timeline
  table is row-capped, not duration-scaled, so this stays roughly flat
  for longer files too.
- **Audio diff** (`cochlea-features::compare`, `cochlea diff`):
  feature-space comparison with `byte-identical` / `tier2-equivalent` /
  `different (dimensions…)` verdicts, reusing the workspace's Tier-2
  tolerances (LUFS 0.1 LU, onsets 2 ms, pitch 5 cents); `--tier2` exits
  nonzero when not equivalent.
- **MCP server** (`cochlea-mcp`): stdio JSON-RPC 2.0 server (hand-rolled,
  no async runtime) exposing `render_score`, `probe_audio`, `spectrogram`,
  `lint_score`, `probe_digest`, and `audio_diff` as agent tools; see
  `docs/mcp.md`.
- **FLAC input** (`cochlea-decode`): lossless decode via pure-Rust
  symphonia, verified bit-exact against WAV twins; `probe`, `diff`,
  `spectro`, and every MCP tool accept `.flac` alongside `.wav`. Still
  ffmpeg-free.
- **Tempo/beat tracking** (`cochlea-features::tempo`): onset-envelope
  autocorrelation with a log-Gaussian tempo prior and an Ellis-2007-style
  DP beat grid; reports BPM, a beat grid, a 0–1 confidence, and a
  calibrated `clear_rhythm` boolean an agent can trust.
- **Stereo image + loudness range** (`cochlea-features`): width /
  correlation / balance, and EBU R128 LRA.
- **Structure detection** (`cochlea-features`): self-similarity novelty
  section boundaries (Foote checkerboard-kernel novelty over per-second
  chroma+band+RMS vectors).
- **Report schema v2**: probe JSON now carries tempo (bpm / confidence /
  `clear_rhythm` / beat count), stereo image, structure, and LRA; the
  digest surfaces the same. Five new RON-embeddable verify assertions
  (`TempoIs`, `HasClearRhythm`, `StereoWidthWithin`, `LraBelow`,
  `SectionCount`) and a fourth demo (`drum_groove`, 110 BPM) with its own
  golden PCM hash.
- Per-crate READMEs and crates.io metadata for the whole workspace.

## [0.1.0] — 2026-07-03

Initial complete v1: an audio engine with no human ear in the loop.

- `cochlea-score`: declarative score IR — integer ticks at 960 PPQ, tempo
  map, bar/beat builder, typed params, RON data form (`version: 1`) with
  embeddable `verify:` assertions.
- `cochlea-synth`: six presets over fundsp (`sine`, `saw_lead`,
  `square_bass`, `chord_pad`, `noise_hat`, `pluck`) plus an in-repo
  Schroeder reverb insert; counter-based RNG keyed `(seed, sample_index)`.
- `cochlea-render`: 64-sample block engine split at event boundaries,
  sample-accurate note scheduling, pure voice allocation/stealing,
  per-track stems, f64 master sum, WAV out.
- `cochlea-features`: schema-versioned JSON probe — integrated LUFS /
  momentary max / true peak (ebur128), spectral-flux onsets, YIN pitch,
  chroma + Krumhansl-Schmuckler key, silence/tail, clipping.
- `cochlea-spectro`: mel spectrogram PNGs (hand-rolled HTK-style
  filterbank — a documented choice over Slaney's, see the crate's
  `mel` module — viridis, time ruler, caller-supplied markers), tiled
  contact sheets, image diff for Tier-3 sentinels.
- `cochlea-verify`: assertion DSL over renders (`integrated_lufs`,
  `true_peak_below`, `onset_at`, `pitch_matches_score`, `monotone`,
  `no_discontinuity`, `silent_after`) with JSON reports.
- `cochlea` CLI: `render` / `probe` / `lint` / `spectro`; exit codes
  0 ok, 1 assertion failures, 2 usage/IO.
- The three-tier determinism contract, CI-enforced: byte-identical PCM on
  the pinned target (libm-only transcendentals via clippy bans, no
  fast-math, no implicit FMA, denormals honored, fixed summation order),
  cross-platform feature tolerances, spectrogram sentinels. Confirmed
  byte-identical across aarch64-macos and x86_64-linux by CI.
