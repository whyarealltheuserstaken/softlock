//! Which physical rows this machine can run. A setup mismatch is a skip.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetupSkip {
    pub outcome: &'static str,
    pub note: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LidPresence {
    Present,
    Absent,
    Unknown,
}

fn no_lid_skip() -> SetupSkip {
    SetupSkip {
        outcome: "no-lid",
        note: "this Mac has no lid",
    }
}

fn lid_unknown_skip() -> SetupSkip {
    SetupSkip {
        outcome: "lid-unknown",
        note: "lid unknown",
    }
}

pub fn lid_on_power(lid: LidPresence, on_ac: Option<bool>) -> Option<SetupSkip> {
    match lid {
        LidPresence::Absent => Some(no_lid_skip()),
        LidPresence::Unknown => Some(lid_unknown_skip()),
        LidPresence::Present if on_ac == Some(false) => Some(SetupSkip {
            outcome: "on-battery",
            note: "on battery; lid close on power needs AC",
        }),
        LidPresence::Present => None,
    }
}

pub fn lid_external(lid: LidPresence, displays: i32) -> Option<SetupSkip> {
    match lid {
        LidPresence::Absent => Some(no_lid_skip()),
        LidPresence::Unknown => Some(lid_unknown_skip()),
        LidPresence::Present if displays < 2 => Some(SetupSkip {
            outcome: "no-external-display",
            note: "skipped before the prompt: fewer than two displays",
        }),
        LidPresence::Present => None,
    }
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

pub fn desktop_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.contains("macmini")
        || lower.contains("imac")
        || lower.contains("macpro")
        || lower.contains("macstudio")
}

/// `registry` is `Some` when the clamshell property or an internal battery was checked.
/// Mac14 and Mac15 laptops do not contain "MacBook" in `hw.model`.
pub fn lid_presence(model: &str, registry: Option<bool>) -> LidPresence {
    match registry {
        Some(true) => LidPresence::Present,
        Some(false) => LidPresence::Absent,
        None if laptop_model(model) => LidPresence::Present,
        None if desktop_model(model) => LidPresence::Absent,
        None => LidPresence::Unknown,
    }
}

pub fn has_lid(model: &str, registry: Option<bool>) -> bool {
    lid_presence(model, registry) == LidPresence::Present
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
            lid_on_power(LidPresence::Absent, Some(true)).map(|skip| skip.outcome),
            Some("no-lid")
        );
        assert_eq!(
            lid_on_power(LidPresence::Present, Some(false)).map(|skip| skip.outcome),
            Some("on-battery")
        );
        assert!(lid_on_power(LidPresence::Present, Some(true)).is_none());
        assert!(lid_on_power(LidPresence::Present, None).is_none());
        assert_eq!(
            lid_external(LidPresence::Present, 1).map(|skip| skip.outcome),
            Some("no-external-display")
        );
        assert!(lid_external(LidPresence::Present, 2).is_none());
        assert_eq!(
            lid_external(LidPresence::Absent, 2).map(|skip| skip.outcome),
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

    #[test]
    fn unknown_lid_skips_as_lid_unknown_not_no_lid() {
        assert_eq!(lid_presence("Mac14,2", None), LidPresence::Unknown);
        assert_eq!(lid_presence("Mac15,12", None), LidPresence::Unknown);
        let skip = lid_on_power(LidPresence::Unknown, Some(true)).expect("skip");
        assert_eq!(skip.outcome, "lid-unknown");
        assert_eq!(skip.note, "lid unknown");
        assert_ne!(skip.outcome, "no-lid");
        let external = lid_external(LidPresence::Unknown, 2).expect("skip");
        assert_eq!(external.outcome, "lid-unknown");
        assert_eq!(external.note, "lid unknown");
    }

    #[test]
    fn known_laptop_without_registry_still_runs_or_skips_for_setup() {
        assert_eq!(lid_presence("MacBookPro16,2", None), LidPresence::Present);
        assert!(lid_on_power(LidPresence::Present, Some(true)).is_none());
        assert!(lid_on_power(LidPresence::Present, None).is_none());
        assert_eq!(
            lid_on_power(LidPresence::Present, Some(false)).map(|skip| skip.outcome),
            Some("on-battery")
        );
        assert_eq!(
            lid_external(LidPresence::Present, 1).map(|skip| skip.outcome),
            Some("no-external-display")
        );
        assert!(lid_external(LidPresence::Present, 2).is_none());
    }

    #[test]
    fn known_desktop_or_registry_no_lid_skips_as_no_lid() {
        assert_eq!(lid_presence("Macmini8,1", None), LidPresence::Absent);
        assert_eq!(lid_presence("iMac20,1", None), LidPresence::Absent);
        assert_eq!(lid_presence("Macmini8,1", Some(false)), LidPresence::Absent);
        assert_eq!(
            lid_on_power(LidPresence::Absent, Some(true)).map(|skip| skip.outcome),
            Some("no-lid")
        );
        assert_eq!(
            lid_external(LidPresence::Absent, 2).map(|skip| skip.outcome),
            Some("no-lid")
        );
    }

    #[test]
    fn registry_overrides_the_model_name() {
        assert_eq!(
            lid_presence("MacBookPro16,2", Some(false)),
            LidPresence::Absent
        );
        assert_eq!(
            lid_on_power(LidPresence::Absent, Some(true)).map(|skip| skip.outcome),
            Some("no-lid")
        );
        assert_eq!(lid_presence("Mac14,2", Some(true)), LidPresence::Present);
        assert_eq!(lid_presence("Mac15,12", Some(true)), LidPresence::Present);
        assert!(lid_on_power(LidPresence::Present, Some(true)).is_none());
        assert!(lid_external(LidPresence::Present, 2).is_none());
    }
}
