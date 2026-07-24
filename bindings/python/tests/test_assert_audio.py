"""End-to-end tests for the cochlea Python bindings: render a score, probe it,
and assert — the whole listen-and-assert loop through Python."""

import cochlea

# A one-bar A4 whole note on the sine preset — the simplest thing to render
# and read back a pitch from.
SINE_A4 = """Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: "lead", instrument: Preset("sine"),
        notes: [ Note(at: (1, 1), dur: "1/1", pitch: "A4", vel: 100) ]) ],
)"""

# A I-IV-V-I in C major on the pad — for the harmony assertions.
CHORDS = """Score(
    version: 1, sample_rate: 48000, ppq: 960, time_signature: (4, 4),
    tempo: [(tick: 0, bpm: 120.0)],
    tracks: [ Track(name: "pad", instrument: Preset("chord_pad"), notes: [
        Note(at: (1, 1), dur: "1/1", pitch: "C4", vel: 90),
        Note(at: (1, 1), dur: "1/1", pitch: "E4", vel: 90),
        Note(at: (1, 1), dur: "1/1", pitch: "G4", vel: 90),
        Note(at: (2, 1), dur: "1/1", pitch: "C4", vel: 88),
        Note(at: (2, 1), dur: "1/1", pitch: "F4", vel: 88),
        Note(at: (2, 1), dur: "1/1", pitch: "A4", vel: 88),
    ]) ],
)"""


def test_render_probe_and_assert(tmp_path):
    out = str(tmp_path / "sine.wav")
    cochlea.render(SINE_A4, out)

    report = cochlea.probe(out)
    assert report["schema_version"] == 5
    assert report["source"]["sample_rate"] == 48000

    cochlea.assert_audio(out).pitch_matches("A4", tol_cents=30).not_clipping()


def test_digest_is_text(tmp_path):
    out = str(tmp_path / "sine.wav")
    cochlea.render(SINE_A4, out)
    digest = cochlea.probe_digest(out)
    assert "cochlea digest" in digest
    assert "key:" in digest


def test_bit_depths(tmp_path):
    for bits in ("float", "24", "16"):
        out = str(tmp_path / f"sine_{bits}.wav")
        cochlea.render(SINE_A4, out, bits=bits)
        cochlea.assert_audio(out).duration_between(1.5, 3.0)


def test_diff_identical_is_exact(tmp_path):
    a, b = str(tmp_path / "a.wav"), str(tmp_path / "b.wav")
    cochlea.render(SINE_A4, a)
    cochlea.render(SINE_A4, b)
    assert cochlea.samples_identical(a, b)
    cochlea.assert_audio(a).matches(b, tier="exact")


def test_harmony_progression(tmp_path):
    out = str(tmp_path / "chords.wav")
    cochlea.render(CHORDS, out)
    cochlea.assert_audio(out).has_chord("C").has_chord("F")


def test_pytest_fixture(assert_audio, tmp_path):
    out = str(tmp_path / "sine.wav")
    cochlea.render(SINE_A4, out)
    assert_audio(out).true_peak_below(0.0)
