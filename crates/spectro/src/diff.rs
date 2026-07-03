//! Tier 3 sentinel comparator (`docs/determinism.md`): image diff with a
//! per-channel tolerance and a max-fraction-differing threshold, used to
//! compare renders against committed golden PNGs across platforms where
//! byte-identity isn't promised.

use image::RgbImage;

/// Result of comparing two images with [`diff_images`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffResult {
    /// `true` iff dimensions match and `fraction_differing <=
    /// max_frac_differing`.
    pub passed: bool,
    pub differing_pixels: u64,
    pub total_pixels: u64,
    pub fraction_differing: f64,
    pub dimensions_match: bool,
}

/// Compare two images. A pixel counts as *differing* if any channel (R, G,
/// or B) differs by more than `per_channel_tol`. Passes iff dimensions
/// match and the fraction of differing pixels is `<= max_frac_differing`.
/// A dimension mismatch always fails, reported with `fraction_differing =
/// 1.0` and `total_pixels = 0` (no addressable common canvas to count over).
pub fn diff_images(
    a: &RgbImage,
    b: &RgbImage,
    per_channel_tol: u8,
    max_frac_differing: f64,
) -> DiffResult {
    if a.dimensions() != b.dimensions() {
        return DiffResult {
            passed: false,
            differing_pixels: 0,
            total_pixels: 0,
            fraction_differing: 1.0,
            dimensions_match: false,
        };
    }

    let total_pixels = a.width() as u64 * a.height() as u64;
    let mut differing_pixels = 0u64;
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        let differs =
            pa.0.iter()
                .zip(pb.0.iter())
                .any(|(&ca, &cb)| ca.abs_diff(cb) > per_channel_tol);
        if differs {
            differing_pixels += 1;
        }
    }

    let fraction_differing = if total_pixels == 0 {
        0.0
    } else {
        differing_pixels as f64 / total_pixels as f64
    };

    DiffResult {
        passed: fraction_differing <= max_frac_differing,
        differing_pixels,
        total_pixels,
        fraction_differing,
        dimensions_match: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> RgbImage {
        RgbImage::from_fn(w, h, |_, _| image::Rgb(rgb))
    }

    #[test]
    fn identical_images_pass_at_zero_tolerance() {
        let a = solid(16, 16, [50, 60, 70]);
        let b = a.clone();
        let result = diff_images(&a, &b, 0, 0.0);
        assert!(result.passed);
        assert_eq!(result.differing_pixels, 0);
        assert_eq!(result.fraction_differing, 0.0);
        assert!(result.dimensions_match);
    }

    #[test]
    fn dimension_mismatch_always_fails() {
        let a = solid(16, 16, [0, 0, 0]);
        let b = solid(8, 8, [0, 0, 0]);
        let result = diff_images(&a, &b, 255, 1.0);
        assert!(!result.passed);
        assert!(!result.dimensions_match);
    }

    #[test]
    fn a_stamped_rectangle_fails_at_zero_tolerance() {
        let a = solid(16, 16, [10, 10, 10]);
        let mut b = a.clone();
        for y in 4..8 {
            for x in 4..8 {
                b.put_pixel(x, y, image::Rgb([250, 250, 250]));
            }
        }
        let result = diff_images(&a, &b, 0, 0.0);
        assert!(!result.passed);
        assert_eq!(result.differing_pixels, 16);
        assert!((result.fraction_differing - 16.0 / 256.0).abs() < 1e-9);
    }

    #[test]
    fn small_noise_below_tolerance_passes() {
        let a = solid(16, 16, [128, 128, 128]);
        let mut b = a.clone();
        // Deterministic +/-2 checkerboard perturbation, well within tol=5.
        for y in 0..16 {
            for x in 0..16 {
                let delta: i16 = if (x + y) % 2 == 0 { 2 } else { -2 };
                let base = a.get_pixel(x, y).0;
                let noisy = base.map(|c| (c as i16 + delta).clamp(0, 255) as u8);
                b.put_pixel(x, y, image::Rgb(noisy));
            }
        }
        let result = diff_images(&a, &b, 5, 0.0);
        assert!(result.passed);
        assert_eq!(result.differing_pixels, 0);
    }
}
