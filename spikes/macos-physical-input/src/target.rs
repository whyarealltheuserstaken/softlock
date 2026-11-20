//! Target-window state from `target-state.txt`. No operating-system calls.

pub const TARGET_CAP_SECS: u64 = 30 * 60;
pub const STALE_AFTER_MS: i64 = 5_000;

const _: () = assert!(TARGET_CAP_SECS >= 20 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetState {
    pub ready: bool,
    pub count_key: i32,
    pub count_click: i32,
    pub count_scroll: i32,
    pub count_probe: i32,
    pub written_ms: i64,
    pub key: bool,
}

pub fn parse_target_state(text: &str) -> TargetState {
    TargetState {
        ready: int_field(text, "ready=") == 1,
        count_key: int_field(text, "count-key="),
        count_click: int_field(text, "count-click="),
        count_scroll: int_field(text, "count-scroll="),
        count_probe: int_field(text, "count-probe="),
        written_ms: int_field(text, "written-ms=") as i64,
        key: int_field(text, "key=") == 1,
    }
}

pub fn count_for_kind(state: &TargetState, kind: &str) -> i32 {
    match kind {
        "key" | "usb" | "any-key" => state.count_key,
        "click" => state.count_click,
        "scroll" => state.count_scroll,
        _ => state.count_probe,
    }
}

pub fn window_received(before: i32, after: i32) -> bool {
    after > before && before >= 0
}

pub fn is_stale(written_ms: i64, now_ms: i64) -> bool {
    written_ms <= 0 || now_ms.saturating_sub(written_ms) > STALE_AFTER_MS
}

pub fn post_kind(source: &str) -> &'static str {
    match source {
        "kCGEventSourceStatePrivate" => "post-private",
        "kCGEventSourceStateCombinedSessionState" => "post-combined",
        _ => "post-hid",
    }
}

fn int_field(text: &str, key: &str) -> i32 {
    let Some(rest) = text.split(key).nth(1) else {
        return 0;
    };
    rest.split(['\n', '\r', ' '])
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_received_counts_the_matching_event_type_not_only_the_probe_key() {
        let before = parse_target_state(
            "ready=1\ncount-key=0\ncount-click=0\ncount-scroll=0\ncount-probe=3\nwritten-ms=1000\nkey=1\n",
        );
        let after_click = parse_target_state(
            "ready=1\ncount-key=0\ncount-click=1\ncount-scroll=0\ncount-probe=3\nwritten-ms=1100\nkey=1\n",
        );
        let after_scroll = parse_target_state(
            "ready=1\ncount-key=0\ncount-click=0\ncount-scroll=2\ncount-probe=3\nwritten-ms=1200\nkey=1\n",
        );
        let after_probe = parse_target_state(
            "ready=1\ncount-key=1\ncount-click=0\ncount-scroll=0\ncount-probe=4\nwritten-ms=1300\nkey=1\n",
        );
        assert!(window_received(
            count_for_kind(&before, "click"),
            count_for_kind(&after_click, "click")
        ));
        assert!(!window_received(
            count_for_kind(&before, "post-private"),
            count_for_kind(&after_click, "post-private")
        ));
        assert!(window_received(
            count_for_kind(&before, "scroll"),
            count_for_kind(&after_scroll, "scroll")
        ));
        assert!(window_received(
            count_for_kind(&before, "post-hid"),
            count_for_kind(&after_probe, "post-hid")
        ));
        assert!(window_received(
            count_for_kind(&before, "key"),
            count_for_kind(&after_probe, "key")
        ));
    }

    #[test]
    fn a_stale_heartbeat_fails_the_row_and_the_target_cap_is_tens_of_minutes() {
        assert!(is_stale(0, 10_000));
        assert!(!is_stale(9_000, 10_000));
        assert!(is_stale(1_000, 10_000));
        assert_eq!(TARGET_CAP_SECS / 60, 30);
        assert_eq!(post_kind("kCGEventSourceStatePrivate"), "post-private");
        assert_eq!(
            post_kind("kCGEventSourceStateCombinedSessionState"),
            "post-combined"
        );
        assert_eq!(post_kind("kCGEventSourceStateHIDSystemState"), "post-hid");
    }
}
