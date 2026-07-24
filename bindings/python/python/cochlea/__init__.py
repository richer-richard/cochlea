"""cochlea — deterministic ears and golden-audio tests for agents and audio
pipelines.

Probe any WAV/FLAC/mp3/ogg into a few KB of feature data, diff two renders in
feature space, render a RON score to deterministic PCM, and assert what you
heard — all backed by the byte-reproducible Rust core.

    from cochlea import probe, assert_audio

    report = probe("mix.wav")            # full feature report as a dict
    assert_audio("mix.wav").true_peak_below(-1.0).key_is("C", "major")
"""

from ._cochlea import (
    __version__,
    diff,
    probe,
    probe_digest,
    render,
    samples_identical,
    spectrogram,
    version,
)
from .assertions import AudioAssertion, assert_audio

__all__ = [
    "__version__",
    "assert_audio",
    "AudioAssertion",
    "diff",
    "probe",
    "probe_digest",
    "render",
    "samples_identical",
    "spectrogram",
    "version",
]
