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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub fn rect_inside(image: &BgraImage, origin_x: i32, origin_y: i32, rect: ScreenRect) -> bool {
    if rect.width <= 0 || rect.height <= 0 {
        return false;
    }
    let x0 = rect.x.saturating_sub(origin_x);
    let y0 = rect.y.saturating_sub(origin_y);
    if rect.x < origin_x || rect.y < origin_y {
        return false;
    }
    let x1 = x0.saturating_add(rect.width);
    let y1 = y0.saturating_add(rect.height);
    x1 <= image.width as i32 && y1 <= image.height as i32
}

pub fn count_rgb_rect(
    image: &BgraImage,
    origin_x: i32,
    origin_y: i32,
    rect: ScreenRect,
    color: Rgb,
    tolerance: u8,
) -> Option<u32> {
    if !rect_inside(image, origin_x, origin_y, rect) {
        return None;
    }
    let x0 = (rect.x - origin_x) as usize;
    let y0 = (rect.y - origin_y) as usize;
    let width = rect.width as usize;
    let height = rect.height as usize;
    let mut count = 0u32;
    for y in y0..y0 + height {
        for x in x0..x0 + width {
            let index = (y * image.width + x) * 4;
            let pixel = &image.pixels[index..index + 4];
            if channel_near(pixel[2], color.r, tolerance)
                && channel_near(pixel[1], color.g, tolerance)
                && channel_near(pixel[0], color.b, tolerance)
            {
                count = count.saturating_add(1);
            }
        }
    }
    Some(count)
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
    fn adjacent_region_counts_only_inside_its_rectangle() {
        let mut image = BgraImage::solid(80, 40, CONTROL);
        let marker_pixels = BgraImage::solid(40, 40, MARKER);
        for y in 0..40 {
            let row = y * 80 * 4;
            let source = y * 40 * 4;
            image.pixels[row..row + 40 * 4]
                .copy_from_slice(&marker_pixels.pixels[source..source + 40 * 4]);
        }
        let marker = ScreenRect {
            x: 0,
            y: 0,
            width: 40,
            height: 40,
        };
        let adjacent = ScreenRect {
            x: 40,
            y: 0,
            width: 40,
            height: 40,
        };
        assert!(count_rgb_rect(&image, 0, 0, marker, MARKER, 2).unwrap() >= PRESENT_PIXELS);
        assert_eq!(
            count_rgb_rect(&image, 0, 0, adjacent, MARKER, 2).unwrap(),
            0
        );
        assert!(count_rgb_rect(&image, 0, 0, adjacent, CONTROL, 2).unwrap() >= CONTROL_PIXELS);
        let shifted = ScreenRect {
            x: 100,
            y: 20,
            width: 40,
            height: 40,
        };
        assert_eq!(
            count_rgb_rect(&image, 100, 20, shifted, MARKER, 2).unwrap(),
            count_rgb_rect(&image, 0, 0, marker, MARKER, 2).unwrap()
        );
        assert!(count_rgb_rect(&image, 0, 0, shifted, CONTROL, 2).is_none());
    }

    #[test]
    fn empty_buffer_is_rejected() {
        assert!(BgraImage::new(0, 10, vec![]).is_none());
        assert!(BgraImage::new(2, 2, vec![0; 8]).is_none());
    }
}
