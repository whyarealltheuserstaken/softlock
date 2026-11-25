//! Whether a set of overlay windows covers every monitor. Pure, so it runs on
//! every platform; the Windows side supplies the rectangles.

/// Screen rectangle in virtual-desktop coordinates, right and bottom exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl ScreenRect {
    pub fn contains(&self, other: &ScreenRect) -> bool {
        self.left <= other.left
            && self.top <= other.top
            && self.right >= other.right
            && self.bottom >= other.bottom
    }

    fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }
}

/// Monitors with no single overlay window containing them. An empty monitor
/// list is uncovered too: a lock with no display proves nothing.
pub fn uncovered(monitors: &[ScreenRect], windows: &[ScreenRect]) -> Vec<ScreenRect> {
    if monitors.is_empty() {
        return vec![ScreenRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        }];
    }
    monitors
        .iter()
        .filter(|monitor| {
            monitor.is_empty() || !windows.iter().any(|window| window.contains(monitor))
        })
        .copied()
        .collect()
}

/// Overlay windows that sit on no monitor. After a display is removed its
/// window must drop out of the covered set.
pub fn stray(monitors: &[ScreenRect], windows: &[ScreenRect]) -> Vec<ScreenRect> {
    windows
        .iter()
        .filter(|window| !monitors.iter().any(|monitor| window.contains(monitor)))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> ScreenRect {
        ScreenRect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn one_window_per_monitor_covers_both() {
        let monitors = [rect(0, 0, 1920, 1080), rect(1920, 0, 3200, 1024)];
        assert!(uncovered(&monitors, &monitors).is_empty());
        assert!(stray(&monitors, &monitors).is_empty());
    }

    #[test]
    fn a_window_left_at_the_old_resolution_does_not_cover_a_larger_monitor() {
        let monitors = [rect(0, 0, 1280, 1024)];
        let windows = [rect(0, 0, 1024, 768)];
        assert_eq!(uncovered(&monitors, &windows), monitors.to_vec());
    }

    #[test]
    fn an_added_monitor_without_a_window_is_uncovered() {
        let monitors = [rect(0, 0, 1024, 768), rect(1024, 0, 2048, 768)];
        let windows = [rect(0, 0, 1024, 768)];
        assert_eq!(uncovered(&monitors, &windows), vec![monitors[1]]);
    }

    #[test]
    fn a_window_for_a_removed_monitor_is_stray() {
        let monitors = [rect(0, 0, 1024, 768)];
        let windows = [rect(0, 0, 1024, 768), rect(1024, 0, 2048, 768)];
        assert!(uncovered(&monitors, &windows).is_empty());
        assert_eq!(stray(&monitors, &windows), vec![windows[1]]);
    }

    #[test]
    fn no_monitors_is_never_covered() {
        assert_eq!(uncovered(&[], &[]).len(), 1);
        assert_eq!(uncovered(&[rect(0, 0, 0, 0)], &[rect(0, 0, 0, 0)]).len(), 1);
    }
}
