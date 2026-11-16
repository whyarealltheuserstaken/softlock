//! Which windows cover which monitors.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reconcile {
    pub create: Vec<MonitorRect>,
    pub keep: Vec<MonitorRect>,
    pub close: Vec<MonitorRect>,
}

/// One overlay window per monitor, using the monitor rectangle exactly.
pub fn windows_for(monitors: &[MonitorRect]) -> Result<Vec<MonitorRect>, String> {
    if monitors.is_empty() {
        return Err("no monitors to cover".into());
    }
    for (index, monitor) in monitors.iter().enumerate() {
        if monitor.width <= 0 || monitor.height <= 0 {
            return Err(format!("monitor {index} has no area"));
        }
    }
    let mut seen = monitors.to_vec();
    seen.sort_by_key(|rect| (rect.x, rect.y, rect.width, rect.height));
    for pair in seen.windows(2) {
        if pair[0] == pair[1] {
            return Err("two monitors have the same rectangle".into());
        }
    }
    Ok(monitors.to_vec())
}

/// Diff the windows we already have against the monitors we should cover.
pub fn reconcile(current: &[MonitorRect], next: &[MonitorRect]) -> Reconcile {
    let mut keep = Vec::new();
    let mut create = Vec::new();
    for rect in next {
        if current.contains(rect) {
            keep.push(*rect);
        } else {
            create.push(*rect);
        }
    }
    let close = current
        .iter()
        .copied()
        .filter(|rect| !next.contains(rect))
        .collect();
    Reconcile {
        create,
        keep,
        close,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> MonitorRect {
        MonitorRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn every_monitor_gets_its_own_exact_window() {
        let monitors = [
            rect(0, 0, 1920, 1080),
            rect(-1920, 0, 1920, 1080),
            rect(1920, -200, 2560, 1440),
        ];
        assert_eq!(windows_for(&monitors).unwrap(), monitors);
    }

    #[test]
    fn no_monitors_is_an_error() {
        assert!(windows_for(&[]).is_err());
    }

    #[test]
    fn a_new_monitor_is_created_and_a_missing_one_is_closed() {
        let primary = rect(0, 0, 1920, 1080);
        let extra = rect(1920, 0, 1920, 1080);
        let plan = reconcile(&[primary], &[primary, extra]);
        assert_eq!(plan.keep, vec![primary]);
        assert_eq!(plan.create, vec![extra]);
        assert!(plan.close.is_empty());

        let plan = reconcile(&[primary, extra], &[primary]);
        assert_eq!(plan.close, vec![extra]);
        assert!(plan.create.is_empty());
    }
}
