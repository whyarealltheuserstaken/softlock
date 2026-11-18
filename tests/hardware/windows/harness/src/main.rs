//! Write the suite plan and the result files. The PowerShell runner does the rest.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use serde_json::Value;
use softlock_win_hwsuite::{
    Flags, Outcome, Report, conform, plan_value, profile_name, render, section_contains,
};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let rest = args.collect::<Vec<_>>();
    match command.as_str() {
        "plan" => finish(plan_main(&rest)),
        "report" => finish(report_main(&rest)),
        "requests" => finish(requests_main(&rest)),
        "conform" => finish(conform_main(&rest)),
        _ => {
            eprintln!("usage: softlock-win-hwsuite plan|report|requests|conform");
            ExitCode::from(2)
        }
    }
}

fn finish(result: Result<ExitCode, String>) -> ExitCode {
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn plan_main(args: &[String]) -> Result<ExitCode, String> {
    let facts_path = required(args, "--facts")?;
    let facts = read_json(facts_path)?;
    let flags = flags_from(&facts)?;
    let (machine, rows) = plan_value(&facts, &flags)?;
    let body = serde_json::json!({
        "profile": profile_name(&flags),
        "config": machine.to_json(),
        "rows": rows.iter().map(|row| serde_json::json!({
            "id": row.id,
            "action": row.action.as_str(),
            "reason": row.reason,
            "detail": row.detail,
            "prompts": row.prompts,
            "physical": row.physical,
            "needs_admin": row.needs_admin,
            "blocks_input": row.blocks_input,
        })).collect::<Vec<_>>(),
    });
    println!("{body}");
    Ok(ExitCode::SUCCESS)
}

fn report_main(args: &[String]) -> Result<ExitCode, String> {
    let bundle_path = required(args, "--bundle")?;
    let out_dir = required(args, "--out-dir")?;
    let bundle = read_json(bundle_path)?;
    let facts = bundle
        .get("facts")
        .cloned()
        .ok_or_else(|| "bundle is missing facts".to_string())?;
    let flags = flags_from(&facts)?;
    let elapsed_ms = bundle
        .get("elapsed_ms")
        .and_then(Value::as_u64)
        .ok_or_else(|| "bundle is missing elapsed_ms".to_string())?;
    let supplied = bundle
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (machine, _) = plan_value(&facts, &flags)?;
    let outcome = Outcome {
        started_utc: text_field(&bundle, "started_utc"),
        finished_utc: text_field(&bundle, "finished_utc"),
        interrupted: flag_or(&bundle, "interrupted", false),
        restored: flag_or(&bundle, "restored", true),
        locked: flag_or(&bundle, "locked", false),
        fail_closed: flag_or(&bundle, "fail_closed", false),
        hklm_command: text_field(&bundle, "hklm_command"),
    };
    let report = render(&machine, &flags, elapsed_ms, &supplied, &outcome);
    let body = report.to_json();
    conform(&body)?;
    write_report(Path::new(out_dir), &report, &body)?;
    print!("{}", report.to_text());
    if report.outcome.interrupted || report.result != "pass" {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn requests_main(args: &[String]) -> Result<ExitCode, String> {
    let text_path = required(args, "--text")?;
    let needle = required(args, "--needle")?;
    let text = fs::read_to_string(text_path).map_err(|error| error.to_string())?;
    let body = serde_json::json!({
        "system": section_contains(&text, "SYSTEM", needle),
        "display": section_contains(&text, "DISPLAY", needle),
    });
    println!("{body}");
    Ok(ExitCode::SUCCESS)
}

fn conform_main(args: &[String]) -> Result<ExitCode, String> {
    let path = required(args, "--file")?;
    let value = read_json(path)?;
    conform(&value)?;
    Ok(ExitCode::SUCCESS)
}

fn write_report(dir: &Path, report: &Report, body: &serde_json::Value) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(body).map_err(|error| error.to_string())?;
    fs::write(dir.join("result.json"), json + "\n").map_err(|error| error.to_string())?;
    fs::write(dir.join("result.txt"), report.to_text()).map_err(|error| error.to_string())?;
    Ok(())
}

fn flag_or(value: &Value, name: &str, default: bool) -> bool {
    match value.get(name) {
        Some(Value::Bool(flag)) => *flag,
        _ => default,
    }
}

fn text_field(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn flags_from(facts: &Value) -> Result<Flags, String> {
    Ok(Flags {
        ci: bool_field(facts, "ci")?,
        admin: bool_field(facts, "admin")?,
        elevated: bool_field(facts, "elevated")?,
        github_actions: bool_field(facts, "github_actions")?,
        school: bool_field(facts, "school")?,
    })
}

fn bool_field(facts: &Value, name: &str) -> Result<bool, String> {
    match facts.get(name) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("{name} must be a boolean")),
        None => Ok(false),
    }
}

fn required<'a>(args: &'a [String], flag: &str) -> Result<&'a str, String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
        .ok_or_else(|| format!("missing {flag}"))
}

fn read_json(path: &str) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))
}
