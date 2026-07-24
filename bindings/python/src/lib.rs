//! Python bindings for cochlea — a thin pyo3 layer over the pure-Rust core.
//!
//! The determinism contract lives in the Rust crates; this layer only
//! marshals arguments and returns their results as Python objects (JSON
//! reports come back as `dict`, digests as `str`). Nothing here re-implements
//! any DSP — it is the reach layer, not the engine. The ergonomic
//! `assert_audio(...)` fluent API and the pytest plugin are pure Python on top
//! of these primitives (see `python/cochlea/`).

use std::path::Path;

use pyo3::exceptions::{PyIOError, PyValueError};
use pyo3::prelude::*;

/// Map any `Display` error into a Python `ValueError`.
fn value_err<E: std::fmt::Display>(err: E) -> PyErr {
    PyValueError::new_err(err.to_string())
}

/// Turn a serde-serializable value into a Python object by round-tripping
/// through JSON (`serde_json::Value` → `json.loads`), so the report's exact
/// field names and shapes reach Python as ordinary dicts/lists.
fn to_py<T: serde::Serialize>(py: Python<'_>, value: &T) -> PyResult<Py<PyAny>> {
    let json = serde_json::to_string(value).map_err(value_err)?;
    let json_mod = py.import("json")?;
    let obj = json_mod.call_method1("loads", (json,))?;
    Ok(obj.into())
}

/// Load an audio file into `Audio`, mapping decode errors to Python.
fn load_audio(path: &str) -> PyResult<cochlea_features::Audio> {
    cochlea_decode::load(Path::new(path)).map_err(|e| PyIOError::new_err(e.to_string()))
}

/// Render a RON score string to a stereo WAV file. `bits` is `"float"`
/// (default, lossless), `"24"`, or `"16"`.
#[pyfunction]
#[pyo3(signature = (score_ron, out_path, bits = "float"))]
fn render(score_ron: &str, out_path: &str, bits: &str) -> PyResult<()> {
    let depth = match bits {
        "float" | "f32" | "32" => cochlea_render::WavBitDepth::Float32,
        "24" => cochlea_render::WavBitDepth::Int24,
        "16" => cochlea_render::WavBitDepth::Int16,
        other => return Err(value_err(format!("unknown bits {other:?}"))),
    };
    let score = cochlea_score::Score::from_ron(score_ron).map_err(value_err)?;
    let rendered = cochlea_render::render(&score).map_err(value_err)?;
    rendered
        .write_wav_as(out_path, depth)
        .map_err(|e| PyIOError::new_err(e.to_string()))
}

/// Probe an audio file (WAV/FLAC/mp3/ogg) into the full feature report as a
/// Python dict (schema-versioned; the same JSON `cochlea probe` emits).
#[pyfunction]
fn probe(py: Python<'_>, path: &str) -> PyResult<Py<PyAny>> {
    let audio = load_audio(path)?;
    let report = cochlea_features::probe(&audio, &cochlea_features::ProbeOpts::default());
    to_py(py, &report)
}

/// Probe an audio file into the compact, token-cheap text digest.
#[pyfunction]
#[pyo3(signature = (path, window_ms = 1000.0))]
fn probe_digest(path: &str, window_ms: f64) -> PyResult<String> {
    let audio = load_audio(path)?;
    let report = cochlea_features::probe(&audio, &cochlea_features::ProbeOpts::default());
    let timeline = cochlea_features::segment_timeline(
        &audio,
        &cochlea_features::SegmentOpts::default().with_window_ms(window_ms),
    );
    Ok(cochlea_features::digest_text(&report, &timeline))
}

/// Compare two audio files in feature space, returning the `CompareReport` as
/// a dict (verdict, plus per-dimension deltas).
#[pyfunction]
#[pyo3(signature = (path_a, path_b, window_ms = 1000.0))]
fn diff(py: Python<'_>, path_a: &str, path_b: &str, window_ms: f64) -> PyResult<Py<PyAny>> {
    let a = load_audio(path_a)?;
    let b = load_audio(path_b)?;
    let opts = cochlea_features::SegmentOpts::default().with_window_ms(window_ms);
    let report_a = cochlea_features::probe(&a, &cochlea_features::ProbeOpts::default());
    let report_b = cochlea_features::probe(&b, &cochlea_features::ProbeOpts::default());
    let tl_a = cochlea_features::segment_timeline(&a, &opts);
    let tl_b = cochlea_features::segment_timeline(&b, &opts);
    let byte_identical = cochlea_features::samples_identical(&a, &b);
    let compare = cochlea_features::compare_with_identity(
        cochlea_features::Analysis {
            report: &report_a,
            timeline: &tl_a,
        },
        cochlea_features::Analysis {
            report: &report_b,
            timeline: &tl_b,
        },
        byte_identical,
    );
    to_py(py, &compare)
}

/// Render a mel-spectrogram PNG of an audio file to `out_path`.
#[pyfunction]
fn spectrogram(path: &str, out_path: &str) -> PyResult<()> {
    let audio = load_audio(path)?;
    let spec = cochlea_spectro::mel_spectrogram(
        &audio.samples,
        audio.channels,
        audio.sample_rate,
        &cochlea_spectro::SpectroOpts::new(),
    );
    let img = cochlea_spectro::render_png(&spec, &[]);
    cochlea_spectro::write_png(&img, Path::new(out_path))
        .map_err(|e| PyIOError::new_err(e.to_string()))
}

/// Whether two audio files are byte-identical in their decoded samples — the
/// strongest (Tier-1) equality, exposed for golden tests that want exactness.
#[pyfunction]
fn samples_identical(path_a: &str, path_b: &str) -> PyResult<bool> {
    let a = load_audio(path_a)?;
    let b = load_audio(path_b)?;
    Ok(cochlea_features::samples_identical(&a, &b))
}

/// cochlea's version, so a test suite can record which engine produced a
/// golden.
#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The native extension module. The ergonomic `assert_audio` fluent API and
/// the pytest plugin are pure Python wrappers around these functions.
#[pymodule]
fn _cochlea(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(render, m)?)?;
    m.add_function(wrap_pyfunction!(probe, m)?)?;
    m.add_function(wrap_pyfunction!(probe_digest, m)?)?;
    m.add_function(wrap_pyfunction!(diff, m)?)?;
    m.add_function(wrap_pyfunction!(spectrogram, m)?)?;
    m.add_function(wrap_pyfunction!(samples_identical, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add("__version__", version())?;
    Ok(())
}
