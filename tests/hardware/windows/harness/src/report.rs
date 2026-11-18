//! `result.json` and the short `result.txt` summary.
//!
//! The JSON matches schema `softlock-hardware-suite/1`.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::SUITE_NAME;
use crate::machine::Machine;
use crate::plan::{Action, Flags, Planned, plan};
use crate::redact::redact;

pub const SCHEMA: &str = "softlock-hardware-suite/1";

const SKIP_REASONS: &[&str] = &[
    "no-input",
    "budget",
    "needs-admin",
    "not-applicable",
    "ci",
    "operator",
    "interrupted",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub started_utc: String,
    pub finished_utc: String,
    pub interrupted: bool,
    pub restored: bool,
    pub locked: bool,
    pub fail_closed: bool,
    pub hklm_command: String,
}

impl Default for Outcome {
    fn default() -> Self {
        Self {
            started_utc: String::new(),
            finished_utc: String::new(),
            interrupted: false,
            restored: true,
            locked: false,
            fail_closed: false,
            hklm_command: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub mode: String,
    pub result: String,
    pub elapsed_ms: u64,
    pub machine: Machine,
    pub rows: Vec<Row>,
    pub outcome: Outcome,
    pub summary_pass: u64,
    pub summary_fail: u64,
    pub summary_skip: u64,
}

#[derive(Debug, Deserialize)]
struct Supplied {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    measurements: BTreeMap<String, Value>,
}

pub fn render(
    machine: &Machine,
    flags: &Flags,
    elapsed_ms: u64,
    supplied: &[Value],
    outcome: &Outcome,
) -> Report {
    let planned = plan(machine, flags);
    let supplied = parse_supplied(supplied);
    let mut rows = Vec::new();
    for item in &planned {
        rows.push(merge_row(item, machine, supplied.get(item.id)));
    }
    let summary_pass = rows.iter().filter(|row| row.status == "pass").count() as u64;
    let summary_fail = rows.iter().filter(|row| row.status == "fail").count() as u64;
    let summary_skip = rows.iter().filter(|row| row.status == "skip").count() as u64;
    let result = if summary_fail == 0 { "pass" } else { "fail" };
    Report {
        mode: profile(flags),
        result: result.to_string(),
        elapsed_ms,
        machine: machine.clone(),
        rows,
        outcome: outcome.clone(),
        summary_pass,
        summary_fail,
        summary_skip,
    }
}

pub fn conform(value: &Value) -> Result<(), String> {
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .ok_or("missing schema")?;
    if schema != SCHEMA {
        return Err(format!("schema is {schema}"));
    }
    if value
        .get("suite")
        .and_then(Value::as_str)
        .unwrap_or("")
        .is_empty()
    {
        return Err("missing suite".into());
    }
    if !value.get("config").is_some_and(Value::is_object) {
        return Err("config must be an object".into());
    }
    if value.get("serial_collected") != Some(&Value::Bool(false)) {
        return Err("serial_collected must be false".into());
    }
    let tests = value
        .get("tests")
        .and_then(Value::as_array)
        .ok_or("tests must be an array")?;
    let mut pass = 0u64;
    let mut fail = 0u64;
    let mut skip = 0u64;
    for test in tests {
        let status = test
            .get("status")
            .and_then(Value::as_str)
            .ok_or("test is missing status")?;
        match status {
            "pass" => pass += 1,
            "fail" => fail += 1,
            "skip" => skip += 1,
            other => return Err(format!("status {other} is not pass, fail, or skip")),
        }
        let reason = test.get("skip_reason").cloned().unwrap_or(Value::Null);
        if status == "skip" {
            let code = reason.as_str().ok_or("skip is missing skip_reason")?;
            if !SKIP_REASONS.contains(&code) {
                return Err(format!("skip_reason {code} is not in the schema"));
            }
        } else if !reason.is_null() {
            return Err("pass and fail rows omit skip_reason".into());
        }
    }
    let summary = value.get("summary").ok_or("missing summary")?;
    let counted = |name: &str| summary.get(name).and_then(Value::as_u64);
    if counted("pass") != Some(pass)
        || counted("fail") != Some(fail)
        || counted("skip") != Some(skip)
    {
        return Err("summary does not match the tests".into());
    }
    let state = value.get("state").and_then(Value::as_str).unwrap_or("");
    if state != "never-unlocked" && state != "os-locked" {
        return Err("state must be never-unlocked or os-locked".into());
    }
    let text = value.to_string();
    if text.to_ascii_lowercase().contains("serialnumber")
        || text.contains("PCI\\VEN_")
        || text.contains("USB\\VID_")
    {
        return Err("result collected a hardware id".into());
    }
    Ok(())
}

fn profile(flags: &Flags) -> String {
    crate::plan::profile_name(flags).to_string()
}

fn parse_supplied(values: &[Value]) -> BTreeMap<String, Supplied> {
    let mut map = BTreeMap::new();
    for value in values {
        let Ok(row) = serde_json::from_value::<Supplied>(value.clone()) else {
            continue;
        };
        map.insert(row.id.clone(), row);
    }
    map
}

fn merge_row(item: &Planned, machine: &Machine, supplied: Option<&Supplied>) -> Row {
    if let Some(supplied) = supplied {
        return supplied_row(item, supplied);
    }
    match item.action {
        Action::Skip => Row {
            id: item.id.to_string(),
            name: row_name(item.id),
            status: "skip".to_string(),
            skip_reason: Some(canonical_skip(&item.reason)),
            physical: item.physical,
            needs_admin: item.needs_admin,
            duration_ms: 0,
            detail: if item.detail.is_empty() {
                item.reason.clone()
            } else {
                item.detail.clone()
            },
            measurements: BTreeMap::new(),
        },
        Action::Record => recorded(item, machine),
        Action::Run | Action::Prompt => Row {
            id: item.id.to_string(),
            name: row_name(item.id),
            status: "fail".to_string(),
            skip_reason: None,
            physical: item.physical,
            needs_admin: item.needs_admin,
            duration_ms: 0,
            detail: "missing result".to_string(),
            measurements: BTreeMap::new(),
        },
    }
}

fn supplied_row(item: &Planned, supplied: &Supplied) -> Row {
    let status = normalize_status(&supplied.status);
    let (skip_reason, detail) = if status == "skip" {
        (
            Some(canonical_skip(&supplied.reason)),
            detail_text(&supplied.reason),
        )
    } else {
        (None, redact(&supplied.reason))
    };
    Row {
        id: item.id.to_string(),
        name: row_name(item.id),
        status,
        skip_reason,
        physical: item.physical,
        needs_admin: item.needs_admin,
        duration_ms: supplied.duration_ms,
        detail,
        measurements: clean_measurements(&supplied.measurements),
    }
}

fn canonical_skip(reason: &str) -> String {
    let token = reason.trim().to_ascii_lowercase();
    if SKIP_REASONS.contains(&token.as_str()) {
        return token;
    }
    if token.contains("admin") || token.contains("elevat") {
        return "needs-admin".to_string();
    }
    if token == "budget" || token.contains("budget") {
        return "budget".to_string();
    }
    if token == "operator" || token == "skip" || token == "skipped" {
        return "operator".to_string();
    }
    if token == "interrupted" {
        return "interrupted".to_string();
    }
    if token.contains("stdin") || token.contains("eof") || token.contains("no-input") {
        return "no-input".to_string();
    }
    if token == "ci" || token.contains("--ci") || token.contains("physical step") {
        return "ci".to_string();
    }
    "not-applicable".to_string()
}

fn detail_text(reason: &str) -> String {
    let code = canonical_skip(reason);
    if reason.trim().eq_ignore_ascii_case(&code) {
        String::new()
    } else {
        redact(reason)
    }
}

fn recorded(item: &Planned, machine: &Machine) -> Row {
    let mut measurements = BTreeMap::new();
    let (status, detail) = if item.id == "detect-config" {
        if machine.build == 0 {
            ("fail", "could not read the Windows build".to_string())
        } else {
            measurements.insert("windows".to_string(), json!(machine.windows));
            measurements.insert("build".to_string(), json!(machine.build));
            measurements.insert("sleep".to_string(), json!(machine.power.sleep_label()));
            measurements.insert("displays".to_string(), json!(machine.displays));
            measurements.insert(
                "internal".to_string(),
                json!(count_text(machine.internal_displays)),
            );
            measurements.insert(
                "external".to_string(),
                json!(count_text(machine.external_displays)),
            );
            measurements.insert(
                "configured".to_string(),
                json!(count_text(machine.configured_displays)),
            );
            measurements.insert("hello".to_string(), json!(machine.hello.as_str()));
            measurements.insert("session".to_string(), json!(machine.session.as_str()));
            measurements.insert("lid".to_string(), json!(machine.lid));
            measurements.insert("ac".to_string(), json!(machine.ac));
            (
                "pass",
                format!(
                    "displays={} internal={} external={} configured={}",
                    machine.displays,
                    count_text(machine.internal_displays),
                    count_text(machine.external_displays),
                    count_text(machine.configured_displays),
                ),
            )
        }
    } else {
        ("fail", "unknown recorded row".to_string())
    };
    Row {
        id: item.id.to_string(),
        name: row_name(item.id),
        status: status.to_string(),
        skip_reason: None,
        physical: false,
        needs_admin: false,
        duration_ms: 0,
        detail,
        measurements,
    }
}

fn full_detail(id: &str) -> bool {
    matches!(id, "restore" | "display-hotplug" | "detect-config")
}

fn count_text(count: Option<u32>) -> String {
    match count {
        Some(value) => value.to_string(),
        None => "?".to_string(),
    }
}

fn normalize_status(status: &str) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "pass" | "fail" | "skip" => status.trim().to_ascii_lowercase(),
        _ => "fail".to_string(),
    }
}

fn row_name(id: &str) -> String {
    match id {
        "detect-config" => "Detect this machine",
        "parser-self-test" => "Parser self-test",
        "overlay-topmost" => "Overlay stays on top",
        "overlay-capture" => "Overlay is excluded from capture",
        "toast-hkcu-restore" => "Toast policy restores",
        "input-hook-injected" => "Injected key is observed",
        "keep-awake-request" => "Keep-awake request",
        "cad-hkcu-readback" => "HKCU secure-attention policy",
        "cad-hklm-readback" => "HKLM fast-user-switching policy",
        "fail-closed-blocked" => "Fail-closed lock is blocked",
        "display-enum" => "Display enumeration",
        "display-hotplug" => "Display hotplug",
        "lid-cycle" => "Lid switch",
        "power-key" => "Power button",
        "session-lock" => "Session lock",
        "restore" => "Restore settings",
        _ => id,
    }
    .to_string()
}

fn clean_measurements(measurements: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    let mut clean = BTreeMap::new();
    for (key, value) in measurements {
        let key = key.to_ascii_lowercase();
        if key.contains("serial") || key.contains("hardware") || key.contains("guid") {
            continue;
        }
        clean.insert(key, redact_value(value));
    }
    clean
}

fn redact_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact(text)),
        other => other.clone(),
    }
}

impl Report {
    pub fn state(&self) -> &'static str {
        if self.outcome.locked {
            "os-locked"
        } else {
            "never-unlocked"
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "schema": SCHEMA,
            "suite": SUITE_NAME,
            "started_utc": self.outcome.started_utc,
            "finished_utc": self.outcome.finished_utc,
            "duration_ms": self.elapsed_ms,
            "mode": self.mode,
            "config": self.machine.to_json(),
            "tests": self.rows.iter().map(row_json).collect::<Vec<_>>(),
            "summary": {
                "pass": self.summary_pass,
                "fail": self.summary_fail,
                "skip": self.summary_skip,
            },
            "fail_closed": self.outcome.fail_closed,
            "serial_collected": false,
            "restored": self.outcome.restored,
            "interrupted": self.outcome.interrupted,
            "state": self.state(),
        })
    }

    pub fn to_text(&self) -> String {
        let machine = &self.machine;
        let mut lines = vec![
            format!("Windows hardware suite  schema={SCHEMA}  suite={SUITE_NAME}"),
            format!(
                "pass={} fail={} skip={} duration={}s state={} serial-collected=false restored={} interrupted={}",
                self.summary_pass,
                self.summary_fail,
                self.summary_skip,
                self.elapsed_ms / 1000,
                self.state(),
                self.outcome.restored,
                self.outcome.interrupted
            ),
            format!("mode: {}", self.mode),
            format!(
                "Windows {} build {} | {} | displays {} internal {} external {} | hello {} | {} | lid {}",
                machine.windows,
                machine.build,
                machine.power.sleep_label(),
                machine.displays,
                count_text(machine.internal_displays),
                count_text(machine.external_displays),
                machine.hello.as_str(),
                machine.session.as_str(),
                machine.lid,
            ),
            String::new(),
            format!("{:<24} {:<6} {}", "id", "status", "detail"),
        ];
        for row in &self.rows {
            let detail = if row.status == "skip" {
                row.skip_reason.clone().unwrap_or_default()
            } else if row.detail.is_empty() {
                measurement_detail(&row.measurements)
            } else {
                row.detail.clone()
            };
            let shown = if full_detail(&row.id) {
                detail
            } else {
                truncate(&detail, 60)
            };
            lines.push(format!("{:<24} {:<6} {}", row.id, row.status, shown));
        }
        if !self.outcome.hklm_command.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "HKLM was not restored. From an elevated prompt run: {}",
                self.outcome.hklm_command
            ));
        }
        lines.push(String::new());
        lines.join("\n")
    }
}

fn row_json(row: &Row) -> Value {
    json!({
        "id": row.id,
        "name": row.name,
        "status": row.status,
        "skip_reason": row.skip_reason,
        "physical": row.physical,
        "needs_admin": row.needs_admin,
        "duration_ms": row.duration_ms,
        "detail": row.detail,
        "measurements": row.measurements,
    })
}

fn measurement_detail(measurements: &BTreeMap<String, Value>) -> String {
    measurements
        .iter()
        .take(3)
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(text: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.chars().count() == max {
            return out;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::decode_facts;
    use crate::plan::{Action, BUDGET_MS, Flags};

    fn sample() -> (Machine, Flags) {
        let machine = decode_facts(&serde_json::json!({
            "current_build": "26100",
            "installation_type": "Client",
            "powercfg_a": "The following sleep states are available on this system:\n    Standby (S0 Low Power Idle)\nThe following sleep states are not available on this system:\n    Standby (S3)\n",
            "monitor_count": 1,
            "biometric": false,
            "pin": true,
            "client_protocol": 0,
            "elevated": false,
            "chassis_laptop": true,
            "ac_line": 0
        }))
        .unwrap();
        let flags = Flags {
            ci: true,
            admin: false,
            elevated: true,
            github_actions: true,
            school: false,
        };
        (machine, flags)
    }

    fn outcome() -> Outcome {
        Outcome {
            started_utc: "2026-11-17T18:00:00Z".to_string(),
            finished_utc: "2026-11-17T18:00:42Z".to_string(),
            interrupted: false,
            restored: true,
            locked: false,
            fail_closed: false,
            hklm_command: String::new(),
        }
    }

    #[test]
    fn a_ci_summary_fits_on_one_screen_and_drops_a_serial() {
        let (machine, flags) = sample();
        let supplied = vec![serde_json::json!({
            "id": "overlay-topmost",
            "status": "pass",
            "reason": "",
            "measurements": {"topmost": true, "serial": "ZZZ"}
        })];
        let report = render(&machine, &flags, 42_000, &supplied, &outcome());
        assert_eq!(report.mode, "ci");
        assert_eq!(report.result, "fail");
        let text = report.to_text();
        assert!(text.lines().count() < 30, "{text}");
        assert!(text.starts_with(
            "Windows hardware suite  schema=softlock-hardware-suite/1  suite=windows"
        ));
        assert!(text.contains("pass="));
        assert!(text.contains("serial-collected=false"));
        assert!(text.contains("mode: ci"));
        assert!(!text.contains("ZZZ"));
        assert!(!report.to_json().to_string().contains("ZZZ"));
        let topmost = report
            .rows
            .iter()
            .find(|row| row.id == "overlay-topmost")
            .unwrap();
        assert!(!topmost.measurements.contains_key("serial"));
        let hotplug = report
            .rows
            .iter()
            .find(|row| row.id == "display-hotplug")
            .unwrap();
        assert_eq!(hotplug.status, "skip");
        assert_eq!(hotplug.skip_reason.as_deref(), Some("ci"));
        assert!(
            report
                .rows
                .iter()
                .all(|row| row.id != "ci-probes-stayed-off")
        );
    }

    #[test]
    fn budget_drops_are_skips_and_do_not_fail_the_run() {
        let (machine, flags) = sample();
        let planned = plan(&machine, &flags);
        let supplied: Vec<Value> = planned
            .iter()
            .filter(|row| matches!(row.action, Action::Run | Action::Prompt))
            .map(|row| {
                serde_json::json!({
                    "id": row.id,
                    "status": "skip",
                    "reason": "budget",
                })
            })
            .collect();
        let report = render(&machine, &flags, BUDGET_MS + 1, &supplied, &outcome());
        assert_eq!(report.result, "pass");
        assert!(report.summary_skip > 0);
        assert!(
            report.rows.iter().any(|row| {
                row.status == "skip" && row.skip_reason.as_deref() == Some("budget")
            })
        );
        assert!(report.rows.iter().all(|row| row.id != "budget"));
        conform(&report.to_json()).unwrap();
    }

    #[test]
    fn a_physical_step_that_was_done_but_missed_is_a_failure() {
        let (machine, flags) = sample();
        let supplied = vec![serde_json::json!({
            "id": "display-hotplug",
            "status": "fail",
            "reason": "not detected",
        })];
        let mut school = flags.clone();
        school.ci = false;
        let report = render(&machine, &school, 1_000, &supplied, &outcome());
        let hotplug = report
            .rows
            .iter()
            .find(|row| row.id == "display-hotplug")
            .unwrap();
        assert_eq!(hotplug.status, "fail");
        assert!(hotplug.skip_reason.is_none());
        assert_eq!(hotplug.detail, "not detected");
    }

    #[test]
    fn an_armed_failure_reports_os_locked() {
        let (machine, flags) = sample();
        let mut outcome = outcome();
        outcome.locked = true;
        outcome.fail_closed = false;
        let locked = render(&machine, &flags, 1_000, &[], &outcome);
        assert_eq!(locked.state(), "os-locked");
        outcome.fail_closed = true;
        let report = render(&machine, &flags, 1_000, &[], &outcome);
        assert_eq!(report.state(), "os-locked");
        assert_eq!(report.to_json()["fail_closed"], true);
        assert_eq!(report.to_json()["serial_collected"], false);
        conform(&report.to_json()).unwrap();
    }

    #[test]
    fn a_personal_laptop_is_bench_and_the_restore_line_is_complete() {
        let (machine, mut flags) = sample();
        flags.ci = false;
        flags.elevated = false;
        flags.github_actions = false;
        let command = "reg add \"HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\System\" /v DisableLockWorkstation /t REG_DWORD /d 0 /f";
        let detail = format!(
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\System value DisableLockWorkstation was changed and was not put back. Run: {command}"
        );
        let supplied = vec![serde_json::json!({
            "id": "restore",
            "status": "fail",
            "reason": detail,
        })];
        let report = render(&machine, &flags, 1_000, &supplied, &outcome());
        assert_eq!(report.mode, "bench");
        assert!(report.to_text().contains(command));
        flags.school = true;
        let school = render(&machine, &flags, 1_000, &[], &outcome());
        assert_eq!(school.mode, "school");
    }
}
