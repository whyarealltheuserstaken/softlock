//! Marker colour and pixel counts. No operating-system calls.

/// Colour the probe window paints. Captures look for this.
pub const MARKER: Rgb = Rgb {
    r: 220,
    g: 10,
    b: 160,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Counts BGRA pixels within `tolerance` of `color` on each channel.
pub fn count_near(pixels: &[u8], color: Rgb, tolerance: u8) -> u64 {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| {
            near(pixel[2], color.r, tolerance)
                && near(pixel[1], color.g, tolerance)
                && near(pixel[0], color.b, tolerance)
        })
        .count() as u64
}

fn near(value: u8, expected: u8, tolerance: u8) -> bool {
    value.abs_diff(expected) <= tolerance
}
