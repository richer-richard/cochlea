# cochlea-render

The offline block engine of
[cochlea](https://github.com/richer-richard/cochlea): renders a
`cochlea-score` to deterministic PCM — 64-sample blocks split at event
boundaries (note timing is sample-accurate; automation is control-rate),
pure voice allocation and oldest-note stealing, per-track stems, an f64
master sum in fixed track order, and WAV out. Byte-identical output for
identical inputs on the pinned CI target, enforced by golden PCM hashes.

```rust
let rendered = cochlea_render::render(&score)?;
rendered.write_wav("mix.wav")?;
```

There is no realtime path, and there never will be one — offline render
is ground truth. Docs: <https://richer-richard.github.io/cochlea/>.

License: MIT OR Apache-2.0.
