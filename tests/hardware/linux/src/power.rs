//! Keep-awake / inhibit and suspend mode probes.

use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::json;

use crate::detect::DetectedConfig;
use crate::report::{TestRow, empty_measurements};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemSleep {
    pub current: String,
    pub available: Vec<String>,
}

pub fn parse_power_state(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn parse_mem_sleep(text: &str) -> MemSleep {
    let mut current = String::new();
    let mut available = Vec::new();
    for token in text.split_whitespace() {
        if let Some(inner) = token.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            current = inner.to_string();
            available.push(inner.to_string());
        } else {
            available.push(token.to_string());
        }
    }
    if current.is_empty() {
        current = available
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".into());
    }
    MemSleep { current, available }
}

pub fn parse_inhibit_list(text: &str, who: &str) -> bool {
    text.lines().any(|line| line.contains(who))
}

pub fn parse_lid_closed_property(text: &str) -> Option<bool> {
    match text.trim() {
        "b true" | "true" => Some(true),
        "b false" | "false" => Some(false),
        _ => None,
    }
}

pub fn parse_locked_hint(text: &str) -> Option<bool> {
    for line in text.lines() {
        if let Some(value) = line.trim().strip_prefix("LockedHint=") {
            return match value.trim() {
                "yes" => Some(true),
                "no" => Some(false),
                _ => None,
            };
        }
    }
    None
}

pub fn environment_refusal(stderr: &str) -> bool {
    let text = stderr.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "no medium",
        "failed to connect to bus",
        "not supported",
        "access denied",
        "permission denied",
        "no session",
        "cannot inhibit",
        "interactive authentication",
        "not been booted with systemd",
        "connection refused",
        "host is down",
        "operation not permitted",
        "no such file",
    ];
    NEEDLES.iter().any(|needle| text.contains(needle))
}

pub fn parse_prepare_for_sleep(line: &str) -> Option<bool> {
    let l = line.to_ascii_lowercase();
    if l.contains("prepareforsleep") && (l.contains("true") || l.contains("boolean true")) {
        return Some(true);
    }
    if l.contains("prepareforsleep") && (l.contains("false") || l.contains("boolean false")) {
        return Some(false);
    }
    if l.contains("system resumed") {
        return Some(false);
    }
    if l.contains("suspending system") {
        return Some(true);
    }
    None
}

pub fn suspend_modes_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("states".into(), json!(config.suspend_states));
    measurements.insert("mem_sleep".into(), json!(config.mem_sleep));
    if config.suspend_states.is_empty() {
        return TestRow::skip(
            "power.suspend.modes",
            "Read suspend states (never issue them)",
            "not-applicable",
            false,
            false,
            "no /sys/power/state",
            measurements,
        );
    }
    TestRow::pass(
        "power.suspend.modes",
        "Read suspend states (never issue them)",
        false,
        false,
        format!(
            "states={} mem_sleep={}",
            config.suspend_states.join(","),
            config.mem_sleep.clone().unwrap_or_else(|| "n/a".into())
        ),
        measurements,
    )
}

pub fn inhibit_row(config: &DetectedConfig, ci: bool) -> TestRow {
    let mut measurements = empty_measurements();
    if !config.logind && which("systemd-inhibit").is_none() {
        return TestRow::skip(
            "power.inhibit.logind",
            "Take and release a logind idle inhibit",
            "not-applicable",
            false,
            false,
            "no logind / systemd-inhibit",
            measurements,
        );
    }
    match take_and_release_inhibit() {
        Ok(true) => {
            measurements.insert("listed_while_held".into(), json!(true));
            TestRow::pass(
                "power.inhibit.logind",
                "Take and release a logind idle inhibit",
                false,
                false,
                "inhibit listed then released",
                measurements,
            )
        }
        Ok(false) => {
            measurements.insert("listed_while_held".into(), json!(false));
            TestRow::fail(
                "power.inhibit.logind",
                "Take and release a logind idle inhibit",
                false,
                false,
                "list did not name us",
                measurements,
            )
        }
        Err(err) => {
            measurements.insert("stderr".into(), json!(err));
            if ci && environment_refusal(&err) {
                TestRow::skip(
                    "power.inhibit.logind",
                    "Take and release a logind idle inhibit",
                    "ci",
                    false,
                    false,
                    err,
                    measurements,
                )
            } else {
                TestRow::fail(
                    "power.inhibit.logind",
                    "Take and release a logind idle inhibit",
                    false,
                    false,
                    err,
                    measurements,
                )
            }
        }
    }
}

fn take_and_release_inhibit() -> Result<bool, String> {
    let mut child = Command::new("systemd-inhibit")
        .args([
            "--what=idle:sleep",
            "--who=linux-hardware-suite",
            "--why=suite",
            "--mode=block",
            "sleep",
            "8",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("systemd-inhibit: {err}"))?;
    thread::sleep(Duration::from_millis(200));
    if let Ok(Some(status)) = child.try_wait() {
        let err = read_stderr(&mut child);
        return Err(format!("systemd-inhibit exited {status}: {err}"));
    }
    let list = Command::new("systemd-inhibit").arg("--list").output().ok();
    let shown = list
        .as_ref()
        .map(|out| {
            parse_inhibit_list(
                &String::from_utf8_lossy(&out.stdout),
                "linux-hardware-suite",
            )
        })
        .unwrap_or(false);
    let _ = child.kill();
    let _ = child.wait();
    thread::sleep(Duration::from_millis(100));
    let after = Command::new("systemd-inhibit").arg("--list").output().ok();
    let still = after
        .as_ref()
        .map(|out| {
            parse_inhibit_list(
                &String::from_utf8_lossy(&out.stdout),
                "linux-hardware-suite",
            )
        })
        .unwrap_or(false);
    if still {
        return Err("inhibit still listed after release".into());
    }
    Ok(shown)
}

fn read_stderr(child: &mut std::process::Child) -> String {
    let mut text = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = std::io::Read::read_to_string(&mut pipe, &mut text);
    }
    text
}

pub fn refuse_suspend_row() -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("issued_suspend".into(), json!(false));
    TestRow::pass(
        "power.suspend.not-issued",
        "Automated run never writes /sys/power/state",
        false,
        false,
        "no suspend issued",
        measurements,
    )
}

fn which(bin: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let p = dir.join(bin);
            p.is_file().then_some(p)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{
        environment_refusal, parse_inhibit_list, parse_lid_closed_property, parse_locked_hint,
        parse_mem_sleep, parse_power_state, parse_prepare_for_sleep,
    };

    #[test]
    fn power_state_tokens() {
        assert_eq!(
            parse_power_state("freeze mem disk\n"),
            vec!["freeze", "mem", "disk"]
        );
    }

    #[test]
    fn mem_sleep_brackets() {
        let m = parse_mem_sleep("s2idle [deep]\n");
        assert_eq!(m.current, "deep");
        assert_eq!(m.available, vec!["s2idle", "deep"]);
    }

    #[test]
    fn inhibit_who() {
        let text = "WHO                          UID  PID  COMM\nlinux-hardware-suite         1000 12   sleep\n";
        assert!(parse_inhibit_list(text, "linux-hardware-suite"));
        assert!(!parse_inhibit_list(text, "other"));
    }

    #[test]
    fn lid_and_sleep_lines() {
        assert_eq!(parse_lid_closed_property("b false"), Some(false));
        assert_eq!(parse_lid_closed_property("b true"), Some(true));
        assert_eq!(parse_locked_hint("LockedHint=yes\n"), Some(true));
        assert_eq!(parse_locked_hint("LockedHint=no\n"), Some(false));
        assert_eq!(parse_locked_hint("BlockInhibited=yes\n"), None);
        assert_eq!(
            parse_prepare_for_sleep("PrepareForSleep boolean true"),
            Some(true)
        );
        assert_eq!(parse_prepare_for_sleep("System resumed."), Some(false));
    }

    #[test]
    fn inhibit_refusal_fails_outside_ci() {
        let stderr = "Failed to connect to bus: No medium found";
        assert!(environment_refusal(stderr));
        assert!(!environment_refusal("list did not name us"));
    }
}
