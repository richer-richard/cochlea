# cochlea (Python)

Deterministic ears and golden-audio tests for agents and audio pipelines —
the Python reach layer over [cochlea](https://github.com/richer-richard/cochlea)'s
byte-reproducible Rust core.

Probe any WAV/FLAC/mp3/ogg into a few KB of feature data, diff two renders in
feature space, render a RON score to deterministic PCM, and **assert what you
heard** — with no human ear in the loop.

```python
from cochlea import probe, assert_audio

report = probe("mix.wav")          # full feature report as a dict
print(report["key"]["tonic"], report["tempo"]["bpm"])

assert_audio("mix.wav") \
    .true_peak_below(-1.0) \
    .key_is("C", "major") \
    .has_chord("G") \
    .not_clipping()
```

The whole point is that these numbers are **deterministic**: the same input
gives byte-identical PCM and the same feature report on the pinned target, so
an assertion that passes today isn't flaky tomorrow.

## Install

```
pip install cochlea            # prebuilt wheels where available
# or, from source (needs a Rust toolchain):
pip install maturin && maturin develop -m bindings/python/Cargo.toml
```

## The golden-audio test pattern

Check a reference render into your repo, then fail CI when a change moves the
audio outside Tier-2 tolerances:

```python
def test_tts_output_has_not_regressed():
    synth("hello world", "out.wav")
    assert_audio("out.wav").matches("golden/hello_world.wav")   # tier-2 by default
```

`matches(..., tier="exact")` demands byte-identity; the default `tier="tier2"`
allows cross-platform feature tolerances (integrated LUFS within 0.1 LU, onsets
within 2 ms, pitch within 5 cents).

## pytest plugin

Installing cochlea alongside pytest registers an `assert_audio` fixture:

```python
def test_render_stays_in_spec(assert_audio):
    assert_audio("build/mix.wav").true_peak_below(-1.0).not_clipping()
```

## API

| function | returns |
|---|---|
| `probe(path)` | full feature report (`dict`) |
| `probe_digest(path, window_ms=1000)` | compact text digest (`str`) |
| `diff(a, b, window_ms=1000)` | feature-space comparison report (`dict`) |
| `render(score_ron, out_path, bits="float")` | writes a WAV (`float`/`24`/`16`) |
| `spectrogram(path, out_path)` | writes a mel-spectrogram PNG |
| `samples_identical(a, b)` | byte-identity of decoded samples (`bool`) |
| `assert_audio(path)` | fluent assertion chain (below) |

**Assertions** (each returns `self`, raising `AssertionError` on mismatch):
`true_peak_below`, `integrated_lufs`, `not_clipping`, `pitch_matches`,
`key_is`, `has_chord`, `progression`, `tempo_near`, `has_clear_rhythm`,
`duration_between`, `section_count`, `matches`.

License: MIT OR Apache-2.0.
