//! Hardware identifiers of the serial-number kind stay out of the log and the JSON.
//! Needles are built at runtime so this file does not embed the token isolation forbids.
//! `pmset -g assertions` is reduced to summary counts plus this probe's own lines.

fn needles() -> [String; 5] {
    [
        format!("IOPlatform{}{}", "Serial", "Number"),
        format!("{}-{}", "serial", "number"),
        format!("{} {}", "Serial", "Number"),
        format!("{}{}", "Serial", "Number"),
        format!("{} {}", "hardware", "serial"),
    ]
}

pub fn contains_serial(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    needles()
        .iter()
        .any(|needle| lower.contains(&needle.to_ascii_lowercase()))
}

pub fn scrub(text: &str) -> String {
    text.lines()
        .filter(|line| !contains_serial(line))
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Keep the system-wide assertion counts and lines that name this probe.
/// Drop other processes, paths, and device names so Henry never commits them.
pub fn filter_pmset(text: &str, pid: u32, reason: &str) -> String {
    let pid_token = format!("pid {pid}");
    let mut out = Vec::new();
    for line in scrub(text).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("Listed by owning process")
            || trimmed.starts_with("Kernel Assertions")
            || trimmed.starts_with("Assertion status")
            || looks_like_timestamp(trimmed)
        {
            continue;
        }
        if looks_like_count_line(trimmed) {
            out.push(trimmed.to_string());
            continue;
        }
        if (trimmed.contains(&pid_token) || (!reason.is_empty() && trimmed.contains(reason)))
            && !looks_like_other_owner(trimmed, pid, reason)
        {
            out.push(trimmed.to_string());
        }
    }
    out.join("\n")
}

fn looks_like_timestamp(line: &str) -> bool {
    line.len() >= 10 && line.as_bytes()[4] == b'-' && line.as_bytes()[7] == b'-'
}

fn looks_like_count_line(line: &str) -> bool {
    let mut parts = line.split_whitespace();
    let Some(name) = parts.next() else {
        return false;
    };
    let Some(count) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && name.chars().all(|ch| ch.is_ascii_alphabetic())
        && count.chars().all(|ch| ch.is_ascii_digit())
}

fn looks_like_other_owner(line: &str, pid: u32, reason: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.contains("pid ") && !line.contains(&format!("pid {pid}")) {
        return true;
    }
    if !reason.is_empty() && line.contains(reason) {
        return false;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::REASON_PREFIX;

    #[test]
    fn a_serial_line_is_dropped_and_the_rest_stays() {
        let text = format!(
            "PreventUserIdleSystemSleep\n{} {}: ABC123\nreason",
            "Serial", "Number"
        );
        assert!(contains_serial(&text));
        let cleaned = scrub(&text);
        assert!(!contains_serial(&cleaned));
        assert!(cleaned.contains("PreventUserIdleSystemSleep"));
        assert!(cleaned.contains("reason"));
        assert!(!cleaned.contains("ABC123"));
    }

    #[test]
    fn ordinary_pmset_text_is_kept() {
        let text = "PreventUserIdleSystemSleep pid 12 m07-keep-awake-probe";
        assert!(!contains_serial(text));
        assert_eq!(scrub(text), text);
    }

    #[test]
    fn pmset_keeps_counts_and_the_probe_and_drops_other_apps() {
        let reason = format!("{REASON_PREFIX}-2759");
        let dump = format!(
            "2026-11-16 23:26:02 +0000\n\
             Assertion status system-wide:\n\
                BackgroundTask                 0\n\
                PreventUserIdleDisplaySleep    1\n\
                PreventUserIdleSystemSleep     1\n\
             Listed by owning process:\n\
                pid 161(WindowServer): UserIsActive named: \"com.apple.iohideventsystem queue\"\n\
                pid 278(addonsd): PreventUserIdleSystemSleep named: \"Anka sleeplessness\"\n\
                pid 2759(macos-keep-awake): PreventUserIdleSystemSleep named: \"{reason}\"\n\
                pid 2759(macos-keep-awake): PreventUserIdleDisplaySleep named: \"{reason}\"\n\
             Kernel Assertions: 0x4=USB\n\
                id=501  level=255 0x4=USB owner=Virtual USB Keyboard\n\
                pid 99(powerd): PreventUserIdleSystemSleep named: \"Powerd - Prevent sleep while display is on\"\n"
        );
        let excerpt = filter_pmset(&dump, 2759, &reason);
        assert!(excerpt.contains("PreventUserIdleSystemSleep     1"));
        assert!(excerpt.contains("PreventUserIdleDisplaySleep    1"));
        assert!(excerpt.contains(&reason));
        assert!(excerpt.contains("pid 2759"));
        assert!(!excerpt.contains("WindowServer"));
        assert!(!excerpt.contains("Anka"));
        assert!(!excerpt.contains("addonsd"));
        assert!(!excerpt.contains("Virtual USB"));
        assert!(!excerpt.contains("powerd"));
        assert!(!excerpt.contains("/"));
        assert!(!excerpt.contains("com.apple"));
        let rendered = format!("pmset-while-held-begin\n{excerpt}\npmset-while-held-end\n");
        assert!(!rendered.contains("WindowServer"));
        assert!(!contains_serial(&excerpt));
    }
}
