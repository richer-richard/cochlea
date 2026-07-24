//! `Audio::from_wav_reader` — the reader-based decode entry point added so
//! `cochlea_decode` can peek a WAV's declared length for its read-path cap and
//! then decode the *same* open reader, parsing the header once instead of
//! reopening the file. Two guarantees are pinned here: it decodes an in-memory
//! reader correctly, and it produces byte-for-byte the same `Audio` as
//! `Audio::from_wav` on identical bytes (the path wrapper is just
//! `from_wav_reader(open(path))`).

use std::io::Cursor;

use cochlea_features::Audio;

/// A deterministic float-WAV byte blob (no transcendentals — a plain ramp, so
/// the test never trips the DSP-path libm ban).
fn float_wav(channels: u16, sample_rate: u32, samples: &[f32]) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut buf = Vec::new();
    {
        let mut writer = hound::WavWriter::new(Cursor::new(&mut buf), spec).unwrap();
        for &s in samples {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
    }
    buf
}

#[test]
fn from_wav_reader_decodes_an_in_memory_reader() {
    // 480 stereo frames of a ramp in [-0.4, 0.4).
    let interleaved: Vec<f32> = (0..960).map(|i| (i as f32 / 960.0) * 0.8 - 0.4).collect();
    let bytes = float_wav(2, 48_000, &interleaved);

    let reader = hound::WavReader::new(Cursor::new(&bytes)).unwrap();
    let audio = Audio::from_wav_reader(reader).unwrap();

    assert_eq!(audio.channels, 2);
    assert_eq!(audio.sample_rate, 48_000);
    assert_eq!(audio.samples.len(), interleaved.len());
    assert_eq!(audio.samples, interleaved, "float PCM decodes verbatim");
}

#[test]
fn from_wav_reader_matches_from_wav_on_identical_bytes() {
    let interleaved: Vec<f32> = (0..600).map(|i| (i as f32 / 600.0) - 0.5).collect();
    let bytes = float_wav(1, 44_100, &interleaved);

    let via_reader = Audio::from_wav_reader(hound::WavReader::new(Cursor::new(&bytes)).unwrap())
        .expect("reader path decodes");

    let dir = std::env::temp_dir().join(format!("cochlea-wav-reader-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("same.wav");
    std::fs::write(&path, &bytes).unwrap();
    let via_path = Audio::from_wav(&path).expect("path path decodes");

    assert_eq!(via_reader.channels, via_path.channels);
    assert_eq!(via_reader.sample_rate, via_path.sample_rate);
    assert_eq!(
        via_reader.samples, via_path.samples,
        "the reader and path decode must be identical"
    );
}
