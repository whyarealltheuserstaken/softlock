//! Shared hardware-suite output (`softlock-hardware-suite/1`).

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::detect::DetectedConfig;
use crate::redact::redact;

pub const SCHEMA: &str = "softlock-hardware-suite/1";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Summary {
    pub pass: u32,
    pub fail: u32,
    pub skip: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct TestRow {
    pub id: String,
    pub name: String,
    pub status: String,
    pub skip_reason: Option<String>,
    pub physical: bool,
    pub needs_admin: bool,
    pub duration_ms: u64,
    pub detail: String,
    pub measurements: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SuiteResult {
    pub schema: String,
    pub suite: String,
    pub started_utc: String,
    pub finished_utc: String,
    pub duration_ms: u64,
    pub mode: String,
    pub config: DetectedConfig,
    pub tests: Vec<TestRow>,
    pub summary: Summary,
    pub fail_closed: bool,
    pub serial_collected: bool,
    pub restored: bool,
    pub interrupted: bool,
    pub state: String,
}

impl SuiteResult {
    #[allow(clippy::too_many_arguments)]
    pub fn finish(
        started_utc: String,
        started: SystemTime,
        mode: String,
        config: DetectedConfig,
        tests: Vec<TestRow>,
        restored: bool,
        interrupted: bool,
        state: String,
    ) -> Self {
        let mut summary = Summary {
            pass: 0,
            fail: 0,
            skip: 0,
        };
        for row in &tests {
            match row.status.as_str() {
                "pass" => summary.pass += 1,
                "fail" => summary.fail += 1,
                _ => summary.skip += 1,
            }
        }
        let duration_ms = started.elapsed().map(|d| d.as_millis() as u64).unwrap_or(0);
        Self {
            schema: SCHEMA.to_string(),
            suite: "linux".to_string(),
            started_utc,
            finished_utc: utc_stamp(SystemTime::now()),
            duration_ms,
            mode,
            config,
            tests,
            summary,
            fail_closed: true,
            serial_collected: false,
            restored,
            interrupted,
            state,
        }
    }

    pub fn to_json(&self) -> String {
        let raw = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string());
        redact(&format!("{raw}\n"))
    }

    pub fn to_txt(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "Linux hardware suite  schema={SCHEMA}  suite=linux"
        ));
        lines.push(format!(
            "pass={} fail={} skip={} duration={}s state={} serial-collected=false restored={} interrupted={}",
            self.summary.pass,
            self.summary.fail,
            self.summary.skip,
            self.duration_ms / 1000,
            self.state,
            self.restored,
            self.interrupted
        ));
        lines.push(format!(
            "config: distro={} {} kernel={} arch={} session={} compositor={} displays={} suspend={} sudo={}",
            self.config.distro_id,
            self.config.distro_version,
            self.config.kernel,
            self.config.arch,
            self.config.session_type,
            self.config.compositor,
            self.config.display_count,
            self.config.suspend_states.join(","),
            self.config.sudo
        ));
        lines.push("ID                           STATUS  ADMIN PHYS  NOTE".to_string());
        for row in &self.tests {
            let admin = if row.needs_admin { "yes" } else { "no " };
            let phys = if row.physical { "yes" } else { "no " };
            let note = row
                .skip_reason
                .clone()
                .unwrap_or_else(|| row.detail.clone());
            let note = truncate(&note, 48);
            lines.push(format!(
                "{:<28} {:<6}  {}   {}   {note}",
                row.id, row.status, admin, phys
            ));
        }
        redact(&lines.join("\n"))
    }
}

pub fn utc_stamp(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs:016}")
}

pub fn utc_dir_name(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

fn truncate(text: &str, max: usize) -> String {
    let one = text.replace('\n', " ");
    if one.chars().count() <= max {
        return one;
    }
    one.chars().take(max.saturating_sub(3)).collect::<String>() + "..."
}

#[cfg(test)]
mod tests {
    use super::{SCHEMA, SuiteResult, TestRow, utc_stamp};
    use crate::detect::{DetectedConfig, RemoteTools};
    use std::collections::BTreeMap;
    use std::time::SystemTime;

    fn config() -> DetectedConfig {
        DetectedConfig {
            os: "linux".into(),
            distro_id: "ubuntu".into(),
            distro_version: "24.04".into(),
            kernel: "6.8.0".into(),
            arch: "x86_64".into(),
            session_type: "none".into(),
            compositor: "none".into(),
            desktop: "none".into(),
            display_count: 0,
            displays: vec![],
            suspend_states: vec!["mem".into()],
            mem_sleep: Some("s2idle".into()),
            sudo: "none".into(),
            uid_is_root: false,
            logind: false,
            dbus_system: true,
            evdev_nodes: 0,
            uinput: "missing".into(),
            backend_input_guard: "unsupported".into(),
            backend_overlay: "unsupported".into(),
            backend_power: "unsupported".into(),
            remote: RemoteTools {
                xrdp: false,
                vnc: false,
                gnome_remote_desktop: false,
                rustdesk: false,
            },
        }
    }

    #[test]
    fn json_schema_and_txt_are_short() {
        let row = TestRow {
            id: "detect.config".into(),
            name: "Detect config".into(),
            status: "pass".into(),
            skip_reason: None,
            physical: false,
            needs_admin: false,
            duration_ms: 3,
            detail: String::new(),
            measurements: BTreeMap::new(),
        };
        let result = SuiteResult::finish(
            utc_stamp(SystemTime::UNIX_EPOCH),
            SystemTime::UNIX_EPOCH,
            "ci".into(),
            config(),
            vec![row],
            true,
            false,
            "never-unlocked".into(),
        );
        let json = result.to_json();
        assert!(json.contains(SCHEMA));
        assert!(json.contains("\"suite\": \"linux\""));
        assert!(json.contains("never-unlocked"));
        assert!(json.contains("serial_collected"));
        let txt = result.to_txt();
        assert!(txt.lines().count() <= 40);
        assert!(txt.contains("pass=1"));
        assert!(txt.contains("detect.config"));
    }
}
