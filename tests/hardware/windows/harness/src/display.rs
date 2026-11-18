//! Whether a hotplug step was seen. A device or mode-change event is not a
//! change: the attached, enumerated, internal, or external count has to move.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayStep {
    Connect,
    Disconnect,
    /// The arrangement is unknown, so either direction counts.
    Either,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayWatch {
    /// Active paths from QueryDisplayConfig. `-1` means the query failed.
    pub active: i32,
    /// `EnumDisplayMonitors` count. `-1` means the query failed.
    pub monitors: i32,
    /// Active internal targets. `-1` means the query failed.
    pub internal: i32,
    /// Active external targets. `-1` means the query failed.
    pub external: i32,
    pub adds: u32,
    pub removes: u32,
    pub changes: u32,
}

pub fn display_step_seen(step: DisplayStep, before: DisplayWatch, after: DisplayWatch) -> bool {
    let connect = connect_seen(before, after);
    let disconnect = disconnect_seen(before, after);
    match step {
        DisplayStep::Connect => connect,
        DisplayStep::Disconnect => disconnect,
        DisplayStep::Either => connect || disconnect,
    }
}

fn connect_seen(before: DisplayWatch, after: DisplayWatch) -> bool {
    count_rose(before.active, after.active)
        || count_rose(before.monitors, after.monitors)
        || count_rose(before.internal, after.internal)
        || count_rose(before.external, after.external)
}

fn disconnect_seen(before: DisplayWatch, after: DisplayWatch) -> bool {
    count_fell(before.active, after.active)
        || count_fell(before.monitors, after.monitors)
        || count_fell(before.internal, after.internal)
        || count_fell(before.external, after.external)
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
            internal: -1,
            external: -1,
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
    fn a_device_event_without_a_count_change_is_not_a_hotplug() {
        let before = watch(1, 1, 0, 0, 0);
        let added = watch(1, 1, 1, 1, 2);
        assert!(!display_step_seen(DisplayStep::Connect, before, added));
        assert!(!display_step_seen(DisplayStep::Disconnect, before, added));
        assert!(!display_step_seen(DisplayStep::Either, before, added));
        let mut plugged = before;
        plugged.active = 2;
        plugged.external = 1;
        plugged.internal = 1;
        assert!(display_step_seen(DisplayStep::Connect, before, plugged));
    }

    #[test]
    fn an_unknown_arrangement_accepts_either_direction() {
        let attached = watch(1, 1, 0, 0, 0);
        let detached = watch(0, 1, 0, 0, 0);
        assert!(display_step_seen(DisplayStep::Either, attached, detached));
        assert!(display_step_seen(DisplayStep::Either, detached, attached));
        assert!(!display_step_seen(DisplayStep::Either, attached, attached));
    }

    #[test]
    fn an_unchanged_count_with_no_event_is_not_detected() {
        let sample = watch(1, 1, 0, 0, 0);
        assert!(!display_step_seen(DisplayStep::Connect, sample, sample));
        assert!(!display_step_seen(DisplayStep::Disconnect, sample, sample));
    }
}
