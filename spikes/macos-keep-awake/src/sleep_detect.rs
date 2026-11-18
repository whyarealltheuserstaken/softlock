//! Classify system and display sleep from public power logs and flags.
//!
//! IOKit notifications are wired on macOS; this module is the parser and
//! combiner so Linux unit tests can lock the strings.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SleepEvent {
    None,
    DisplaySleep,
    SystemSleep,
    Wake,
}

impl SleepEvent {
    pub fn slept(self) -> bool {
        matches!(self, Self::DisplaySleep | Self::SystemSleep)
    }
}

/// Keep `pmset -g log` lines whose local timestamp is at or after `since`
/// (`YYYY-MM-DD HH:MM:SS`). Older sleep entries must not count.
pub fn log_since(text: &str, since: &str) -> String {
    text.lines()
        .filter(|line| line_stamp(line).is_some_and(|stamp| stamp.as_str() >= since))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn line_stamp(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b' ' {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    Some(trimmed[..19].to_string())
}

pub fn classify_pmset_since(text: &str, since: &str) -> SleepEvent {
    classify_pmset_log(&log_since(text, since))
}

/// Scan `pmset -g log` text captured after a physical step started.
pub fn classify_pmset_log(text: &str) -> SleepEvent {
    let mut system = false;
    let mut display = false;
    let mut wake = false;
    for line in text.lines() {
        match classify_pmset_log_line(line) {
            SleepEvent::None => {}
            SleepEvent::Wake => wake = true,
            SleepEvent::DisplaySleep => display = true,
            SleepEvent::SystemSleep => system = true,
        }
    }
    // A later wake is the unlock after the lid closed. It does not erase the sleep.
    if system {
        SleepEvent::SystemSleep
    } else if display {
        SleepEvent::DisplaySleep
    } else if wake {
        SleepEvent::Wake
    } else {
        SleepEvent::None
    }
}

pub fn classify_pmset_log_line(line: &str) -> SleepEvent {
    let lower = line.to_ascii_lowercase();
    // Darkwake is a maintenance wake, not a lid close.
    if lower.contains("darkwake") {
        return SleepEvent::None;
    }
    if lower.contains("entering sleep")
        || lower.contains("clamshell sleep")
        || lower.contains("due to idle sleep")
        || column_is(&lower, "sleep")
    {
        return SleepEvent::SystemSleep;
    }
    if lower.contains("display is turned off")
        || lower.contains("display sleep")
        || column_is(&lower, "displaysleep")
        || lower.contains("notification displayisoff")
    {
        return SleepEvent::DisplaySleep;
    }
    if lower.contains("display is turned on")
        || column_is(&lower, "wake")
        || lower.contains("wake from")
        || lower.contains("darkwake using")
    {
        return SleepEvent::Wake;
    }
    SleepEvent::None
}

fn column_is(lower_line: &str, name: &str) -> bool {
    let trimmed = lower_line.trim();
    let Some(rest) = skip_timestamp(trimmed) else {
        return false;
    };
    let token = rest.split_whitespace().next().unwrap_or("");
    token == name
}

fn skip_timestamp(line: &str) -> Option<&str> {
    // "2026-11-17 12:00:01 -0800 Sleep ..."
    let bytes = line.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let mut parts = line.splitn(4, char::is_whitespace);
    parts.next()?;
    parts.next()?;
    parts.next()?;
    parts.next()
}

/// Combine IOKit flags with a parsed `pmset` window.
pub fn detected_sleep(
    iokit_system: bool,
    iokit_display: bool,
    pmset: SleepEvent,
) -> Option<SleepEvent> {
    if iokit_system || pmset == SleepEvent::SystemSleep {
        return Some(SleepEvent::SystemSleep);
    }
    if iokit_display || pmset == SleepEvent::DisplaySleep {
        return Some(SleepEvent::DisplaySleep);
    }
    None
}

pub fn detection_method(
    iokit_system: bool,
    iokit_display: bool,
    pmset: SleepEvent,
) -> &'static str {
    if iokit_system {
        "iokit-power-notification"
    } else if iokit_display {
        "nsworkspace-display"
    } else if pmset.slept() {
        "pmset-log"
    } else {
        "none"
    }
}

/// A lid close is a clamshell sleep in the log, not darkwake and not display sleep.
pub fn clamshell_sleep(text: &str) -> bool {
    text.to_ascii_lowercase().contains("clamshell sleep")
}

/// Positive lid signal: the system-sleep watcher, a closed clamshell, or a clamshell-sleep line.
pub fn lid_close_seen(iokit_system: bool, clamshell_closed: bool, pmset_window: &str) -> bool {
    iokit_system || clamshell_closed || clamshell_sleep(pmset_window)
}

pub fn outcome_token(event: SleepEvent) -> &'static str {
    match event {
        SleepEvent::SystemSleep | SleepEvent::DisplaySleep => "slept",
        SleepEvent::Wake | SleepEvent::None => "awake",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmset_log_lines_match_sleep_display_and_wake() {
        assert_eq!(
            classify_pmset_log_line(
                "2026-11-17 18:01:02 -0800 Entering Sleep state due to 'Clamshell Sleep'"
            ),
            SleepEvent::SystemSleep
        );
        assert_eq!(
            classify_pmset_log_line("2026-11-17 18:01:02 +0000 Sleep                 DarkWake"),
            SleepEvent::None
        );
        assert_eq!(
            classify_pmset_log_line("2026-11-17 18:01:03 +0000 Display is turned off"),
            SleepEvent::DisplaySleep
        );
        assert_eq!(
            classify_pmset_log_line("2026-11-17 18:01:04 +0000 Display is turned on"),
            SleepEvent::Wake
        );
        assert_eq!(
            classify_pmset_log_line("2026-11-17 18:01:04 +0000 Wake                  from Sleep"),
            SleepEvent::Wake
        );
        assert_eq!(
            classify_pmset_log_line("Assertion status system-wide:"),
            SleepEvent::None
        );
        assert_eq!(
            classify_pmset_log_line("PreventUserIdleSystemSleep pid 12"),
            SleepEvent::None
        );
        assert_eq!(
            classify_pmset_log_line("Sleep preventers: IODisplayWrangler"),
            SleepEvent::None
        );
    }

    #[test]
    fn a_window_prefers_system_sleep_over_display_sleep() {
        let text = "\
2026-11-17 18:00:00 +0000 Display is turned off\n\
2026-11-17 18:00:01 +0000 Entering Sleep state due to 'Clamshell Sleep'\n\
2026-11-17 18:00:20 +0000 Wake from Sleep\n";
        assert_eq!(classify_pmset_log(text), SleepEvent::SystemSleep);
        assert_eq!(
            classify_pmset_log("2026-11-17 18:00:20 +0000 Wake from Sleep\n"),
            SleepEvent::Wake
        );
        let only_display = "2026-11-17 18:00:00 +0000 Display is turned off\n";
        assert_eq!(classify_pmset_log(only_display), SleepEvent::DisplaySleep);
        assert_eq!(
            classify_pmset_log("2026-11-17 18:00:00 +0000 DarkWake from Sleep"),
            SleepEvent::None
        );
    }

    #[test]
    fn iokit_flags_win_then_pmset_then_nothing() {
        assert_eq!(
            detected_sleep(true, false, SleepEvent::DisplaySleep),
            Some(SleepEvent::SystemSleep)
        );
        assert_eq!(
            detected_sleep(false, true, SleepEvent::None),
            Some(SleepEvent::DisplaySleep)
        );
        assert_eq!(
            detected_sleep(false, false, SleepEvent::DisplaySleep),
            Some(SleepEvent::DisplaySleep)
        );
        assert_eq!(detected_sleep(false, false, SleepEvent::None), None);
        assert_eq!(
            detection_method(true, true, SleepEvent::SystemSleep),
            "iokit-power-notification"
        );
        assert_eq!(
            detection_method(false, true, SleepEvent::None),
            "nsworkspace-display"
        );
        assert_eq!(
            detection_method(false, false, SleepEvent::SystemSleep),
            "pmset-log"
        );
        assert_eq!(outcome_token(SleepEvent::SystemSleep), "slept");
        assert_eq!(outcome_token(SleepEvent::None), "awake");
    }

    #[test]
    fn darkwake_alone_is_not_a_lid_close() {
        let dark = "\
2026-11-17 18:00:00 +0000 DarkWake from Sleep\n\
2026-11-17 18:00:01 +0000 Display is turned off\n";
        assert!(!clamshell_sleep(dark));
        assert!(!lid_close_seen(false, false, dark));
        assert_eq!(classify_pmset_log(dark), SleepEvent::DisplaySleep);
        let clamshell = "2026-11-17 18:00:02 +0000 Entering Sleep state due to 'Clamshell Sleep'\n";
        assert!(clamshell_sleep(clamshell));
        assert!(lid_close_seen(false, false, clamshell));
        assert!(lid_close_seen(true, false, ""));
        assert!(lid_close_seen(false, true, dark));
    }

    #[test]
    fn pmset_lines_before_the_prompt_are_ignored() {
        let text = "\
2020-01-01 00:00:01 -0800 Entering Sleep state due to 'Clamshell Sleep'\n\
2026-11-17 18:00:00 -0800 Assertion status system-wide\n\
2026-11-17 18:05:01 -0800 Display is turned off\n";
        assert_eq!(
            classify_pmset_since(text, "2026-11-17 18:04:00"),
            SleepEvent::DisplaySleep
        );
        assert_eq!(
            classify_pmset_since(text, "2026-11-17 18:06:00"),
            SleepEvent::None
        );
        assert!(log_since(text, "2026-11-17 18:04:00").contains("Display is turned off"));
        assert!(!log_since(text, "2026-11-17 18:04:00").contains("2020-01-01"));
    }
}
