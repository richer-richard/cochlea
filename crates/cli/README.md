# cochlea (CLI)

![cochlea: the wordmark filled with a mel spectrogram of first_light.ron, over an agent session that renders, probes, and verifies the score](https://raw.githubusercontent.com/richer-richard/cochlea/main/docs/assets/cover.png)

The command-line front end of
[cochlea](https://github.com/richer-richard/cochlea), a headless
deterministic audio engine for AI agents — compose a score as data,
render it offline to byte-identical PCM, then "listen" through numbers
and images:

```
cochlea render score.ron --out mix.wav --stems stems/ --verify
cochlea probe input.wav --json report.json --spectro spec.png
cochlea probe input.mp3 --digest --from 42 --to 60   # LLM-sized digest of a window
cochlea diff a.wav b.wav --tier2 --spectro delta.png # equivalence gate + heat map
cochlea lint score.ron
cochlea spectro input.wav --out spec.png --annotate  # beats/onsets/pitch drawn on
cochlea import song.mid --out score.ron              # SMF -> score, timing exact
cochlea transcribe solo.wav --out score.ron          # audio -> score, the inverse of render
cochlea reference                         # the full score-authoring reference
```

`probe`/`diff`/`spectro` work on any WAV, FLAC, mp3, or ogg — no score
required.
Exit codes: 0 ok, 1 assertion failures, 2 usage/IO.

Docs: <https://richer-richard.github.io/cochlea/>. License: MIT OR Apache-2.0.
