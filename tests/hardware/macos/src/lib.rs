//! Parse kit logs into one suite result. The runner lives in `main.rs`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub id: String,
    pub status: String,
    pub physical: bool,
    pub detail: String,
}

pub fn field(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    text.lines()
        .find_map(|line| line.trim().strip_prefix(&prefix).map(str::to_string))
}

pub fn keep_awake_rows(text: &str) -> Vec<Row> {
    let mode = field(text, "mode").unwrap_or_default();
    let result = field(text, "result").unwrap_or_else(|| "missing".to_string());
    let released = field(text, "assertion-released").as_deref() == Some("true");
    let created = field(text, "assertion-created").as_deref() == Some("true");
    let assertions = if result == "fail" || !created || !released {
        "fail"
    } else {
        "pass"
    };
    let mut rows = vec![Row {
        id: "m07-assertions".into(),
        status: assertions.into(),
        physical: false,
        detail: format!("result={result} created={created} released={released}"),
    }];
    if mode == "runner" {
        rows.push(Row {
            id: "m07-app-nap".into(),
            status: "skip".into(),
            physical: false,
            detail: "not-run-on-runner".into(),
        });
    } else {
        let verdict = field(text, "app-nap-verdict").unwrap_or_else(|| "missing".into());
        let status = match verdict.as_str() {
            "napped" | "not-napped" => "pass",
            "not-claimed" | "" | "missing" => "incomplete",
            _ => "fail",
        };
        rows.push(Row {
            id: "m07-app-nap".into(),
            status: status.into(),
            physical: false,
            detail: format!(
                "verdict={verdict} max-gap-ms={} threshold-ms={}",
                field(text, "app-nap-max-gap-ms").unwrap_or_else(|| "missing".into()),
                field(text, "app-nap-threshold-ms").unwrap_or_else(|| "missing".into())
            ),
        });
    }
    for (id, key) in [
        ("m07-lid-on-power", "lid-on-power"),
        ("m07-lid-external", "lid-external"),
        ("m07-power-key", "power-key"),
    ] {
        rows.push(hardware_row(id, key, text, &mode));
    }
    rows
}

fn hardware_row(id: &str, key: &str, text: &str, mode: &str) -> Row {
    let value = field(text, key).unwrap_or_else(|| "missing".into());
    let method = field(text, &format!("{key}-detection")).unwrap_or_default();
    let status = if mode == "runner" || value == "not-run-on-runner" {
        "skip"
    } else if matches!(
        value.as_str(),
        "slept" | "awake" | "no-external-display" | "restart-panel" | "nothing"
    ) {
        "pass"
    } else if value == "skip" {
        "skip"
    } else if value == "no-input" {
        "no-input"
    } else {
        "incomplete"
    };
    Row {
        id: id.into(),
        status: status.into(),
        physical: mode != "runner",
        detail: format!("{value} method={method}"),
    }
}

pub fn lock_read_only_row(text: &str) -> Row {
    let result = field(text, "result").unwrap_or_else(|| "missing".into());
    Row {
        id: "m08-read-only".into(),
        status: if result == "pass" { "pass" } else { "fail" }.into(),
        physical: false,
        detail: format!(
            "result={result} filevault={} session-lock-check={}",
            field(text, "filevault").unwrap_or_else(|| "missing".into()),
            field(text, "session-lock-check").unwrap_or_else(|| "missing".into())
        ),
    }
}

pub fn lock_exercise_row(id: &str, text: &str, ran: bool) -> Row {
    if !ran {
        return Row {
            id: id.into(),
            status: "skip".into(),
            physical: true,
            detail: "ci skips the lock exercise".into(),
        };
    }
    if text.contains("no-input") {
        return Row {
            id: id.into(),
            status: "no-input".into(),
            physical: true,
            detail: "stdin closed".into(),
        };
    }
    let result = field(text, "result").unwrap_or_else(|| "missing".into());
    let documented = field(text, "documented-path").unwrap_or_default();
    let status = if documented == "refused" || result == "refuse" {
        "skip"
    } else if result == "fail" || result == "missing" {
        "fail"
    } else {
        "pass"
    };
    Row {
        id: id.into(),
        status: status.into(),
        physical: true,
        detail: format!(
            "result={result} lock-screen={} input-without-login={}",
            field(text, "lock-screen-appeared").unwrap_or_else(|| "missing".into()),
            field(text, "input-reached-desktop-without-login").unwrap_or_else(|| "missing".into())
        ),
    }
}

pub fn smoke_row(text: &str, arch: &str) -> Row {
    if arch != "x86_64" {
        return Row {
            id: "m11-smoke".into(),
            status: "skip".into(),
            physical: false,
            detail: format!("host arch is {arch}; native Intel smoke is the Intel runner"),
        };
    }
    let native = field(text, "native-intel").as_deref() == Some("true");
    Row {
        id: "m11-smoke".into(),
        status: if native { "pass" } else { "fail" }.into(),
        physical: false,
        detail: format!(
            "native-intel={} fat={}",
            field(text, "native-intel").unwrap_or_else(|| "missing".into()),
            field(text, "fat").unwrap_or_else(|| "missing".into())
        ),
    }
}

pub fn exit_row(id: &str, ok: bool, physical: bool, detail: &str) -> Row {
    Row {
        id: id.into(),
        status: if ok { "pass" } else { "fail" }.into(),
        physical,
        detail: detail.into(),
    }
}

pub fn suite_status(rows: &[Row]) -> &'static str {
    if rows.iter().any(|row| row.status == "fail") {
        "fail"
    } else if rows.iter().any(|row| row.status == "no-input") {
        "no-input"
    } else if rows.iter().any(|row| row.status == "incomplete") {
        "incomplete"
    } else {
        "pass"
    }
}

pub fn summary_table(rows: &[Row]) -> String {
    let mut out = String::from("id                         status     physical\n");
    for row in rows {
        out.push_str(&format!(
            "{:<26} {:<10} {}\n",
            row.id,
            row.status,
            if row.physical { "yes" } else { "no" }
        ));
    }
    out
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
        assert_eq!(rows[1].id, "m07-app-nap");
        assert_eq!(rows[1].status, "skip");
        assert!(rows[2].status == "skip" && !rows.iter().any(|row| row.status == "fail"));
    }

    #[test]
    fn kit_app_nap_and_lid_rows_keep_the_verdict() {
        let text = "\
mode=kit
assertion-created=true
assertion-released=true
result=pass
app-nap-verdict=napped
app-nap-max-gap-ms=1400
app-nap-threshold-ms=1000
lid-on-power=slept
lid-on-power-detection=pmset-log
lid-external=skip
lid-external-detection=operator-skip
power-key=nothing
power-key-detection=iokit-power-notification
";
        let rows = keep_awake_rows(text);
        assert_eq!(rows[1].status, "pass");
        assert!(rows[1].detail.contains("1400"));
        assert_eq!(rows[2].status, "pass");
        assert!(rows[2].detail.contains("pmset-log"));
        assert_eq!(rows[3].status, "skip");
        assert_eq!(rows[4].status, "pass");
    }

    #[test]
    fn closed_stdin_is_no_input_and_a_refused_lock_is_a_skip() {
        let closed = lock_exercise_row("m08-private", "result=pass\nno-input=true\n", true);
        assert_eq!(closed.status, "no-input");
        let refused = lock_exercise_row(
            "m08-documented",
            "result=refuse\ndocumented-path=refused\n",
            true,
        );
        assert_eq!(refused.status, "skip");
        let skipped = lock_exercise_row("m08-private", "", false);
        assert_eq!(skipped.status, "skip");
        assert!(skipped.physical);
    }

    #[test]
    fn smoke_passes_only_a_native_intel_host() {
        assert_eq!(smoke_row("native-intel=false\n", "arm64").status, "skip");
        assert_eq!(
            smoke_row("native-intel=true\nfat=false\n", "x86_64").status,
            "pass"
        );
        assert_eq!(smoke_row("native-intel=false\n", "x86_64").status, "fail");
        assert_eq!(scrub_model("MacBookPro16,2"), "MacBookPro16,2");
        assert_eq!(scrub_model("serial number"), "");
    }

    #[test]
    fn suite_status_prefers_fail_then_no_input() {
        let rows = vec![
            Row {
                id: "a".into(),
                status: "pass".into(),
                physical: false,
                detail: String::new(),
            },
            Row {
                id: "b".into(),
                status: "no-input".into(),
                physical: true,
                detail: String::new(),
            },
        ];
        assert_eq!(suite_status(&rows), "no-input");
        let mut failed = rows.clone();
        failed[0].status = "fail".into();
        assert_eq!(suite_status(&failed), "fail");
    }
}
