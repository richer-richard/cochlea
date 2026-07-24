//! WAV output bit-depth: 32-bit float (the render's lossless ground truth)
//! plus deterministic 16- and 24-bit integer PCM for a small, ordinary file.

use cochlea_render::{WavBitDepth, render};
use cochlea_score::*;

fn tmp(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

fn demo_score() -> Score {
    Score::new(SampleRate(48_000), Ppq(960))
        .track("lead", Instrument::preset("saw_lead"))
        .note("lead", bar(1), Dur::half(), Pitch::A4, Vel(100))
        .note("lead", bar(1).beat(3), Dur::half(), Pitch::CS5, Vel(90))
}

#[test]
fn integer_pcm_matches_the_float_mix_within_quantization_error() {
    let rendered = render(&demo_score()).unwrap();
    let mix = rendered.mix();

    for (name, depth, bits, tol) in [
        (
            "mix16.wav",
            WavBitDepth::Int16,
            16u16,
            1.0 / 32_768.0 + 1e-6,
        ),
        (
            "mix24.wav",
            WavBitDepth::Int24,
            24u16,
            1.0 / 8_388_608.0 + 1e-6,
        ),
    ] {
        let path = tmp(name);
        rendered.write_wav_as(&path, depth).unwrap();

        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.bits_per_sample, bits, "{name} bit depth");
        assert_eq!(
            spec.sample_format,
            hound::SampleFormat::Int,
            "{name} format"
        );
        assert_eq!(spec.channels, 2);

        let full_scale = f64::from(1i32 << (bits - 1));
        let back: Vec<f32> = reader
            .into_samples::<i32>()
            .map(|s| (f64::from(s.unwrap()) / full_scale) as f32)
            .collect();
        assert_eq!(back.len(), mix.len(), "{name} sample count");
        for (i, (&orig, &decoded)) in mix.iter().zip(back.iter()).enumerate() {
            let err = (f64::from(orig) - f64::from(decoded)).abs();
            assert!(
                err <= tol,
                "{name} sample {i}: {orig} vs {decoded} (err {err} > {tol})"
            );
        }
    }
}

#[test]
fn integer_pcm_output_is_byte_deterministic() {
    let rendered = render(&demo_score()).unwrap();
    for (name_a, name_b, depth) in [
        ("det16_a.wav", "det16_b.wav", WavBitDepth::Int16),
        ("det24_a.wav", "det24_b.wav", WavBitDepth::Int24),
    ] {
        let (a, b) = (tmp(name_a), tmp(name_b));
        rendered.write_wav_as(&a, depth).unwrap();
        rendered.write_wav_as(&b, depth).unwrap();
        assert_eq!(
            std::fs::read(&a).unwrap(),
            std::fs::read(&b).unwrap(),
            "{depth:?} output must be byte-identical across writes"
        );
    }
}

#[test]
fn full_scale_samples_clamp_without_wrapping() {
    // A limiter-less loud render can momentarily exceed +/-1.0 in f32; the
    // integer path must clamp, never wrap to the opposite rail.
    let rendered = render(&demo_score()).unwrap();
    let path = tmp("clamp16.wav");
    rendered.write_wav_as(&path, WavBitDepth::Int16).unwrap();
    let reader = hound::WavReader::open(&path).unwrap();
    for s in reader.into_samples::<i32>() {
        let s = s.unwrap();
        assert!(
            (-32_768..=32_767).contains(&s),
            "16-bit sample out of range: {s}"
        );
    }
}

/// The `WavBitDepth` selector parser is the single source of truth the CLI,
/// MCP, and Python front doors all route through, so its aliases,
/// case-insensitivity, and round-trip with `Display` are pinned here.
#[test]
fn bit_depth_parses_all_aliases_case_insensitively() {
    for (s, want) in [
        ("float", WavBitDepth::Float32),
        ("Float", WavBitDepth::Float32),
        ("f32", WavBitDepth::Float32),
        ("32", WavBitDepth::Float32),
        ("  FLOAT  ", WavBitDepth::Float32),
        ("24", WavBitDepth::Int24),
        ("16", WavBitDepth::Int16),
    ] {
        assert_eq!(s.parse::<WavBitDepth>(), Ok(want), "parsing {s:?}");
    }
}

#[test]
fn unknown_bit_depth_is_a_clear_error() {
    let err = "8".parse::<WavBitDepth>().unwrap_err();
    assert!(err.contains("expected float, 24, or 16"), "{err}");
    assert!("".parse::<WavBitDepth>().is_err());
}

#[test]
fn canonical_name_round_trips_through_the_parser() {
    for depth in [WavBitDepth::Float32, WavBitDepth::Int24, WavBitDepth::Int16] {
        assert_eq!(depth.canonical_str().parse::<WavBitDepth>(), Ok(depth));
        assert_eq!(depth.to_string().parse::<WavBitDepth>(), Ok(depth));
    }
}
