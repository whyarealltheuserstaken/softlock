//! Which physical rows this machine can run. A setup mismatch is a skip.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetupSkip {
    pub outcome: &'static str,
    pub note: &'static str,
}

pub fn physical_key(keyboard_count: i32) -> Option<SetupSkip> {
    if keyboard_count == 0 {
        Some(SetupSkip {
            outcome: "no-keyboard",
            note: "skipped before the prompt: no keyboard was attached",
        })
    } else {
        None
    }
}

pub fn physical_click(pointing_count: i32) -> Option<SetupSkip> {
    if pointing_count == 0 {
        Some(SetupSkip {
            outcome: "no-pointing-device",
            note: "skipped before the prompt: no trackpad or mouse was attached",
        })
    } else {
        None
    }
}

pub fn physical_scroll(trackpad: Option<bool>, pointing_count: i32) -> Option<SetupSkip> {
    if trackpad == Some(false) {
        return Some(SetupSkip {
            outcome: "no-trackpad",
            note: "skipped before the prompt: this Mac has no trackpad",
        });
    }
    if pointing_count == 0 {
        return Some(SetupSkip {
            outcome: "no-trackpad",
            note: "skipped before the prompt: no pointing device was attached",
        });
    }
    None
}

pub fn usb_keyboard(internal_keyboards: i32, total_keyboards: i32) -> Option<SetupSkip> {
    if total_keyboards <= internal_keyboards.max(1) && total_keyboards < 2 {
        Some(SetupSkip {
            outcome: "no-second-keyboard",
            note: "the machine had no second keyboard",
        })
    } else {
        None
    }
}

/// `Some(true)` when a built-in trackpad usage was seen.
pub fn trackpad_present(code: i32) -> Option<bool> {
    match code {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_setup_skips_with_a_precise_reason_or_runs() {
        assert_eq!(
            physical_key(0).map(|skip| skip.outcome),
            Some("no-keyboard")
        );
        assert!(physical_key(1).is_none());
        assert_eq!(
            physical_click(0).map(|skip| skip.outcome),
            Some("no-pointing-device")
        );
        assert!(physical_click(1).is_none());
        assert_eq!(
            physical_scroll(Some(false), 1).map(|skip| skip.outcome),
            Some("no-trackpad")
        );
        assert!(physical_scroll(Some(true), 1).is_none());
        assert!(physical_scroll(None, 1).is_none());
        assert_eq!(
            usb_keyboard(1, 1).map(|skip| skip.outcome),
            Some("no-second-keyboard")
        );
        assert!(usb_keyboard(1, 2).is_none());
        assert_eq!(trackpad_present(1), Some(true));
        assert_eq!(trackpad_present(0), Some(false));
        assert_eq!(trackpad_present(-1), None);
    }
}
