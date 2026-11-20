//! Draft classification from `docs/platforms/macos.md`. No operating-system calls.
//!
//! State ids are the `CGEventSourceStateID` values in Apple's 10.15 SDK
//! `CGEventTypes.h`: private is -1, combined session is 0, HID system is 1.

#[allow(dead_code)]
pub const HEARTBEAT_MS: u64 = 250;
#[allow(dead_code)]
pub const HEARTBEAT_MISSES: u32 = 3;
#[allow(dead_code)]
pub const DEAD_CALLBACK_LIMIT_MS: u64 = 1_000;

pub const STATE_PRIVATE: i64 = -1;
pub const STATE_COMBINED_SESSION: i64 = 0;
pub const STATE_HID_SYSTEM: i64 = 1;

pub const POST_SOURCES: [&str; 3] = [
    "kCGEventSourceStatePrivate",
    "kCGEventSourceStateCombinedSessionState",
    "kCGEventSourceStateHIDSystemState",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftClass {
    Physical,
    Software,
}

/// HID system state and a source pid of 0 is the draft physical row.
/// Every other pair is software. HID state alone is not physical.
pub fn classify(state_id: i64, source_pid: i64) -> DraftClass {
    if state_id == STATE_HID_SYSTEM && source_pid == 0 {
        DraftClass::Physical
    } else {
        DraftClass::Software
    }
}

pub fn draft_name(class: DraftClass) -> &'static str {
    match class {
        DraftClass::Physical => "physical",
        DraftClass::Software => "software",
    }
}

#[allow(dead_code)]
pub fn state_name(state_id: i64) -> &'static str {
    match state_id {
        STATE_PRIVATE => "kCGEventSourceStatePrivate",
        STATE_COMBINED_SESSION => "kCGEventSourceStateCombinedSessionState",
        STATE_HID_SYSTEM => "kCGEventSourceStateHIDSystemState",
        _ => "other",
    }
}

pub fn disable_event_name(event_type: i64) -> &'static str {
    match event_type as u32 {
        0xFFFF_FFFE => "kCGEventTapDisabledByTimeout",
        0xFFFF_FFFF => "kCGEventTapDisabledByUserInput",
        _ => "none",
    }
}

#[allow(dead_code)]
pub fn dead_callback_reported_in_time(misses: u32, detect_ms: u64) -> bool {
    misses >= HEARTBEAT_MISSES && detect_ms <= DEAD_CALLBACK_LIMIT_MS
}

#[allow(dead_code)]
pub fn healthy_callback_within_one_heartbeat(detect_ms: u64) -> bool {
    detect_ms <= HEARTBEAT_MS
}

/// Confirm the addendum rule, replace it, or leave it pending physical rows.
pub fn classification_verdict(
    physical_rows: &[(i64, i64)],
    posted_rows: &[(i64, i64)],
) -> (&'static str, &'static str) {
    if physical_rows.is_empty() {
        return (
            "pending-physical",
            "hid-and-pid-0-is-physical; posted events stay software until a finger is measured",
        );
    }
    let physical_match = physical_rows
        .iter()
        .all(|(state, pid)| classify(*state, *pid) == DraftClass::Physical);
    let posted_software = posted_rows
        .iter()
        .all(|(state, pid)| classify(*state, *pid) == DraftClass::Software);
    if physical_match && posted_software {
        (
            "confirmed",
            "hid-and-pid-0-is-physical; m14 swallows that row and passes the rest",
        )
    } else {
        (
            "replaced",
            "hid-and-pid-0-is-physical does not match the table; m14 must use the measured rows",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hid_and_pid_zero_is_the_only_draft_physical_row() {
        assert_eq!(classify(STATE_HID_SYSTEM, 0), DraftClass::Physical);
        assert_eq!(classify(STATE_HID_SYSTEM, 501), DraftClass::Software);
        assert_eq!(classify(STATE_PRIVATE, 0), DraftClass::Software);
        assert_eq!(classify(STATE_COMBINED_SESSION, 0), DraftClass::Software);
    }

    #[test]
    fn a_post_under_each_source_state_is_software() {
        for source in POST_SOURCES {
            let state = match source {
                "kCGEventSourceStatePrivate" => STATE_PRIVATE,
                "kCGEventSourceStateCombinedSessionState" => STATE_COMBINED_SESSION,
                "kCGEventSourceStateHIDSystemState" => STATE_HID_SYSTEM,
                _ => unreachable!(),
            };
            assert_eq!(classify(state, 4242), DraftClass::Software, "{source}");
        }
    }

    #[test]
    fn the_classification_rule_is_confirmed_or_replaced_and_m14_is_named() {
        let (pending, pending_rule) = classification_verdict(&[], &[(STATE_HID_SYSTEM, 9)]);
        assert_eq!(pending, "pending-physical");
        assert!(pending_rule.contains("software"));

        let (ok, ok_rule) = classification_verdict(
            &[(STATE_HID_SYSTEM, 0), (STATE_HID_SYSTEM, 0)],
            &[
                (STATE_PRIVATE, 11),
                (STATE_COMBINED_SESSION, 12),
                (STATE_HID_SYSTEM, 13),
            ],
        );
        assert_eq!(ok, "confirmed");
        assert!(ok_rule.contains("m14"));

        let (replaced, replaced_rule) =
            classification_verdict(&[(STATE_HID_SYSTEM, 77)], &[(STATE_PRIVATE, 11)]);
        assert_eq!(replaced, "replaced");
        assert!(replaced_rule.contains("m14"));
    }

    #[test]
    fn disable_names_match_the_header() {
        assert_eq!(
            disable_event_name(0xFFFF_FFFF),
            "kCGEventTapDisabledByUserInput"
        );
        assert_eq!(
            disable_event_name(0xFFFF_FFFE),
            "kCGEventTapDisabledByTimeout"
        );
        assert_eq!(disable_event_name(0), "none");
        assert!(dead_callback_reported_in_time(3, 812));
        assert!(healthy_callback_within_one_heartbeat(1));
        assert_eq!(draft_name(DraftClass::Physical), "physical");
        assert_eq!(
            state_name(STATE_HID_SYSTEM),
            "kCGEventSourceStateHIDSystemState"
        );
    }
}
