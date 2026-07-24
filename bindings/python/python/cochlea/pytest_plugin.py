"""pytest plugin: an ``assert_audio`` fixture so audio regression tests read
naturally, auto-registered via the ``pytest11`` entry point when both cochlea
and pytest are installed.

    def test_render_stays_in_spec(assert_audio):
        assert_audio("build/mix.wav").true_peak_below(-1.0).not_clipping()
"""

from __future__ import annotations

import pytest

from .assertions import assert_audio as _assert_audio


@pytest.fixture
def assert_audio():
    """Return the ``assert_audio`` fluent entry point (see
    :func:`cochlea.assert_audio`)."""
    return _assert_audio
