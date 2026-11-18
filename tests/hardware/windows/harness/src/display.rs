//! Whether a hotplug step was seen. Enumerated monitors stay at 1 when the
//! only display is unplugged, because Windows keeps a placeholder. An active
//! path count of 0 is that moment, and an add/remove or mode-change event
//! covers a replug whose count ends where it started.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayStep {
    Connect,
    Disconnect,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayWatch {
    /// Active paths from QueryDisplayConfig. `-1` means the query failed.
    pub active: i32,
    /// `EnumDisplayMonitors` count. `-1` means the query failed.
    pub monitors: i32,
    pub adds: u32,
    pub removes: u32,
    pub changes: u32,
}

pub fn display_step_seen(step: DisplayStep, before: DisplayWatch, after: DisplayWatch) -> bool {
    let added = after.adds > before.adds;
    let removed = after.removes > before.removes;
    let changed = after.changes > before.changes;
    match step {
        DisplayStep::Disconnect => {
            removed
                || count_fell(before.active, after.active)
                || count_fell(before.monitors, after.monitors)
                || changed
        }
        DisplayStep::Connect => {
            added
                || count_rose(before.active, after.active)
                || count_rose(before.monitors, after.monitors)
                || changed
        }
    }
}

fn count_rose(before: i32, after: i32) -> bool {
    before >= 0 && after > before
}

fn count_fell(before: i32, after: i32) -> bool {
    before >= 0 && after >= 0 && after < before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch(active: i32, monitors: i32, adds: u32, removes: u32, changes: u32) -> DisplayWatch {
        DisplayWatch {
            active,
            monitors,
            adds,
            removes,
            changes,
        }
    }

    #[test]
    fn unplugging_the_only_external_monitor_is_seen_when_no_display_is_attached() {
        let before = watch(1, 1, 0, 0, 0);
        let after = watch(0, 1, 0, 0, 0);
        assert!(display_step_seen(DisplayStep::Disconnect, before, after));
        assert!(!display_step_seen(DisplayStep::Connect, before, after));
    }

    #[test]
    fn replugging_that_monitor_is_seen_when_the_enumerated_count_stays_at_one() {
        let before = watch(0, 1, 0, 1, 1);
        let after = watch(1, 1, 0, 1, 1);
        assert!(display_step_seen(DisplayStep::Connect, before, after));
    }

    #[test]
    fn a_same_count_replug_is_seen_from_the_device_event() {
        let before = watch(1, 1, 0, 0, 0);
        let added = watch(1, 1, 1, 1, 2);
        assert!(display_step_seen(DisplayStep::Connect, before, added));
        let removed = watch(1, 1, 0, 1, 1);
        assert!(display_step_seen(DisplayStep::Disconnect, before, removed));
    }

    #[test]
    fn an_unchanged_count_with_no_event_is_not_detected() {
        let sample = watch(1, 1, 0, 0, 0);
        assert!(!display_step_seen(DisplayStep::Connect, sample, sample));
        assert!(!display_step_seen(DisplayStep::Disconnect, sample, sample));
    }
}
