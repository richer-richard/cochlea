# cochlea (CLI)

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
cochlea reference                         # the full score-authoring reference
```

`probe`/`diff`/`spectro` work on any WAV, FLAC, mp3, or ogg — no score
required.
Exit codes: 0 ok, 1 assertion failures, 2 usage/IO.

Docs: <https://richer-richard.github.io/cochlea/>. License: MIT OR Apache-2.0.
