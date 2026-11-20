//! Capture-path claim for m14. Decision D6: do not claim until Henry's Intel sheet agrees.

pub const SHARING_APIS: [&str; 5] = [
    "CGWindowListCreateImage",
    "CGDisplayCreateImage",
    "screencapture",
    "ScreenCaptureKit",
    "SCStream",
];

pub const SCREEN_CAPTURE_ENTRY: &str = "SCScreenshotManager excludingWindows empty";
pub const SCSTREAM_ENTRY: &str = "SCStream excludingWindows empty";

/// A live frame can disagree with the ordered-out frame by a few predicate pixels.
pub const BASELINE_NOISE: i32 = 16;

/// The marker left the image when the default capture holds the window area
/// above the ordered-out baseline, and `.none` stays within `BASELINE_NOISE` of that baseline.
pub fn window_removed(
    default_count: i32,
    none_count: i32,
    baseline_count: i32,
    window_area: i32,
) -> bool {
    window_area > 0
        && default_count >= baseline_count.saturating_add(window_area)
        && (none_count - baseline_count).abs() <= BASELINE_NOISE
}

/// m14 may not claim a path from this kit alone. The finding names the path after Henry's Intel run.
pub fn m14_may_claim(_removed: bool) -> bool {
    false
}

pub fn m14_token(removed: bool) -> &'static str {
    if m14_may_claim(removed) {
        "claim"
    } else {
        "do-not-claim"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_capture_path_records_marker_counts_for_default_and_none_sharing() {
        assert_eq!(SHARING_APIS.len(), 5);
        assert!(SHARING_APIS.contains(&"CGWindowListCreateImage"));
        assert!(SHARING_APIS.contains(&"CGDisplayCreateImage"));
        assert!(SHARING_APIS.contains(&"screencapture"));
        assert!(SHARING_APIS.contains(&"ScreenCaptureKit"));
        assert!(SHARING_APIS.contains(&"SCStream"));
        assert!(window_removed(76834, 34, 34, 76800));
        assert!(!window_removed(34, 34, 34, 76800));
        assert!(!window_removed(76834, 50000, 34, 76800));
    }

    #[test]
    fn the_finding_tells_m14_which_capture_paths_it_may_claim() {
        assert!(!m14_may_claim(true));
        assert_eq!(m14_token(true), "do-not-claim");
        assert_eq!(m14_token(false), "do-not-claim");
        assert_eq!(
            SCREEN_CAPTURE_ENTRY,
            "SCScreenshotManager excludingWindows empty"
        );
        assert_eq!(SCSTREAM_ENTRY, "SCStream excludingWindows empty");
    }
}
