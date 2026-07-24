//! Degenerate audio shapes are refused at the decode boundary (adversarial
//! review, Finding 4). A malformed header declaring a channel/sample-count
//! shape that doesn't divide evenly (or zero channels / zero sample rate)
//! used to be able to flow through to the mel spectrogram and per-frame
//! analyzers, which assume a well-formed shape and panic (or divide by zero)
//! otherwise.
//!
//! The guarantee this test pins down is the one that matters: malformed shape
//! produces a *clean error, never a panic*. Each decoder's own validation is
//! the first line (hound rejects a `data` chunk that isn't a whole number of
//! frames); `load`'s `check_shape` and `Audio::from_wav`'s guards are the
//! defense-in-depth behind it, covering the FLAC/lossy paths whose shape comes
//! from an attacker-controlled stream header.

use std::io::Write;

/// Assemble a minimal PCM WAV with an explicit (possibly degenerate) shape.
fn wav_bytes(channels: u16, sample_rate: u32, data: &[i16]) -> Vec<u8> {
    let data_len = (data.len() * 2) as u32;
    let block_align = channels * 2;
    let byte_rate = sample_rate * u32::from(block_align);
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in data {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

fn write_temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cochlea-decode-shape-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::File::create(&path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
    path
}

#[test]
fn malformed_shapes_are_clean_errors_never_panics() {
    let cases = [
        // 3 samples on a 2-channel stride: one and a half frames.
        ("ragged.wav", wav_bytes(2, 8000, &[1000, -1000, 500])),
        // Zero channels: the interleave stride would be a divide-by-zero.
        ("zero_channels.wav", wav_bytes(0, 8000, &[0, 0, 0, 0])),
        // Zero sample rate: no time base for any per-second feature.
        ("zero_rate.wav", wav_bytes(1, 0, &[0, 0, 0, 0])),
    ];
    for (name, bytes) in cases {
        let path = write_temp(name, &bytes);
        // Must be Err, and reaching this line at all means it didn't panic.
        assert!(
            cochlea_decode::load(&path).is_err(),
            "{name}: a degenerate shape must be a clean error, not accepted"
        );
    }
}
