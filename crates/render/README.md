# cochlea-render

The offline block engine of
[cochlea](https://github.com/richer-richard/cochlea): renders a
`cochlea-score` to deterministic PCM — 64-sample blocks split at event
boundaries (note timing is sample-accurate; automation is control-rate),
pure voice allocation and oldest-note stealing, per-track stems, an f64
master sum in fixed track order, an optional master stage (output gain
plus a brick-wall lookahead limiter whose sample-peak ceiling holds
exactly; byte-inert when the score has no master section), and WAV out. Byte-identical output for
identical inputs on the pinned CI target, enforced by golden PCM hashes.

```rust
let rendered = cochlea_render::render(&score)?;
rendered.write_wav("mix.wav")?;
```

Stems are written as `<dir>/<track>.wav`, which turns a track name — free-form
score data, and something `cochlea import` can lift straight out of a MIDI
file — into a file name. `stem_file_name` is the one rule for that: a single
portable file name, with separators, `:`, `<>"|?*`, control characters and the
Windows device names refused on every platform so a score exports the same
stems on every host. `write_stems_as` applies it to every track, rejects two
names that differ only by case, and checks where each stem path actually lands
before writing, so a symlink can't redirect a stem out of the directory —
including a *broken* one, which `Path::exists()` reports as absent while
`File::create` follows it anyway. A link that cannot be resolved at all is
refused rather than guessed at. If you build stem paths yourself from
`Rendered::stems()`, go through `stem_file_name` rather than joining the name
directly, and compare the result against your other outputs with `same_file`,
which resolves both sides before applying the rule (two paths differing only by
case are one file on macOS and Windows; so are two spellings of one path).

There is no realtime path, and there never will be one — offline render
is ground truth. Docs: <https://richer-richard.github.io/cochlea/>.

License: MIT OR Apache-2.0.
