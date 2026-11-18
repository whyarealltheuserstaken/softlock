//! Which physical rows this machine can run. A setup mismatch is a skip.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetupSkip {
    pub outcome: &'static str,
    pub note: &'static str,
}

pub fn lid_on_power(laptop: bool, on_ac: Option<bool>) -> Option<SetupSkip> {
    if !laptop {
        return Some(SetupSkip {
            outcome: "no-lid",
            note: "this Mac has no lid",
        });
    }
    if on_ac == Some(false) {
        return Some(SetupSkip {
            outcome: "on-battery",
            note: "on battery; lid close on power needs AC",
        });
    }
    None
}

pub fn lid_external(laptop: bool, displays: i32) -> Option<SetupSkip> {
    if !laptop {
        return Some(SetupSkip {
            outcome: "no-lid",
            note: "this Mac has no lid",
        });
    }
    if displays < 2 {
        return Some(SetupSkip {
            outcome: "no-external-display",
            note: "skipped before the prompt: fewer than two displays",
        });
    }
    None
}

pub fn power_key(touch_id: Option<bool>) -> Option<SetupSkip> {
    if touch_id == Some(false) {
        return Some(SetupSkip {
            outcome: "no-touch-id",
            note: "this Mac has no Touch ID",
        });
    }
    None
}

pub fn laptop_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.contains("macbook")
}

/// `registry` is `Some` when the clamshell property or an internal battery was checked.
/// Mac14 and Mac15 laptops do not contain "MacBook" in `hw.model`.
pub fn has_lid(model: &str, registry: Option<bool>) -> bool {
    match registry {
        Some(present) => present,
        None => laptop_model(model),
    }
}

/// `Some(true)` is AC, `Some(false)` is battery, `None` is unread.
pub fn ac_power(pmset_batt: &str) -> Option<bool> {
    let lower = pmset_batt.to_ascii_lowercase();
    let ac = lower.contains("ac power");
    let battery = lower.contains("battery power");
    match (ac, battery) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// `1` is a sensor, `0` is a registry answer with no sensor, anything else is unread.
pub fn touch_id_sensor(code: i32) -> Option<bool> {
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
            lid_on_power(false, Some(true)).map(|skip| skip.outcome),
            Some("no-lid")
        );
        assert_eq!(
            lid_on_power(true, Some(false)).map(|skip| skip.outcome),
            Some("on-battery")
        );
        assert!(lid_on_power(true, Some(true)).is_none());
        assert!(lid_on_power(true, None).is_none());
        assert_eq!(
            lid_external(true, 1).map(|skip| skip.outcome),
            Some("no-external-display")
        );
        assert!(lid_external(true, 2).is_none());
        assert_eq!(
            lid_external(false, 2).map(|skip| skip.outcome),
            Some("no-lid")
        );
        assert_eq!(
            power_key(Some(false)).map(|skip| skip.outcome),
            Some("no-touch-id")
        );
        assert!(power_key(Some(true)).is_none());
        assert!(power_key(None).is_none());
        assert!(laptop_model("MacBookPro16,2"));
        assert!(!laptop_model("Macmini6,2"));
        assert!(!laptop_model("Mac14,2"));
        assert!(has_lid("Mac14,2", Some(true)));
        assert!(has_lid("Mac15,12", Some(true)));
        assert!(!has_lid("Macmini8,1", Some(false)));
        assert!(!has_lid("Mac14,2", None));
        assert!(has_lid("MacBookPro16,2", None));
        assert_eq!(ac_power("Now drawing from 'AC Power'"), Some(true));
        assert_eq!(ac_power("Now drawing from 'Battery Power'"), Some(false));
        assert_eq!(ac_power(""), None);
        assert_eq!(touch_id_sensor(1), Some(true));
        assert_eq!(touch_id_sensor(0), Some(false));
        assert_eq!(touch_id_sensor(-1), None);
    }
}
