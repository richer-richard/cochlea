//! Integration tests over the public API only: render_png dimensions and
//! byte-determinism, contact_sheet tiling geometry, analysis overlays, and
//! the signed diff spectrogram.

use cochlea_spectro::{
    CONTACT_SHEET_GUTTER_PX, Marker, Overlay, RULER_HEIGHT_PX, SpectroError, SpectroOpts,
    contact_sheet, mel_spectrogram, render_annotated, render_diff_png, render_png,
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

#[test]
fn hz_band_maps_the_frequency_axis() {
    let sr = 8_000u32;
    let samples = sine(440.0, sr, sr as usize, 0.5);
    let opts = SpectroOpts::new().fft(256).hop(64).mels(32);
    let spec = mel_spectrogram(&samples, 1, sr, &opts);

    // In range: monotone along the axis.
    let low = spec.hz_band(100.0).expect("100 Hz is in range");
    let high = spec.hz_band(3_000.0).expect("3 kHz is under Nyquist");
    assert!(low < high, "mel axis must be monotone: {low} vs {high}");
    // Out of range on both sides, and non-finite: None, never a panic.
    assert_eq!(spec.hz_band(1.0), None);
    assert_eq!(spec.hz_band(10_000.0), None);
    assert_eq!(spec.hz_band(f64::NAN), None);

    // The band holding a pure tone's energy is the band hz_band names
    // (±1 for triangle overlap) — the overlay lands on the energy.
    let named = spec.hz_band(440.0).expect("440 Hz is in range");
    let frame = spec.frames / 4;
    let loudest = (0..spec.mels)
        .max_by(|&a, &b| spec.get(a, frame).total_cmp(&spec.get(b, frame)))
        .unwrap();
    assert!(
        named.abs_diff(loudest) <= 1,
        "hz_band said {named}, energy sits in {loudest}"
    );
}

#[test]
fn render_annotated_draws_every_overlay_layer() {
    let sr = 8_000u32;
    let samples = sine(440.0, sr, sr as usize, 0.5);
    let opts = SpectroOpts::new().fft(256).hop(64).mels(32);
    let spec = mel_spectrogram(&samples, 1, sr, &opts);

    let overlay = Overlay {
        beats: vec![2_000],
        onsets: vec![4_000],
        pitch: vec![(1_000, 6_000, 440.0)],
    };
    let plain = render_annotated(&spec, &[], &Overlay::new());
    let annotated = render_annotated(&spec, &[], &overlay);

    assert_eq!(plain, render_png(&spec, &[]), "empty overlay == render_png");
    assert_eq!(plain.dimensions(), annotated.dimensions());

    let beat_x = spec.sample_frame(2_000) as u32;
    assert_eq!(annotated.get_pixel(beat_x, 0), &image::Rgb([255, 96, 64]));
    let onset_x = spec.sample_frame(4_000) as u32;
    let bottom = spec.mels as u32 - 1;
    assert_eq!(
        annotated.get_pixel(onset_x, bottom),
        &image::Rgb([64, 224, 255])
    );
    let pitch_y = (spec.mels - 1 - spec.hz_band(440.0).unwrap()) as u32;
    let mid_x = spec.sample_frame(3_500) as u32;
    assert_eq!(
        annotated.get_pixel(mid_x, pitch_y),
        &image::Rgb([255, 64, 255])
    );
}

#[test]
fn diff_spectrogram_shows_signed_change_and_rejects_axis_mismatch() {
    let sr = 8_000u32;
    let opts = SpectroOpts::new().fft(256).hop(64).mels(32);
    let quiet = mel_spectrogram(&sine(440.0, sr, sr as usize, 0.2), 1, sr, &opts);
    let loud = mel_spectrogram(&sine(440.0, sr, sr as usize, 0.8), 1, sr, &opts);

    // b louder than a -> red channel dominates somewhere; the reverse ->
    // blue dominates; identical -> all black in the spectrogram region.
    let hotter = render_diff_png(&quiet, &loud).unwrap();
    let cooler = render_diff_png(&loud, &quiet).unwrap();
    let same = render_diff_png(&quiet, &quiet).unwrap();

    let count = |img: &image::RgbImage, pick: fn(&image::Rgb<u8>) -> bool| {
        img.pixels().filter(|p| pick(p)).count()
    };
    assert!(count(&hotter, |p| p[0] > 32 && p[2] == 0) > 100, "no red?");
    assert!(count(&cooler, |p| p[2] > 32 && p[0] == 0) > 100, "no blue?");
    let spectro_region_nonblack = same
        .enumerate_pixels()
        .filter(|&(_, y, p)| y < quiet.mels as u32 && (p[0] > 0 || p[2] > 0))
        .count();
    assert_eq!(
        spectro_region_nonblack, 0,
        "identical inputs must diff black"
    );

    let other_axis = mel_spectrogram(
        &sine(440.0, sr, sr as usize, 0.5),
        1,
        sr,
        &SpectroOpts::new().fft(256).hop(64).mels(16),
    );
    assert!(matches!(
        render_diff_png(&quiet, &other_axis),
        Err(SpectroError::AxisMismatch)
    ));
}
