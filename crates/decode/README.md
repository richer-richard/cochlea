# cochlea-decode

Audio file input for
[cochlea](https://github.com/richer-richard/cochlea): WAV (hound),
FLAC (pure-Rust symphonia, lossless — decoded PCM verified bit-exact
against WAV twins in-tree), and mp3/ogg-vorbis (symphonia, analysis
input only — reproducible per build, but no exactness claim once a codec
has discarded the original samples) to a single `Audio` buffer. No
ffmpeg, no system codecs, no subprocesses.

```rust
let audio = cochlea_decode::load(std::path::Path::new("input.flac"))?;
```

Depends only on `cochlea-features` (for the `Audio` type) — never the
score IR or synth. Docs: <https://richer-richard.github.io/cochlea/>.

License: MIT OR Apache-2.0.
