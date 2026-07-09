//! PNG rendering: spectrogram image, time ruler, bar markers, and tiled
//! contact sheets for whole-piece review in one vision call.

use crate::SpectroError;
use crate::font::{DIGIT_HEIGHT, DIGIT_WIDTH, digit_pixel};
use crate::marker::Marker;
use crate::mel::MelSpec;
use crate::viridis::viridis_color;
use image::{Rgb, RgbImage};
use std::path::Path;

/// Height in pixels of the time-ruler strip drawn beneath the spectrogram
/// in [`render_png`] and in every [`contact_sheet`] tile.
pub const RULER_HEIGHT_PX: u32 = 10;
/// Vertical gutter in pixels between stacked tiles in [`contact_sheet`].
pub const CONTACT_SHEET_GUTTER_PX: u32 = 4;

const TICK_HEIGHT_PX: u32 = 3;
const TICK_LABEL_GAP_PX: u32 = 1;
const TICK_COLOR: Rgb<u8> = Rgb([200, 200, 200]);
const MARKER_COLOR: Rgb<u8> = Rgb([255, 255, 255]);

/// Render a [`MelSpec`] to an RGB image: one column per frame, viridis LUT
/// for magnitude, low frequencies at the **bottom** of the spectrogram
/// region, a time ruler (tick marks + `0`-`9` digit labels, see
/// `crate::font`) along the bottom edge at sensible second intervals, and
/// 1px vertical marker lines (bar starts, supplied as plain sample offsets
/// — see `crate::Marker`) through the spectrogram region.
///
/// Image dimensions: `width = spec.frames.max(1)`, `height = spec.mels +
/// RULER_HEIGHT_PX`.
///
/// Pure function of `spec` and `markers` — deterministic and safe to call
/// repeatedly for byte-identical output (no filesystem, no clock, no RNG).
pub fn render_png(spec: &MelSpec, markers: &[Marker]) -> RgbImage {
    let width = spec.frames.max(1) as u32;
    let height = spec.mels as u32 + RULER_HEIGHT_PX;
    let mut img = RgbImage::new(width, height);

    for frame in 0..spec.frames {
        for mel in 0..spec.mels {
            let db = spec.get(mel, frame);
            let color = viridis_color(db, spec.floor_db);
            // Low frequencies (mel 0) at the bottom of the spectrogram
            // region, i.e. the row just above the ruler.
            let y = (spec.mels - 1 - mel) as u32;
            img.put_pixel(frame as u32, y, Rgb(color));
        }
    }

    for marker in markers {
        let frame = spec.sample_frame(marker.sample);
        if frame < spec.frames {
            for y in 0..spec.mels as u32 {
                img.put_pixel(frame as u32, y, MARKER_COLOR);
            }
        }
    }

    draw_ruler(&mut img, spec, spec.mels as u32);

    img
}

/// Split the time axis into tiles and stack them vertically (with a
/// [`CONTACT_SHEET_GUTTER_PX`]-pixel gutter) into one image, so a whole
/// piece reads in a single vision call.
///
/// `per_tile` is dual-purpose, matching the CLI's `--bars-per-tile`
/// (`docs/plan.md`):
/// - If `markers` is non-empty: tile boundaries fall at every `per_tile`-th
///   marker (sorted by sample), i.e. `per_tile` **markers per tile**. The
///   final tile runs to the end of the spectrogram.
/// - If `markers` is empty, there are no bar boundaries to tile at, so
///   `per_tile` is instead read as a **tile count**: the time axis is cut
///   into `per_tile` equal-length slices.
///
/// Each tile is rendered exactly like [`render_png`] (spectrogram + its own
/// ruler), with marker sample offsets rebased to the tile's local frame 0.
/// Tiles narrower than the widest tile are left-aligned on a black
/// background.
pub fn contact_sheet(spec: &MelSpec, markers: &[Marker], per_tile: usize) -> RgbImage {
    let ranges = tile_frame_ranges(spec, markers, per_tile);
    let tile_height = spec.mels as u32 + RULER_HEIGHT_PX;
    let width = ranges
        .iter()
        .map(|&(s, e)| (e - s) as u32)
        .max()
        .unwrap_or(1)
        .max(1);
    let n = ranges.len() as u32;
    let height = n * tile_height + n.saturating_sub(1) * CONTACT_SHEET_GUTTER_PX;

    let mut sheet = RgbImage::new(width, height);

    for (i, &(start, end)) in ranges.iter().enumerate() {
        let tile_spec = spec.sub(start, end);
        let tile_start_sample = spec.frame_sample(start);
        let tile_markers: Vec<Marker> = markers
            .iter()
            .filter_map(|m| {
                let frame = spec.sample_frame(m.sample);
                if frame >= start && frame < end {
                    Some(Marker::new(
                        m.sample.saturating_sub(tile_start_sample),
                        m.label.clone(),
                    ))
                } else {
                    None
                }
            })
            .collect();
        let tile_img = render_png(&tile_spec, &tile_markers);

        let y_off = i as u32 * (tile_height + CONTACT_SHEET_GUTTER_PX);
        for x in 0..tile_img.width() {
            for y in 0..tile_img.height() {
                sheet.put_pixel(x, y_off + y, *tile_img.get_pixel(x, y));
            }
        }
    }

    sheet
}

/// Save an image as a PNG at `path`. The only fallible, side-effecting
/// function in this crate.
pub fn write_png(img: &RgbImage, path: impl AsRef<Path>) -> Result<(), SpectroError> {
    let path = path.as_ref();
    img.save(path).map_err(|source| SpectroError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn tile_frame_ranges(spec: &MelSpec, markers: &[Marker], per_tile: usize) -> Vec<(usize, usize)> {
    let per_tile = per_tile.max(1);
    let total = spec.frames;

    if markers.is_empty() {
        // No bar boundaries: reinterpret per_tile as a tile count.
        let n_tiles = per_tile;
        let chunk = total.div_ceil(n_tiles).max(1);
        let mut ranges = Vec::new();
        let mut start = 0;
        while start < total {
            let end = (start + chunk).min(total);
            ranges.push((start, end));
            start = end;
        }
        if ranges.is_empty() {
            ranges.push((0, total));
        }
        return ranges;
    }

    let mut sorted: Vec<u64> = markers.iter().map(|m| m.sample).collect();
    sorted.sort_unstable();

    let mut starts: Vec<usize> = sorted
        .iter()
        .step_by(per_tile)
        .map(|&s| spec.sample_frame(s))
        .collect();
    if starts.first() != Some(&0) {
        starts.insert(0, 0);
    }
    starts.dedup();

    let mut ranges = Vec::new();
    for i in 0..starts.len() {
        let start = starts[i];
        let end = if i + 1 < starts.len() {
            starts[i + 1]
        } else {
            total
        };
        if end > start {
            ranges.push((start, end));
        }
    }
    if ranges.is_empty() {
        ranges.push((0, total));
    }
    ranges
}

/// Draw tick marks and `0`-`9` digit labels along `[0, top+RULER_HEIGHT_PX)`
/// at sensible second intervals (see `pick_tick_interval_secs`).
fn draw_ruler(img: &mut RgbImage, spec: &MelSpec, top: u32) {
    if spec.sample_rate == 0 || spec.hop == 0 {
        return;
    }
    let seconds_per_frame = spec.hop as f64 / spec.sample_rate as f64;
    let total_seconds = spec.frames as f64 * seconds_per_frame;
    let interval = pick_tick_interval_secs(total_seconds);

    let mut t = 0.0f64;
    // Guard against a pathological zero/negative interval looping forever.
    while t <= total_seconds && interval > 0.0 {
        let frame = (t / seconds_per_frame).round();
        if frame.is_finite() && frame >= 0.0 && (frame as usize) < spec.frames {
            let x = frame as u32;
            for dy in 0..TICK_HEIGHT_PX {
                if top + dy < img.height() {
                    img.put_pixel(x, top + dy, TICK_COLOR);
                }
            }
            let label = format!("{}", t.round() as i64);
            draw_digit_label(img, x, top + TICK_HEIGHT_PX + TICK_LABEL_GAP_PX, &label);
        }
        t += interval;
    }
}

/// Choose a "nice" tick interval (seconds) so a ruler shows roughly 8 ticks
/// across the full duration, regardless of clip length.
fn pick_tick_interval_secs(total_seconds: f64) -> f64 {
    const CANDIDATES: [f64; 13] = [
        0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0,
    ];
    const TARGET_TICKS: f64 = 8.0;
    if total_seconds <= 0.0 {
        return CANDIDATES[0];
    }
    for c in CANDIDATES {
        if total_seconds / c <= TARGET_TICKS {
            return c;
        }
    }
    *CANDIDATES.last().unwrap()
}

/// Draw a digit-only label centered horizontally on `tick_x`, top edge at
/// `y`. Pixels that would land outside the image are silently skipped.
fn draw_digit_label(img: &mut RgbImage, tick_x: u32, y: u32, label: &str) {
    let n = label.chars().filter(char::is_ascii_digit).count() as u32;
    if n == 0 {
        return;
    }
    let total_width = n * DIGIT_WIDTH + n.saturating_sub(1);
    let start_x = tick_x.saturating_sub(total_width / 2);

    let mut i = 0u32;
    for ch in label.chars() {
        let Some(d) = ch.to_digit(10) else { continue };
        let gx = start_x + i * (DIGIT_WIDTH + 1);
        for row in 0..DIGIT_HEIGHT {
            for col in 0..DIGIT_WIDTH {
                if digit_pixel(d as u8, row, col) {
                    let px = gx + col;
                    let py = y + row;
                    if px < img.width() && py < img.height() {
                        img.put_pixel(px, py, TICK_COLOR);
                    }
                }
            }
        }
        i += 1;
    }
}
