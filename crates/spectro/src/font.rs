//! A tiny hand-designed 3x5 pixel bitmap digit font (`0`-`9` only), used to
//! label the time ruler in [`crate::render_png`]/[`crate::contact_sheet`].
//!
//! Not derived from any standard typeface — this is a from-scratch pixel
//! design (unlike the viridis LUT, which is deliberately *not* invented and
//! comes verbatim from matplotlib's published anchor table; see
//! `crate::viridis`). Each glyph is 5 rows of 3 bits, MSB = leftmost pixel.

pub const DIGIT_WIDTH: u32 = 3;
pub const DIGIT_HEIGHT: u32 = 5;

pub const DIGIT_GLYPHS: [[u8; 5]; 10] = [
    // 0
    [0b111, 0b101, 0b101, 0b101, 0b111],
    // 1
    [0b010, 0b110, 0b010, 0b010, 0b111],
    // 2
    [0b111, 0b001, 0b111, 0b100, 0b111],
    // 3
    [0b111, 0b001, 0b111, 0b001, 0b111],
    // 4
    [0b101, 0b101, 0b111, 0b001, 0b001],
    // 5
    [0b111, 0b100, 0b111, 0b001, 0b111],
    // 6
    [0b111, 0b100, 0b111, 0b101, 0b111],
    // 7
    [0b111, 0b001, 0b010, 0b010, 0b010],
    // 8
    [0b111, 0b101, 0b111, 0b101, 0b111],
    // 9
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Bit at `(row, col)` of digit `d`'s glyph, `col` 0 = leftmost.
///
/// # Panics
/// Panics if `d > 9`, `row >= DIGIT_HEIGHT`, or `col >= DIGIT_WIDTH`.
pub fn digit_pixel(d: u8, row: u32, col: u32) -> bool {
    assert!(d <= 9, "digit_pixel expects 0..=9, got {d}");
    let bits = DIGIT_GLYPHS[d as usize][row as usize];
    (bits >> (DIGIT_WIDTH - 1 - col)) & 1 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_row_fits_in_three_bits() {
        for glyph in DIGIT_GLYPHS {
            for row in glyph {
                assert!(row <= 0b111, "row {row:#05b} has bits outside 3 columns");
            }
        }
    }
}
