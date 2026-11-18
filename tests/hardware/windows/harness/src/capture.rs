//! Pixel verdict from the desktop-proof spike. No operating-system calls.

pub const PRESENT_PIXELS: u32 = 1_000;
pub const ABSENT_PIXELS: u32 = 50;
pub const CONTROL_PIXELS: u32 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelCounts {
    pub marker: u32,
    pub control: u32,
}

pub fn judge(sample: PixelCounts) -> &'static str {
    if sample.control < CONTROL_PIXELS {
        return "unusable";
    }
    if sample.marker >= PRESENT_PIXELS {
        "present"
    } else if sample.marker < ABSENT_PIXELS {
        "absent"
    } else {
        "partial"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_without_the_marker_is_absent() {
        assert_eq!(
            judge(PixelCounts {
                marker: 0,
                control: CONTROL_PIXELS,
            }),
            "absent"
        );
        assert_eq!(
            judge(PixelCounts {
                marker: PRESENT_PIXELS,
                control: CONTROL_PIXELS,
            }),
            "present"
        );
        assert_eq!(
            judge(PixelCounts {
                marker: PRESENT_PIXELS,
                control: 0,
            }),
            "unusable"
        );
        assert_eq!(
            judge(PixelCounts {
                marker: ABSENT_PIXELS,
                control: CONTROL_PIXELS,
            }),
            "partial"
        );
    }
}
