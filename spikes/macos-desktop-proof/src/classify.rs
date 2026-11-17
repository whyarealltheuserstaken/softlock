//! Draft classification and the liveness bound. No operating-system calls.
//!
//! State ids are the `CGEventSourceStateID` values in Apple's 10.15 SDK
//! `CGEventTypes.h`: private is -1, combined session is 0, HID system is 1.
//! The proof prints the integers the tap actually reads. This rule is the
//! draft in `docs/platforms/macos.md`. A measurement can replace it.

/// Heartbeat period from `HEARTBEAT_INTERVAL` in softlock-core.
pub const HEARTBEAT_MS: u64 = 250;

/// Three missed heartbeats, the existing fail-closed bound.
pub const HEARTBEAT_MISSES: u32 = 3;

/// A dead callback has to be reported inside this bound.
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

pub fn state_name(state_id: i64) -> &'static str {
    match state_id {
        STATE_PRIVATE => "kCGEventSourceStatePrivate",
        STATE_COMBINED_SESSION => "kCGEventSourceStateCombinedSessionState",
        STATE_HID_SYSTEM => "kCGEventSourceStateHIDSystemState",
        _ => "other",
    }
}

pub fn expected_state_id(source: &str) -> Option<i64> {
    match source {
        "kCGEventSourceStatePrivate" => Some(STATE_PRIVATE),
        "kCGEventSourceStateCombinedSessionState" => Some(STATE_COMBINED_SESSION),
        "kCGEventSourceStateHIDSystemState" => Some(STATE_HID_SYSTEM),
        _ => None,
    }
}

/// `0xFFFFFFFE` and `0xFFFFFFFF` from `CGEventType` in the same header.
pub fn disable_event_name(event_type: i64) -> &'static str {
    match event_type as u32 {
        0xFFFF_FFFE => "kCGEventTapDisabledByTimeout",
        0xFFFF_FFFF => "kCGEventTapDisabledByUserInput",
        _ => "none",
    }
}

pub fn dead_callback_reported_in_time(misses: u32, detect_ms: u64) -> bool {
    misses >= HEARTBEAT_MISSES && detect_ms <= DEAD_CALLBACK_LIMIT_MS
}

pub fn healthy_callback_within_one_heartbeat(detect_ms: u64) -> bool {
    detect_ms <= HEARTBEAT_MS
}

/// A guard failure ends Locked. There is no Unlocked result on this path.
pub fn fail_closed_end() -> &'static str {
    "Locked"
}

#[cfg(test)]
pub fn failure_ends_unlocked(end: &str) -> bool {
    end == "Unlocked"
}

/// Test-only seam. Release and the proof binary are built without this
/// feature, so a nonce cannot classify a posted event as physical there.
#[cfg(feature = "simulated-physical")]
pub mod simulated {
    use super::{DraftClass, classify};

    pub const MARKER: &str = "softlock-simulated-physical-marker";

    pub fn classify_with_nonce(
        state_id: i64,
        source_pid: i64,
        user_data: i64,
        nonce: i64,
    ) -> DraftClass {
        if user_data == nonce {
            DraftClass::Physical
        } else {
            classify(state_id, source_pid)
        }
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
        assert_eq!(classify(STATE_COMBINED_SESSION, 42), DraftClass::Software);
    }

    #[test]
    fn three_misses_of_250_ms_fit_inside_one_second() {
        let bound = HEARTBEAT_MS * u64::from(HEARTBEAT_MISSES);
        assert_eq!(bound, 750);
        assert!(dead_callback_reported_in_time(3, bound));
        assert!(dead_callback_reported_in_time(3, 999));
        assert!(!dead_callback_reported_in_time(3, 1_001));
        assert!(!dead_callback_reported_in_time(2, 500));
        assert!(healthy_callback_within_one_heartbeat(0));
        assert!(healthy_callback_within_one_heartbeat(250));
        assert!(!healthy_callback_within_one_heartbeat(251));
    }

    #[test]
    fn a_failure_ends_locked() {
        assert_eq!(fail_closed_end(), "Locked");
        assert!(!failure_ends_unlocked(fail_closed_end()));
        assert!(failure_ends_unlocked("Unlocked"));
    }

    #[test]
    fn disable_names_match_the_header() {
        assert_eq!(
            disable_event_name(0xFFFF_FFFE),
            "kCGEventTapDisabledByTimeout"
        );
        assert_eq!(disable_event_name(-2), "kCGEventTapDisabledByTimeout");
        assert_eq!(
            disable_event_name(0xFFFF_FFFF),
            "kCGEventTapDisabledByUserInput"
        );
        assert_eq!(disable_event_name(-1), "kCGEventTapDisabledByUserInput");
        assert_eq!(disable_event_name(0), "none");
    }

    #[cfg(feature = "simulated-physical")]
    #[test]
    fn the_nonce_seam_is_compiled_only_behind_the_feature() {
        let nonce = 0x1234_5678_9ABC_DEF0u64 as i64;
        assert_eq!(
            simulated::classify_with_nonce(STATE_COMBINED_SESSION, 99, nonce, nonce),
            DraftClass::Physical
        );
        assert_eq!(
            simulated::classify_with_nonce(STATE_COMBINED_SESSION, 99, nonce + 1, nonce),
            DraftClass::Software
        );
        assert!(simulated::MARKER.contains("simulated-physical"));
    }
}
