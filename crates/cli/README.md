# cochlea (CLI)

The command-line front end of
[cochlea](https://github.com/richer-richard/cochlea), a headless
deterministic audio engine for AI agents — compose a score as data,
render it offline to byte-identical PCM, then "listen" through numbers
and images:

```
cochlea render score.ron --out mix.wav --stems stems/ --verify
cochlea probe input.wav --json report.json --spectro spec.png
cochlea probe input.wav --digest          # LLM-sized text digest
cochlea diff a.wav b.wav --tier2          # feature-space equivalence gate
cochlea lint score.ron
cochlea spectro input.wav --out spec.png --sheet
cochlea reference                         # the full score-authoring reference
```

`probe`/`diff`/`spectro` work on any WAV or FLAC — no score required.
Exit codes: 0 ok, 1 assertion failures, 2 usage/IO.

Docs: <https://richer-richard.github.io/cochlea/>. License: MIT OR Apache-2.0.
