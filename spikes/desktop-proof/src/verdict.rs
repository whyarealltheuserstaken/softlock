//! Pixel counts for the capture-exclusion check. No operating-system calls.

pub const MARKER: Rgb = Rgb {
    r: 220,
    g: 10,
    b: 160,
};
pub const CONTROL: Rgb = Rgb {
    r: 10,
    g: 180,
    b: 40,
};

pub const PRESENT_PIXELS: u32 = 1_000;
pub const ABSENT_PIXELS: u32 = 50;
pub const CONTROL_PIXELS: u32 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BgraImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl BgraImage {
    pub fn new(width: usize, height: usize, pixels: Vec<u8>) -> Option<Self> {
        if width == 0
            || height == 0
            || pixels.len() != width.saturating_mul(height).saturating_mul(4)
        {
            return None;
        }
        Some(Self {
            width,
            height,
            pixels,
        })
    }

    #[cfg(test)]
    pub fn solid(width: usize, height: usize, color: Rgb) -> Self {
        let pixel = [color.b, color.g, color.r, 255];
        let mut pixels = Vec::with_capacity(width * height * 4);
        for _ in 0..width * height {
            pixels.extend_from_slice(&pixel);
        }
        Self {
            width,
            height,
            pixels,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelCounts {
    pub marker: u32,
    pub control: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Present,
    Absent,
    Partial,
    Unusable,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Absent => "absent",
            Self::Partial => "partial",
            Self::Unusable => "unusable",
        }
    }
}

pub fn count_rgb(image: &BgraImage, color: Rgb, tolerance: u8) -> u32 {
    let mut count = 0u32;
    for pixel in image.pixels.as_chunks::<4>().0 {
        if channel_near(pixel[2], color.r, tolerance)
            && channel_near(pixel[1], color.g, tolerance)
            && channel_near(pixel[0], color.b, tolerance)
        {
            count = count.saturating_add(1);
        }
    }
    count
}

pub fn counts(image: &BgraImage) -> PixelCounts {
    PixelCounts {
        marker: count_rgb(image, MARKER, 2),
        control: count_rgb(image, CONTROL, 2),
    }
}

pub fn judge(sample: PixelCounts) -> Verdict {
    if sample.control < CONTROL_PIXELS {
        return Verdict::Unusable;
    }
    if sample.marker >= PRESENT_PIXELS {
        Verdict::Present
    } else if sample.marker < ABSENT_PIXELS {
        Verdict::Absent
    } else {
        Verdict::Partial
    }
}

fn channel_near(actual: u8, expected: u8, tolerance: u8) -> bool {
    actual.abs_diff(expected) <= tolerance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_marker_is_present_only_when_control_is_also_there() {
        let marker = BgraImage::solid(40, 40, MARKER);
        let marker_counts = counts(&marker);
        assert!(marker_counts.marker >= PRESENT_PIXELS);
        assert_eq!(judge(marker_counts), Verdict::Unusable);
        assert_eq!(judge(marker_counts).as_str(), "unusable");

        let mut mixed = BgraImage::solid(80, 40, CONTROL);
        mixed.pixels[..40 * 40 * 4].copy_from_slice(&BgraImage::solid(40, 40, MARKER).pixels);
        assert_eq!(judge(counts(&mixed)), Verdict::Present);
    }

    #[test]
    fn control_without_marker_is_absent() {
        let image = BgraImage::solid(80, 40, CONTROL);
        assert_eq!(judge(counts(&image)), Verdict::Absent);
    }

    #[test]
    fn near_colors_count_within_tolerance() {
        let mut image = BgraImage::solid(80, 40, CONTROL);
        let drifted = Rgb {
            r: MARKER.r + 2,
            g: MARKER.g,
            b: MARKER.b,
        };
        image.pixels[..40 * 40 * 4].copy_from_slice(&BgraImage::solid(40, 40, drifted).pixels);
        assert!(counts(&image).marker >= PRESENT_PIXELS);
    }

    #[test]
    fn empty_buffer_is_rejected() {
        assert!(BgraImage::new(0, 10, vec![]).is_none());
        assert!(BgraImage::new(2, 2, vec![0; 8]).is_none());
    }
}
