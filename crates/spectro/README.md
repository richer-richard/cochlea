# cochlea-spectro

Mel spectrograms as small PNGs, for agents that review audio with a
single vision call: hand-rolled HTK-style mel filterbank over
`rustfft`'s scalar planner (no runtime CPU dispatch — deterministic),
viridis LUT, time ruler, caller-supplied bar markers, analysis
overlays (beat/onset/pitch marks as plain data drawn onto the image),
signed A→B difference heat maps, tiled contact sheets for whole-piece
review, in-memory PNG encoding, and an image differ for sentinel
testing.

Part of [cochlea](https://github.com/richer-richard/cochlea); depends on
neither the score IR nor the synth, so it works on any decoded audio.
Docs: <https://richer-richard.github.io/cochlea/>.

License: MIT OR Apache-2.0.
