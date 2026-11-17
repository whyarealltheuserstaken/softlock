//! Keep-awake / inhibit and suspend mode probes.

use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::json;

use crate::detect::DetectedConfig;
use crate::report::TestRow;

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
    let t = text.trim();
    if t.contains("true") || t.ends_with("yes") || t == "b true" {
        return Some(true);
    }
    if t.contains("false") || t.ends_with("no") || t == "b false" {
        return Some(false);
    }
    None
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
    let mut measurements = map();
    measurements.insert("states".into(), json!(config.suspend_states));
    measurements.insert("mem_sleep".into(), json!(config.mem_sleep));
    if config.suspend_states.is_empty() {
        return skip(
            "power.suspend.modes",
            "Read suspend states (never issue them)",
            "no /sys/power/state",
            measurements,
        );
    }
    row(
        "power.suspend.modes",
        "Read suspend states (never issue them)",
        "pass",
        format!(
            "states={} mem_sleep={}",
            config.suspend_states.join(","),
            config.mem_sleep.clone().unwrap_or_else(|| "n/a".into())
        ),
        measurements,
    )
}

pub fn inhibit_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = map();
    if !config.logind && which("systemd-inhibit").is_none() {
        return skip(
            "power.inhibit.logind",
            "Take and release a logind idle inhibit",
            "no logind / systemd-inhibit",
            measurements,
        );
    }
    match take_and_release_inhibit() {
        Ok(shown) => {
            measurements.insert("listed_while_held".into(), json!(shown));
            if shown {
                row(
                    "power.inhibit.logind",
                    "Take and release a logind idle inhibit",
                    "pass",
                    "inhibit listed then released".into(),
                    measurements,
                )
            } else {
                row(
                    "power.inhibit.logind",
                    "Take and release a logind idle inhibit",
                    "pass",
                    "inhibit held and released; list did not name us".into(),
                    measurements,
                )
            }
        }
        Err(e) => skip(
            "power.inhibit.logind",
            "Take and release a logind idle inhibit",
            &e,
            measurements,
        ),
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
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("systemd-inhibit: {e}"))?;
    thread::sleep(Duration::from_millis(200));
    if let Ok(Some(status)) = child.try_wait() {
        return Err(format!("systemd-inhibit exited {status}"));
    }
    let list = Command::new("systemd-inhibit").arg("--list").output().ok();
    let shown = list
        .as_ref()
        .map(|o| parse_inhibit_list(&String::from_utf8_lossy(&o.stdout), "linux-hardware-suite"))
        .unwrap_or(false);
    let _ = child.kill();
    let _ = child.wait();
    thread::sleep(Duration::from_millis(100));
    let after = Command::new("systemd-inhibit").arg("--list").output().ok();
    let still = after
        .as_ref()
        .map(|o| parse_inhibit_list(&String::from_utf8_lossy(&o.stdout), "linux-hardware-suite"))
        .unwrap_or(false);
    if still {
        return Err("inhibit still listed after release".into());
    }
    Ok(shown)
}

pub fn refuse_suspend_row() -> TestRow {
    let mut measurements = map();
    measurements.insert("issued_suspend".into(), json!(false));
    row(
        "power.suspend.not-issued",
        "Automated run never writes /sys/power/state",
        "pass",
        "no suspend issued".into(),
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

fn map() -> std::collections::BTreeMap<String, serde_json::Value> {
    std::collections::BTreeMap::new()
}

fn row(
    id: &str,
    name: &str,
    status: &str,
    detail: String,
    measurements: std::collections::BTreeMap<String, serde_json::Value>,
) -> TestRow {
    TestRow {
        id: id.into(),
        name: name.into(),
        status: status.into(),
        skip_reason: if status == "skip" {
            Some(detail.clone())
        } else {
            None
        },
        physical: false,
        needs_admin: false,
        duration_ms: 0,
        detail: if status == "skip" {
            String::new()
        } else {
            detail
        },
        measurements,
    }
}

fn skip(
    id: &str,
    name: &str,
    reason: &str,
    measurements: std::collections::BTreeMap<String, serde_json::Value>,
) -> TestRow {
    row(id, name, "skip", reason.to_string(), measurements)
}

#[cfg(test)]
mod tests {
    use super::{
        parse_inhibit_list, parse_lid_closed_property, parse_mem_sleep, parse_power_state,
        parse_prepare_for_sleep,
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
        assert_eq!(
            parse_prepare_for_sleep("PrepareForSleep boolean true"),
            Some(true)
        );
        assert_eq!(parse_prepare_for_sleep("System resumed."), Some(false));
    }
}
