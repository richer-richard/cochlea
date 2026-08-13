"""End-to-end tests for the cochlea Python bindings: render a score, probe it,
and assert — the whole listen-and-assert loop through Python."""

import shutil

import pytest

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


def test_spectrogram_refuses_to_overwrite_its_input(tmp_path):
    # The whole file is decoded before a byte of PNG is written, so an
    # aliasing out_path destroys the audio and reports success. Decode
    # identifies a file by its magic bytes, so a WAV *named* .png is the
    # case that actually lands: the PNG encoder is happy with the extension
    # and nothing else stood between them.
    wav = tmp_path / "take.wav"
    cochlea.render(SINE_A4, str(wav))
    disguised = tmp_path / "take.png"
    shutil.copy(wav, disguised)
    before = disguised.read_bytes()

    with pytest.raises(ValueError):
        cochlea.spectrogram(str(disguised), str(disguised))
    assert disguised.read_bytes() == before

    # And it still renders normally to a path of its own.
    png = tmp_path / "spec.png"
    cochlea.spectrogram(str(wav), str(png))
    assert png.read_bytes()[:4] == b"\x89PNG"


def test_pytest_fixture(assert_audio, tmp_path):
    out = str(tmp_path / "sine.wav")
    cochlea.render(SINE_A4, out)
    assert_audio(out).true_peak_below(0.0)
