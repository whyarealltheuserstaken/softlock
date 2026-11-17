//! `result.json` and the short `result.txt` summary.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::SUITE_NAME;
use crate::machine::Machine;
use crate::plan::{Action, BUDGET_MS, Flags, Planned, plan};
use crate::redact::redact;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub status: String,
    pub reason: String,
    pub measurements: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub profile: String,
    pub result: String,
    pub elapsed_ms: u64,
    pub machine: Machine,
    pub rows: Vec<Row>,
}

#[derive(Debug, Deserialize)]
struct Supplied {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    measurements: BTreeMap<String, Value>,
}

pub fn render(machine: &Machine, flags: &Flags, elapsed_ms: u64, supplied: &[Value]) -> Report {
    let planned = plan(machine, flags);
    let supplied = parse_supplied(supplied);
    let mut rows = Vec::new();
    for item in &planned {
        if item.id == "budget" {
            continue;
        }
        rows.push(merge_row(item, machine, supplied.get(item.id)));
    }
    rows.push(budget_row(elapsed_ms));
    let result = if rows.iter().any(|row| row.status == "fail") {
        "fail"
    } else {
        "pass"
    };
    Report {
        profile: profile(flags),
        result: result.to_string(),
        elapsed_ms,
        machine: machine.clone(),
        rows,
    }
}

fn profile(flags: &Flags) -> String {
    if flags.ci {
        "ci".to_string()
    } else if flags.admin {
        "admin".to_string()
    } else {
        "school".to_string()
    }
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
        return Row {
            id: item.id.to_string(),
            status: normalize_status(&supplied.status),
            reason: redact(&supplied.reason),
            measurements: clean_measurements(&supplied.measurements),
        };
    }
    match item.action {
        Action::Skip => Row {
            id: item.id.to_string(),
            status: "skip".to_string(),
            reason: item.reason.clone(),
            measurements: BTreeMap::new(),
        },
        Action::Record => recorded(item.id, machine),
        Action::Run | Action::Prompt => Row {
            id: item.id.to_string(),
            status: "fail".to_string(),
            reason: "missing result".to_string(),
            measurements: BTreeMap::new(),
        },
    }
}

fn recorded(id: &str, machine: &Machine) -> Row {
    let mut measurements = BTreeMap::new();
    let (status, reason) = if id == "detect-config" {
        if machine.build == 0 {
            ("fail", "could not read the Windows build".to_string())
        } else {
            measurements.insert("windows".to_string(), json!(machine.windows));
            measurements.insert("build".to_string(), json!(machine.build));
            measurements.insert("sleep".to_string(), json!(machine.power.sleep_label()));
            measurements.insert("displays".to_string(), json!(machine.displays));
            measurements.insert("hello".to_string(), json!(machine.hello.as_str()));
            measurements.insert("session".to_string(), json!(machine.session.as_str()));
            measurements.insert("lid".to_string(), json!(machine.lid));
            measurements.insert("ac".to_string(), json!(machine.ac));
            ("pass", String::new())
        }
    } else if id == "ci-probes-stayed-off" {
        measurements.insert("launched".to_string(), json!(false));
        ("pass", "service not installed".to_string())
    } else {
        ("fail", "unknown recorded row".to_string())
    };
    Row {
        id: id.to_string(),
        status: status.to_string(),
        reason,
        measurements,
    }
}

fn budget_row(elapsed_ms: u64) -> Row {
    let mut measurements = BTreeMap::new();
    measurements.insert("elapsed_ms".to_string(), json!(elapsed_ms));
    measurements.insert("budget_ms".to_string(), json!(BUDGET_MS));
    let ok = elapsed_ms <= BUDGET_MS;
    Row {
        id: "budget".to_string(),
        status: if ok { "pass" } else { "fail" }.to_string(),
        reason: if ok {
            String::new()
        } else {
            format!("run took {elapsed_ms} ms")
        },
        measurements,
    }
}

fn normalize_status(status: &str) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "pass" | "fail" | "skip" => status.trim().to_ascii_lowercase(),
        _ => "fail".to_string(),
    }
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
    pub fn to_json(&self) -> Value {
        json!({
            "suite": SUITE_NAME,
            "profile": self.profile,
            "result": self.result,
            "elapsed_ms": self.elapsed_ms,
            "budget_ms": BUDGET_MS,
            "config": self.machine.to_json(),
            "tests": self.rows.iter().map(row_json).collect::<Vec<_>>(),
        })
    }

    pub fn to_text(&self) -> String {
        let machine = &self.machine;
        let mut lines = vec![
            "SoftLock Windows hardware suite".to_string(),
            format!("profile: {}", self.profile),
            format!("result: {}", self.result),
            format!("elapsed: {}s", self.elapsed_ms / 1000),
            format!(
                "Windows {} build {} | {} | displays {} | hello {} | {} | lid {}",
                machine.windows,
                machine.build,
                machine.power.sleep_label(),
                machine.displays,
                machine.hello.as_str(),
                machine.session.as_str(),
                machine.lid,
            ),
            String::new(),
            format!("{:<24} {:<6} {}", "id", "status", "detail"),
        ];
        for row in &self.rows {
            let detail = if row.reason.is_empty() {
                measurement_detail(&row.measurements)
            } else {
                row.reason.clone()
            };
            lines.push(format!(
                "{:<24} {:<6} {}",
                row.id,
                row.status,
                truncate(&detail, 60)
            ));
        }
        lines.push(String::new());
        lines.join("\n")
    }
}

fn row_json(row: &Row) -> Value {
    json!({
        "id": row.id,
        "status": row.status,
        "reason": row.reason,
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
    use crate::plan::Flags;

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
        };
        (machine, flags)
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
        let report = render(&machine, &flags, 42_000, &supplied);
        assert_eq!(report.profile, "ci");
        assert_eq!(report.result, "fail");
        let text = report.to_text();
        assert!(text.lines().count() < 30, "{text}");
        assert!(text.contains("profile: ci"));
        assert!(text.contains("elapsed: 42s"));
        assert!(!text.contains("ZZZ"));
        assert!(!report.to_json().to_string().contains("ZZZ"));
        let topmost = report
            .rows
            .iter()
            .find(|row| row.id == "overlay-topmost")
            .unwrap();
        assert!(!topmost.measurements.contains_key("serial"));
        assert_eq!(
            report
                .rows
                .iter()
                .find(|row| row.id == "display-hotplug")
                .unwrap()
                .status,
            "skip"
        );
        assert_eq!(
            report
                .rows
                .iter()
                .find(|row| row.id == "ci-probes-stayed-off")
                .unwrap()
                .status,
            "pass"
        );
    }

    #[test]
    fn over_budget_fails_the_run() {
        let (machine, flags) = sample();
        let report = render(&machine, &flags, BUDGET_MS + 1, &[]);
        assert_eq!(report.result, "fail");
        assert_eq!(
            report
                .rows
                .iter()
                .find(|row| row.id == "budget")
                .unwrap()
                .status,
            "fail"
        );
    }
}
