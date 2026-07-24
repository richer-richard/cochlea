# Golden-audio testing

The core idea behind cochlea: audio is too big and too opaque to diff by eye or
by byte, but its *features* are small, meaningful, and — on the pinned target —
byte-reproducible. That makes audio testable the way any other output is
testable: render (or synthesize) it, compare against a checked-in reference,
and fail the build when it moves outside tolerance.

This is the **golden-audio** pattern, and it's what cochlea is really for. No
human ear in the loop.

## The three tiers of "did it change"

cochlea answers "did this audio change" at three levels of strictness (see
[determinism.md](determinism.md) for the full contract):

- **Tier 1 — byte-identical PCM.** The exact same samples. Use this for a
  deterministic renderer on a fixed target: any difference is a real change.
- **Tier 2 — feature-equivalent.** The audio measures the same within
  cross-platform tolerances (integrated LUFS within 0.1 LU, onsets within
  2 ms, pitch within 5 cents). Use this across platforms, or for a model whose
  output isn't bit-exact but should stay perceptually stable.
- **Tier 3 — spectrogram sentinel.** An image diff of the mel spectrogram with
  a pixel tolerance, for a visual regression check.

## `cochlea diff` — compare two files

```
cochlea diff candidate.wav golden.wav --tier2
```

Exit code `0` means byte-identical *or* Tier-2 equivalent; exit code `1` means
they differ. That single exit-code gate is all a CI step needs. Add `--json
report.json` for the full per-dimension comparison, or `--spectro diff.png` for
a signed A→B difference heat map (red = louder in B, blue = quieter).

## `cochlea eval` — score a directory of outputs

For evaluating a generative model (TTS, voice, music-gen), you usually have a
*directory* of outputs to check against a directory of references. `cochlea
eval` does the whole set in one deterministic pass:

```
cochlea eval --candidates out/ --references golden/ --json eval.json
```

It matches files by name, compares each pair, prints a per-file verdict table
and an aggregate pass rate, and exits `1` if any pair regressed (or a reference
is missing). Add `--exact` to demand byte-identity instead of Tier-2
equivalence. This is a reference-render regression oracle for an audio model or
DSP library: check the golden set in once, and every change is scored against
it with no listening and no flakiness.

## In Python (pytest)

The [Python bindings](https://pypi.org/project/cochlea/) turn the same check
into an ordinary assertion:

```python
from cochlea import assert_audio

def test_tts_output_has_not_regressed():
    synth("hello world", "out.wav")
    assert_audio("out.wav").matches("golden/hello_world.wav")   # tier-2
```

or, checking properties instead of a golden:

```python
assert_audio("out.wav").true_peak_below(-1.0).pitch_matches("A4").not_clipping()
```

## In CI (GitHub Actions)

The bundled composite action wraps `cochlea eval`:

```yaml
# .github/workflows/audio-regression.yml
name: audio regression
on: [pull_request]
jobs:
  golden:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo run -p cochlea -- render examples/scores/first_light.ron --out /tmp/out.wav
      - uses: ./.github/actions/golden-audio
        with:
          candidates: /tmp
          references: examples/golden
          tier: tier2
```

See [`.github/actions/golden-audio/action.yml`](../.github/actions/golden-audio/action.yml).

## Blessing a golden

When a change to the audio is *intended*, re-bless the reference deliberately —
regenerate the golden file and commit it with a note on why the sound changed.
For cochlea's own render goldens that's `cochlea render … --out golden.wav`; for
the internal PCM-hash and spectrogram sentinels see the "Blessing goldens" notes
in the project README. The discipline is the same everywhere: a golden only
changes when a human decides the new sound is correct.
