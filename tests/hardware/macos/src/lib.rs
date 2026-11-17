//! Parse kit logs into one `softlock-hardware-suite/1` result.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const SCHEMA: &str = "softlock-hardware-suite/1";

pub const SKIP_REASONS: &[&str] = &[
    "no-input",
    "budget",
    "needs-admin",
    "not-applicable",
    "ci",
    "operator",
    "interrupted",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub pass: u32,
    pub fail: u32,
    pub skip: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub model: String,
    pub ci: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SuiteResult {
    pub schema: String,
    pub suite: String,
    pub started_utc: String,
    pub finished_utc: String,
    pub duration_ms: u64,
    pub mode: String,
    pub config: Config,
    pub tests: Vec<Row>,
    pub summary: Summary,
    pub fail_closed: bool,
    pub serial_collected: bool,
    pub restored: bool,
    pub interrupted: bool,
    pub state: String,
    pub build_ms: u64,
    pub test_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailClosedPlan {
    pub state: &'static str,
    pub lock: bool,
    pub argv: Vec<&'static str>,
}

pub fn field(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    text.lines()
        .find_map(|line| line.trim().strip_prefix(&prefix).map(str::to_string))
}

fn row(
    id: &str,
    name: &str,
    status: &str,
    skip_reason: Option<&str>,
    physical: bool,
    detail: String,
) -> Row {
    let mut measurements = BTreeMap::new();
    if let Some(reason) = skip_reason {
        measurements.insert("skip_reason".into(), json!(reason));
    }
    if !detail.is_empty() {
        measurements.insert("detail".into(), json!(detail));
    }
    Row {
        id: id.into(),
        name: name.into(),
        status: status.into(),
        skip_reason: skip_reason.map(str::to_string),
        physical,
        needs_admin: false,
        duration_ms: 0,
        detail,
        measurements,
    }
}

pub fn keep_awake_rows(text: &str) -> Vec<Row> {
    let mode = field(text, "mode").unwrap_or_default();
    let result = field(text, "result").unwrap_or_else(|| "missing".to_string());
    let released = field(text, "assertion-released").as_deref() == Some("true");
    let created = field(text, "assertion-created").as_deref() == Some("true");
    let assertions = if !created || !released || result == "fail" {
        "fail"
    } else {
        "pass"
    };
    let mut assertion = row(
        "m07-assertions",
        "Idle-sleep assertions",
        assertions,
        None,
        false,
        format!("result={result} created={created} released={released}"),
    );
    assertion.measurements.insert(
        "assertion-released".into(),
        json!(if released { "true" } else { "false" }),
    );
    let mut rows = vec![assertion];
    if mode == "runner" {
        rows.push(row(
            "m07-app-nap",
            "App Nap sample",
            "skip",
            Some("ci"),
            false,
            "not-run-on-runner".into(),
        ));
    } else {
        let verdict = field(text, "app-nap-verdict").unwrap_or_else(|| "missing".into());
        let (status, reason) = match verdict.as_str() {
            "napped" | "not-napped" => ("pass", None),
            "not-claimed" => ("skip", Some("not-applicable")),
            "" | "missing" => ("fail", None),
            _ => ("fail", None),
        };
        rows.push(row(
            "m07-app-nap",
            "App Nap sample",
            status,
            reason,
            false,
            format!(
                "verdict={verdict} max-gap-ms={} threshold-ms={}",
                field(text, "app-nap-max-gap-ms").unwrap_or_else(|| "missing".into()),
                field(text, "app-nap-threshold-ms").unwrap_or_else(|| "missing".into())
            ),
        ));
    }
    for (id, name, key) in [
        ("m07-lid-on-power", "Lid close on power", "lid-on-power"),
        (
            "m07-lid-external",
            "Lid close with an external display",
            "lid-external",
        ),
        ("m07-power-key", "Touch ID / power key", "power-key"),
    ] {
        rows.push(hardware_row(id, name, key, text, &mode));
    }
    if field(text, "interrupted").as_deref() == Some("true") {
        rows.push(row(
            "m07-interrupted",
            "Keep-awake interrupt",
            "fail",
            None,
            false,
            "Ctrl-C".into(),
        ));
    }
    rows
}

fn hardware_row(id: &str, name: &str, key: &str, text: &str, mode: &str) -> Row {
    let value = field(text, key).unwrap_or_else(|| "missing".into());
    let method = field(text, &format!("{key}-detection")).unwrap_or_default();
    let detail = format!("{value} method={method}");
    if mode == "runner" || value == "not-run-on-runner" {
        return row(id, name, "skip", Some("ci"), false, detail);
    }
    let physical = true;
    match value.as_str() {
        "slept" | "awake" | "restart-panel" | "nothing" if method != "none" => {
            row(id, name, "pass", None, physical, detail)
        }
        "no-external-display" => row(id, name, "skip", Some("not-applicable"), physical, detail),
        "skip" => row(id, name, "skip", Some("operator"), physical, detail),
        "no-input" => row(id, name, "skip", Some("no-input"), physical, detail),
        "interrupted" => row(id, name, "skip", Some("interrupted"), physical, detail),
        "not-detected" | "missing" | "error" => row(id, name, "fail", None, physical, detail),
        _ => row(id, name, "fail", None, physical, detail),
    }
}

pub fn lock_read_only_row(text: &str) -> Row {
    let result = field(text, "result").unwrap_or_else(|| "missing".into());
    let status = if result == "pass" { "pass" } else { "fail" };
    row(
        "m08-read-only",
        "Lock kit read-only",
        status,
        None,
        false,
        format!(
            "result={result} filevault={} session-lock-check={}",
            field(text, "filevault").unwrap_or_else(|| "missing".into()),
            field(text, "session-lock-check").unwrap_or_else(|| "missing".into())
        ),
    )
}

pub fn lock_exercise_row(id: &str, text: &str, ran: bool) -> Row {
    let name = if id.contains("documented") {
        "Documented lock"
    } else {
        "Private lock"
    };
    if !ran {
        return row(
            id,
            name,
            "skip",
            Some("ci"),
            true,
            "ci skips the lock exercise".into(),
        );
    }
    if text.contains("no-input") {
        return row(
            id,
            name,
            "skip",
            Some("no-input"),
            true,
            "stdin closed".into(),
        );
    }
    let result = field(text, "result").unwrap_or_else(|| "missing".into());
    let documented = field(text, "documented-path").unwrap_or_default();
    if documented == "refused" || result == "refuse" {
        return row(
            id,
            name,
            "skip",
            Some("not-applicable"),
            true,
            format!("result={result} documented-path={documented}"),
        );
    }
    let locked = field(text, "state").as_deref() == Some("os-locked")
        && field(text, "lock-screen-appeared").as_deref() == Some("yes")
        && field(text, "input-reached-desktop-without-login").as_deref() == Some("no");
    let status = if result == "pass" && locked {
        "pass"
    } else {
        "fail"
    };
    row(
        id,
        name,
        status,
        None,
        true,
        format!(
            "result={result} lock-screen={} input-without-login={}",
            field(text, "lock-screen-appeared").unwrap_or_else(|| "missing".into()),
            field(text, "input-reached-desktop-without-login").unwrap_or_else(|| "missing".into())
        ),
    )
}

pub fn smoke_row(text: &str, exit_code: i32, arch: &str) -> Row {
    if arch != "x86_64" {
        return row(
            "m11-smoke",
            "Native Intel smoke",
            "skip",
            Some("not-applicable"),
            false,
            format!("host arch is {arch}"),
        );
    }
    let passed = exit_code == 0
        && field(text, "smoke").as_deref() == Some("pass")
        && field(text, "fat").as_deref() == Some("true")
        && field(text, "native-intel").as_deref() == Some("true");
    row(
        "m11-smoke",
        "Native Intel smoke",
        if passed { "pass" } else { "fail" },
        None,
        false,
        format!(
            "exit={exit_code} smoke={} fat={} native-intel={}",
            field(text, "smoke").unwrap_or_else(|| "missing".into()),
            field(text, "fat").unwrap_or_else(|| "missing".into()),
            field(text, "native-intel").unwrap_or_else(|| "missing".into())
        ),
    )
}

pub fn exit_row(id: &str, name: &str, ok: bool, physical: bool, detail: &str) -> Row {
    row(
        id,
        name,
        if ok { "pass" } else { "fail" },
        None,
        physical,
        detail.into(),
    )
}

pub fn decided_skip(id: &str, name: &str, reason: &str, detail: &str, physical: bool) -> Row {
    row(id, name, "skip", Some(reason), physical, detail.into())
}

pub fn budget_skip(id: &str, name: &str, physical: bool) -> Row {
    row(
        id,
        name,
        "skip",
        Some("budget"),
        physical,
        "skipped before the prompt because the 5 minute budget was already spent".into(),
    )
}

pub fn over_budget(elapsed_ms: u64, limit_ms: u64) -> bool {
    elapsed_ms >= limit_ms
}

pub fn summary_of(rows: &[Row]) -> Summary {
    let mut summary = Summary {
        pass: 0,
        fail: 0,
        skip: 0,
    };
    for row in rows {
        match row.status.as_str() {
            "pass" => summary.pass += 1,
            "fail" => summary.fail += 1,
            _ => summary.skip += 1,
        }
    }
    summary
}

/// A failure on a real Mac locks. CI and an interrupt do not start a new lock.
/// The argv never unlocks.
pub fn fail_closed(
    any_fail: bool,
    ci: bool,
    interrupted: bool,
    session_locked: bool,
) -> FailClosedPlan {
    let state = if session_locked || (any_fail && !ci && !interrupted) {
        "os-locked"
    } else {
        "never-unlocked"
    };
    if interrupted || ci || !any_fail {
        return FailClosedPlan {
            state,
            lock: false,
            argv: Vec::new(),
        };
    }
    FailClosedPlan {
        state: "os-locked",
        lock: true,
        argv: vec!["--fail-closed"],
    }
}

/// Assertions the kit released, and no fail-closed lock left the session held.
pub fn restored_from(rows: &[Row], fail_closed_lock: bool) -> bool {
    if fail_closed_lock {
        return false;
    }
    rows.iter().all(|row| {
        row.measurements
            .get("assertion-released")
            .and_then(Value::as_str)
            != Some("false")
    })
}

pub fn suite_exit(summary: &Summary, interrupted: bool) -> i32 {
    if interrupted || summary.fail > 0 {
        1
    } else {
        0
    }
}

impl SuiteResult {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string()) + "\n"
    }

    pub fn to_txt(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "macOS hardware suite  schema={SCHEMA}  suite=macos"
        ));
        lines.push(format!(
            "pass={} fail={} skip={} duration={}s build={}s test={}s state={} serial-collected=false restored={} interrupted={}",
            self.summary.pass,
            self.summary.fail,
            self.summary.skip,
            self.duration_ms / 1000,
            self.build_ms / 1000,
            self.test_ms / 1000,
            self.state,
            self.restored,
            self.interrupted
        ));
        lines.push(format!(
            "config: os={} version={} arch={} model={} ci={}",
            self.config.os,
            self.config.os_version,
            self.config.arch,
            self.config.model,
            self.config.ci
        ));
        lines.push("ID                           STATUS  PHYS  NOTE".to_string());
        for row in &self.tests {
            let phys = if row.physical { "yes" } else { "no " };
            let note = row
                .skip_reason
                .clone()
                .filter(|reason| !reason.is_empty())
                .unwrap_or_else(|| row.detail.clone());
            lines.push(format!(
                "{:<28} {:<6}  {}   {}",
                row.id,
                row.status,
                phys,
                truncate(&note, 56)
            ));
        }
        lines.join("\n") + "\n"
    }
}

fn truncate(text: &str, max: usize) -> String {
    let one = text.replace('\n', " ");
    if one.chars().count() <= max {
        return one;
    }
    one.chars().take(max.saturating_sub(3)).collect::<String>() + "..."
}

pub fn conformance_errors(value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if value.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        errors.push("schema".into());
    }
    if value.get("config").and_then(Value::as_object).is_none() {
        errors.push("config".into());
    }
    let Some(summary) = value.get("summary") else {
        errors.push("summary".into());
        return errors;
    };
    for key in ["pass", "fail", "skip"] {
        if summary.get(key).and_then(Value::as_u64).is_none() {
            errors.push(format!("summary.{key}"));
        }
    }
    let Some(tests) = value.get("tests").and_then(Value::as_array) else {
        errors.push("tests".into());
        return errors;
    };
    for (index, test) in tests.iter().enumerate() {
        let status = test.get("status").and_then(Value::as_str).unwrap_or("");
        if !matches!(status, "pass" | "fail" | "skip") {
            errors.push(format!("tests[{index}].status"));
        }
        if status == "skip" {
            let reason = test
                .get("skip_reason")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !SKIP_REASONS.contains(&reason) {
                errors.push(format!("tests[{index}].skip_reason"));
            }
        }
    }
    errors
}

pub fn scrub_model(model: &str) -> String {
    let lower = model.to_ascii_lowercase();
    if lower.contains("serial") {
        String::new()
    } else {
        model.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_keep_awake_skips_hardware_and_passes_assertions() {
        let text = "\
mode=runner
assertion-created=true
assertion-released=true
result=pass
lid-on-power=not-run-on-runner
lid-external=not-run-on-runner
power-key=not-run-on-runner
";
        let rows = keep_awake_rows(text);
        assert_eq!(rows[0].status, "pass");
        assert_eq!(rows[1].status, "skip");
        assert_eq!(rows[1].skip_reason.as_deref(), Some("ci"));
        assert!(rows[2].status == "skip");
        assert!(
            rows.iter()
                .all(|row| matches!(row.status.as_str(), "pass" | "skip"))
        );
    }

    #[test]
    fn a_missed_physical_step_fails_and_a_chosen_skip_stays_a_skip() {
        let text = "\
mode=kit
assertion-created=true
assertion-released=true
result=pass
app-nap-verdict=napped
app-nap-max-gap-ms=1400
app-nap-threshold-ms=1000
lid-on-power=not-detected
lid-on-power-detection=none
lid-external=skip
lid-external-detection=operator-skip
power-key=nothing
power-key-detection=iokit-power-notification
";
        let rows = keep_awake_rows(text);
        assert_eq!(rows[1].status, "pass");
        assert_eq!(rows[2].status, "fail");
        assert!(rows[2].skip_reason.is_none());
        assert_eq!(rows[3].status, "skip");
        assert_eq!(rows[3].skip_reason.as_deref(), Some("operator"));
        assert_eq!(rows[4].status, "pass");
    }

    #[test]
    fn a_failed_assertion_is_a_failure() {
        let text = "\
mode=kit
assertion-created=false
assertion-released=false
result=fail
app-nap-verdict=not-claimed
lid-on-power=no-input
lid-external=no-external-display
power-key=not-run-on-runner
";
        let rows = keep_awake_rows(text);
        assert_eq!(rows[0].status, "fail");
        assert_eq!(rows[1].status, "skip");
        assert_eq!(rows[1].skip_reason.as_deref(), Some("not-applicable"));
        assert_eq!(rows[2].skip_reason.as_deref(), Some("no-input"));
        assert_eq!(rows[3].status, "skip");
        assert_eq!(rows[4].skip_reason.as_deref(), Some("ci"));
    }

    #[test]
    fn closed_stdin_is_skip_and_a_refused_lock_is_not_applicable() {
        let closed = lock_exercise_row("m08-private", "result=pass\nno-input=true\n", true);
        assert_eq!(closed.status, "skip");
        assert_eq!(closed.skip_reason.as_deref(), Some("no-input"));
        let refused = lock_exercise_row(
            "m08-documented",
            "result=refuse\ndocumented-path=refused\n",
            true,
        );
        assert_eq!(refused.status, "skip");
        assert_eq!(refused.skip_reason.as_deref(), Some("not-applicable"));
        let skipped = lock_exercise_row("m08-private", "", false);
        assert_eq!(skipped.skip_reason.as_deref(), Some("ci"));
    }

    #[test]
    fn smoke_passes_only_when_the_x86_64_slice_passes() {
        assert_eq!(
            smoke_row("native-intel=false\n", 1, "arm64")
                .skip_reason
                .as_deref(),
            Some("not-applicable")
        );
        assert_eq!(
            smoke_row("smoke=pass\nfat=true\nnative-intel=true\n", 0, "x86_64").status,
            "pass"
        );
        assert_eq!(
            smoke_row("native-intel=true\nfat=false\nsmoke=pass\n", 0, "x86_64").status,
            "fail"
        );
        assert_eq!(
            smoke_row("smoke=pass\nfat=true\nnative-intel=true\n", 1, "x86_64").status,
            "fail"
        );
        let locked = lock_exercise_row(
            "m08-private",
            "result=pass\nstate=os-locked\nlock-screen-appeared=yes\ninput-reached-desktop-without-login=no\n",
            true,
        );
        assert_eq!(locked.status, "pass");
        let unconfirmed = lock_exercise_row(
            "m08-private",
            "result=pass\nstate=lock-unconfirmed\nlock-screen-appeared=no\ninput-reached-desktop-without-login=unknown\n",
            true,
        );
        assert_eq!(unconfirmed.status, "fail");
        let empty = keep_awake_rows(
            "mode=kit\nassertion-created=true\nassertion-released=true\nresult=pass\napp-nap-verdict=\nlid-on-power=no-input\nlid-external=no-external-display\npower-key=no-input\n",
        );
        assert_eq!(empty[1].status, "fail");
        assert!(restored_from(&empty, false));
        assert!(!restored_from(
            &keep_awake_rows(
                "mode=kit\nassertion-created=true\nassertion-released=false\nresult=fail\napp-nap-verdict=not-claimed\nlid-on-power=no-input\nlid-external=no-input\npower-key=no-input\n"
            ),
            false
        ));
        assert_eq!(scrub_model("MacBookPro16,2"), "MacBookPro16,2");
        assert_eq!(scrub_model("serial number"), "");
    }

    #[test]
    fn a_forced_failure_locks_and_ci_does_not() {
        let plan = fail_closed(true, false, false, false);
        assert!(plan.lock);
        assert_eq!(plan.state, "os-locked");
        assert_eq!(plan.argv, ["--fail-closed"]);
        assert!(plan.argv.iter().all(|arg| !arg.contains("unlock")));
        let ci = fail_closed(true, true, false, false);
        assert!(!ci.lock);
        assert_eq!(ci.state, "never-unlocked");
        let stopped = fail_closed(false, false, true, false);
        assert_eq!(stopped.state, "never-unlocked");
        assert!(!stopped.lock);
        assert_eq!(fail_closed(false, false, true, true).state, "os-locked");
        assert!(!fail_closed(false, false, false, false).lock);
        assert_eq!(suite_exit(&summary_of(&[plan_row("fail")]), false), 1);
        assert_eq!(suite_exit(&summary_of(&[plan_row("skip")]), true), 1);
        assert_eq!(suite_exit(&summary_of(&[plan_row("pass")]), false), 0);
        assert!(over_budget(300_000, 300_000));
        assert_eq!(
            budget_skip("m07-lid-on-power", "Lid", true)
                .skip_reason
                .as_deref(),
            Some("budget")
        );
    }

    fn plan_row(status: &str) -> Row {
        row("x", "x", status, None, false, String::new())
    }

    #[test]
    fn json_matches_the_shared_schema_and_the_summary_line() {
        let tests = vec![
            row(
                "m07-assertions",
                "Assertions",
                "pass",
                None,
                false,
                "ok".into(),
            ),
            row(
                "m07-lid-on-power",
                "Lid",
                "skip",
                Some("ci"),
                true,
                "ci".into(),
            ),
        ];
        let summary = summary_of(&tests);
        let result = SuiteResult {
            schema: SCHEMA.into(),
            suite: "macos".into(),
            started_utc: "0000000000000000".into(),
            finished_utc: "0000000000000001".into(),
            duration_ms: 1000,
            mode: "ci".into(),
            config: Config {
                os: "macOS".into(),
                os_version: "15".into(),
                arch: "x86_64".into(),
                model: "MacBookPro16,2".into(),
                ci: true,
            },
            tests,
            summary,
            fail_closed: true,
            serial_collected: false,
            restored: true,
            interrupted: false,
            state: "never-unlocked".into(),
            build_ms: 2_000,
            test_ms: 1_000,
        };
        let json = result.to_json();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert!(conformance_errors(&value).is_empty(), "{json}");
        let txt = result.to_txt();
        assert!(txt.contains("pass=1 fail=0 skip=1"));
        assert!(txt.contains("build=2s test=1s"));
        assert!(txt.contains(SCHEMA));
        assert!(txt.contains("config:"));
        assert!(!txt.to_ascii_lowercase().contains("serial number"));
    }
}
