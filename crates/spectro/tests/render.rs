//! Integration tests over the public API only: render_png dimensions and
//! byte-determinism, and contact_sheet tiling geometry.

use cochlea_spectro::{
    CONTACT_SHEET_GUTTER_PX, Marker, RULER_HEIGHT_PX, SpectroOpts, contact_sheet, mel_spectrogram,
    render_png,
};
use std::f32::consts::PI;

fn sine(freq: f32, sample_rate: u32, num_samples: usize, amp: f32) -> Vec<f32> {
    (0..num_samples)
        .map(|n| amp * libm::sinf(2.0 * PI * freq * n as f32 / sample_rate as f32))
        .collect()
}

#[test]
fn render_png_has_expected_dimensions() {
    let sr = 8_000u32;
    let samples = sine(440.0, sr, 4_000, 0.6);
    let opts = SpectroOpts::new().fft(256).hop(64).mels(32);
    let spec = mel_spectrogram(&samples, 1, sr, &opts);

    let img = render_png(&spec, &[]);
    assert_eq!(img.width(), spec.frames as u32);
    assert_eq!(img.height(), spec.mels as u32 + RULER_HEIGHT_PX);
}

#[test]
fn render_png_is_byte_deterministic() {
    let sr = 8_000u32;
    let samples = sine(440.0, sr, 4_000, 0.6);
    let opts = SpectroOpts::new().fft(256).hop(64).mels(32);
    let spec = mel_spectrogram(&samples, 1, sr, &opts);
    let markers = vec![Marker::new(500, "bar1"), Marker::new(2000, "bar2")];

    let a = render_png(&spec, &markers);
    let b = render_png(&spec, &markers);

    assert_eq!(a.dimensions(), b.dimensions());
    assert_eq!(a.into_raw(), b.into_raw());
}

#[test]
fn contact_sheet_with_four_markers_stacks_four_tiles() {
    let sr = 8_000u32;
    // 4 seconds so 4 equally spaced markers land at whole-second boundaries.
    let samples = sine(220.0, sr, sr as usize * 4, 0.5);
    let opts = SpectroOpts::new().fft(256).hop(64).mels(24);
    let spec = mel_spectrogram(&samples, 1, sr, &opts);

    let markers = vec![
        Marker::new(0, "bar1"),
        Marker::new(sr as u64, "bar2"),
        Marker::new(sr as u64 * 2, "bar3"),
        Marker::new(sr as u64 * 3, "bar4"),
    ];

    // One marker per tile -> exactly 4 tiles.
    let sheet = contact_sheet(&spec, &markers, 1);

    let tile_height = spec.mels as u32 + RULER_HEIGHT_PX;
    let expected_height = 4 * tile_height + 3 * CONTACT_SHEET_GUTTER_PX;
    assert_eq!(sheet.height(), expected_height);
    assert!(sheet.width() > 0);
}

#[test]
fn contact_sheet_without_markers_splits_into_equal_tiles() {
    let sr = 8_000u32;
    let samples = sine(220.0, sr, sr as usize * 2, 0.5);
    let opts = SpectroOpts::new().fft(256).hop(64).mels(16);
    let spec = mel_spectrogram(&samples, 1, sr, &opts);

    let sheet = contact_sheet(&spec, &[], 5);

    let tile_height = spec.mels as u32 + RULER_HEIGHT_PX;
    let expected_height = 5 * tile_height + 4 * CONTACT_SHEET_GUTTER_PX;
    assert_eq!(sheet.height(), expected_height);
}
