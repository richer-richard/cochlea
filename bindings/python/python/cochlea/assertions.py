"""The ``assert_audio`` fluent API: audio regression checks that read like
ordinary test assertions.

    from cochlea import assert_audio

    assert_audio("out.wav") \\
        .true_peak_below(-1.0) \\
        .key_is("C", "major") \\
        .has_chord("G") \\
        .not_clipping()

Every method reads the deterministic probe report (computed once, lazily,
cached) and raises ``AssertionError`` with a specific message on mismatch, or
returns ``self`` so checks chain. This is a thin, Pythonic layer over the
Rust core — the numbers it asserts against are byte-reproducible.
"""

from __future__ import annotations

import math
from typing import Optional

from . import diff as _diff
from . import probe as _probe

_NOTE_SEMITONES = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}


def _note_to_hz(note: str) -> float:
    """Parse a note name like ``A4``, ``C#3``, ``Bb2`` into its frequency."""
    name = note.strip()
    if not name or name[0].upper() not in _NOTE_SEMITONES:
        raise ValueError(f"unparseable note name: {note!r}")
    semitone = _NOTE_SEMITONES[name[0].upper()]
    i = 1
    while i < len(name) and name[i] in "#b":
        semitone += 1 if name[i] == "#" else -1
        i += 1
    try:
        octave = int(name[i:])
    except ValueError as exc:
        raise ValueError(f"unparseable note name: {note!r}") from exc
    midi = (octave + 1) * 12 + semitone
    return 440.0 * (2.0 ** ((midi - 69) / 12.0))


class AudioAssertion:
    """A fluent set of assertions over one audio file's probe report."""

    def __init__(self, path: str):
        self.path = path
        self._report: Optional[dict] = None

    @property
    def report(self) -> dict:
        if self._report is None:
            self._report = _probe(self.path)
        return self._report

    # ---- loudness ----

    def true_peak_below(self, dbtp: float) -> "AudioAssertion":
        tp = self.report["loudness"]["true_peak_dbtp"]
        assert tp is not None, f"{self.path}: no true-peak reading (silence?)"
        assert tp < dbtp, f"{self.path}: true peak {tp:.2f} dBTP not below {dbtp}"
        return self

    def integrated_lufs(self, target: float, tol: float = 1.0) -> "AudioAssertion":
        lufs = self.report["loudness"]["integrated_lufs"]
        assert lufs is not None, f"{self.path}: no integrated loudness (silence?)"
        assert abs(lufs - target) <= tol, (
            f"{self.path}: integrated {lufs:.2f} LUFS not within {tol} of {target}"
        )
        return self

    def not_clipping(self) -> "AudioAssertion":
        clip = self.report["clipping"]
        assert clip["clipped_samples"] == 0, (
            f"{self.path}: {clip['clipped_samples']} clipped samples"
        )
        assert not clip["true_peak_over_0dbtp"], f"{self.path}: true peak over 0 dBTP"
        return self

    # ---- pitch / key / harmony ----

    def pitch_matches(self, note: str, tol_cents: float = 50.0) -> "AudioAssertion":
        f0 = self.report["pitch"]["median_f0_hz"]
        assert f0 is not None, f"{self.path}: no voiced pitch detected"
        target = _note_to_hz(note)
        cents = 1200.0 * math.log2(f0 / target)
        assert abs(cents) <= tol_cents, (
            f"{self.path}: median pitch {f0:.1f} Hz is {cents:+.1f} cents from {note} "
            f"(> {tol_cents})"
        )
        return self

    def key_is(self, tonic: str, mode: str) -> "AudioAssertion":
        key = self.report["key"]
        assert key["tonic"] == tonic and key["mode"] == mode, (
            f"{self.path}: key is {key['tonic']} {key['mode']}, expected {tonic} {mode}"
        )
        return self

    def has_chord(self, symbol: str) -> "AudioAssertion":
        chords = self.report["harmony"]["chords"]
        symbols = [c["symbol"] for c in chords]
        assert symbol in symbols, (
            f"{self.path}: chord {symbol!r} not among detected {symbols}"
        )
        return self

    def progression(self, expected: list[str]) -> "AudioAssertion":
        """Assert the detected chord symbols contain ``expected`` as an ordered
        subsequence (extra chords between them are allowed)."""
        symbols = [c["symbol"] for c in self.report["harmony"]["chords"]]
        it = iter(symbols)
        matched = all(any(s == want for s in it) for want in expected)
        assert matched, f"{self.path}: {symbols} does not contain progression {expected}"
        return self

    # ---- tempo / rhythm ----

    def tempo_near(self, bpm: float, tol: float = 2.0) -> "AudioAssertion":
        got = self.report["tempo"]["bpm"]
        assert got is not None, f"{self.path}: no tempo detected"
        assert abs(got - bpm) <= tol, (
            f"{self.path}: tempo {got:.1f} BPM not within {tol} of {bpm}"
        )
        return self

    def has_clear_rhythm(self) -> "AudioAssertion":
        assert self.report["rhythm"]["clear_rhythm"], (
            f"{self.path}: rhythm is not clear (onsets don't align to a grid)"
        )
        return self

    # ---- structure / duration ----

    def duration_between(self, lo_s: float, hi_s: float) -> "AudioAssertion":
        dur = self.report["source"]["duration_ms"] / 1000.0
        assert lo_s <= dur <= hi_s, (
            f"{self.path}: duration {dur:.3f} s not in [{lo_s}, {hi_s}]"
        )
        return self

    def section_count(self, count: int) -> "AudioAssertion":
        got = self.report["structure"]["section_count"]
        assert got == count, f"{self.path}: {got} sections, expected {count}"
        return self

    # ---- golden comparison ----

    def matches(self, reference: str, tier: str = "tier2") -> "AudioAssertion":
        """Assert this file is byte-identical (``tier='exact'``) or at least
        Tier-2 equivalent (``tier='tier2'``, the default) to ``reference`` in
        feature space — the golden-audio regression check."""
        report = _diff(self.path, reference)
        # The verdict is internally tagged: {"kind": "ByteIdentical" | ...}.
        verdict = report["verdict"]["kind"]
        if tier == "exact":
            ok = verdict == "ByteIdentical"
        else:
            ok = verdict in ("ByteIdentical", "Tier2Equivalent")
        assert ok, (
            f"{self.path} vs {reference}: verdict {verdict!r} fails tier {tier!r}"
        )
        return self


def assert_audio(path: str) -> AudioAssertion:
    """Begin a chain of assertions over ``path`` (WAV/FLAC/mp3/ogg)."""
    return AudioAssertion(path)
