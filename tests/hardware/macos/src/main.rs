//! Runs the macOS v1 kits and writes one suite-output folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use macos_hardware_suite::{
    Row, exit_row, keep_awake_rows, lock_exercise_row, lock_read_only_row, scrub_model, smoke_row,
    suite_status, summary_table,
};
use serde::Serialize;

#[derive(Serialize)]
struct SuiteReport {
    suite: String,
    ci: bool,
    elapsed_ms: u128,
    model: String,
    arch: String,
    os_version: String,
    serial_collected: bool,
    result: String,
    tests: Vec<Row>,
}

fn main() {
    let code = run();
    std::process::exit(code);
}

fn run() -> i32 {
    let ci = std::env::args().any(|arg| arg == "--ci");
    let root = std::env::var("SUITE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let started = std::time::Instant::now();
    let stamp = unix_secs().to_string();
    let out = root.join("suite-output").join(&stamp);
    if let Err(error) = fs::create_dir_all(&out) {
        eprintln!("output dir: {error}");
        return 1;
    }
    println!("macOS hardware suite. About 4 minutes after the first build.");
    println!(
        "Automated checks run first. The screen goes black for about 60 seconds during App Nap."
    );
    if ci {
        println!("--ci skips lid, Touch ID, and lock exercises.");
    } else {
        println!(
            "Then: lid on power, external display plus lid, Touch ID tap, unlock with your password. Enter skips."
        );
    }

    let arch = command_out(&["uname", "-m"]);
    let os_version = sw_version();
    let model = scrub_model(&command_out(&["sysctl", "-n", "hw.model"]));
    let mut rows = Vec::new();

    let read_only = spawn(
        &bin(&root, "spikes/macos-lock-kit/target/release/macos-lock-kit"),
        &[
            "--read-only",
            "--output-dir",
            &out.join("m08-read-only").to_string_lossy(),
        ],
        &out.join("m08-read-only.log"),
        false,
    );
    let mut read_text_body = read_text(&out.join("m08-read-only").join("result.txt"));
    if read_text_body.is_empty() {
        read_text_body = read_text(&out.join("m08-read-only.log"));
    }
    let mut read_row = lock_read_only_row(&read_text_body);
    if read_only != 0 {
        read_row.status = "fail".into();
        read_row.detail = format!("exit {read_only}; {}", read_row.detail);
    }
    rows.push(read_row);

    let m03 = spawn(
        &bin(
            &root,
            "spikes/macos-desktop-proof/target/release/macos-desktop-proof",
        ),
        &["--m03", "--output-dir", &out.join("m03").to_string_lossy()],
        &out.join("m03.log"),
        false,
    );
    rows.push(exit_row(
        "m03-posted-input",
        m03 == 0,
        false,
        &format!("exit {m03}"),
    ));
    let sharing = spawn(
        &bin(
            &root,
            "spikes/macos-desktop-proof/target/release/macos-desktop-proof",
        ),
        &[
            "--sharing-only",
            "--output-dir",
            &out.join("m04").to_string_lossy(),
        ],
        &out.join("m04.log"),
        false,
    );
    rows.push(exit_row(
        "m04-sharing",
        sharing == 0,
        false,
        &format!("exit {sharing}"),
    ));
    let _smoke = spawn(
        &bin(
            &root,
            "spikes/macos-desktop-proof/target/release/macos-desktop-proof",
        ),
        &[
            "--smoke",
            "--output-dir",
            &out.join("m11").to_string_lossy(),
        ],
        &out.join("m11.log"),
        false,
    );
    rows.push(smoke_row(&read_text(&out.join("m11.log")), arch.trim()));

    let keep = if ci {
        spawn(
            &bin(
                &root,
                "spikes/macos-keep-awake/target/release/macos-keep-awake",
            ),
            &[
                "--runner",
                "--output-dir",
                &out.join("m07").to_string_lossy(),
            ],
            &out.join("m07.log"),
            false,
        )
    } else {
        spawn(
            &bin(
                &root,
                "spikes/macos-keep-awake/target/release/macos-keep-awake",
            ),
            &[
                "--kit",
                "--down-ms",
                "8000",
                "--output-dir",
                &out.join("m07").to_string_lossy(),
            ],
            &out.join("m07.log"),
            true,
        )
    };
    rows.extend(keep_awake_rows(&read_text(
        &out.join("m07").join("result.txt"),
    )));
    if keep != 0 && rows.iter().all(|row| row.status != "fail") {
        rows.push(exit_row(
            "m07-process",
            false,
            false,
            &format!("exit {keep}"),
        ));
    }

    if ci {
        rows.push(lock_exercise_row("m08-private", "", false));
        rows.push(lock_exercise_row("m08-documented", "", false));
    } else {
        println!("The Mac will lock. Unlock with your password.");
        let private_code = spawn(
            &bin(&root, "spikes/macos-lock-kit/target/release/macos-lock-kit"),
            &[
                "--exercise",
                "private",
                "--i-will-unlock",
                "--auto-observe",
                "--output-dir",
                &out.join("m08-private").to_string_lossy(),
            ],
            &out.join("m08-private.log"),
            true,
        );
        let private_body = read_text(&out.join("m08-private").join("result.txt"));
        let mut private_row = lock_exercise_row("m08-private", &private_body, true);
        if private_code != 0 && private_row.status == "pass" {
            private_row.status = "fail".into();
        }
        let leave = private_row.status == "fail" || private_row.status == "no-input";
        rows.push(private_row);
        if leave {
            rows.push(Row {
                id: "m08-documented".into(),
                status: "skip".into(),
                physical: true,
                detail: "skipped so a failed lock stays locked".into(),
            });
        } else {
            println!("The Mac may lock again. Unlock with your password.");
            let documented_code = spawn(
                &bin(&root, "spikes/macos-lock-kit/target/release/macos-lock-kit"),
                &[
                    "--exercise",
                    "documented",
                    "--i-will-unlock",
                    "--auto-observe",
                    "--output-dir",
                    &out.join("m08-documented").to_string_lossy(),
                ],
                &out.join("m08-documented.log"),
                true,
            );
            let documented_body = read_text(&out.join("m08-documented").join("result.txt"));
            let mut documented_row = lock_exercise_row("m08-documented", &documented_body, true);
            if documented_code != 0 && documented_row.status == "pass" {
                documented_row.status = "fail".into();
            }
            rows.push(documented_row);
        }
    }

    let result = suite_status(&rows);
    let report = SuiteReport {
        suite: "macos-hardware-v1".into(),
        ci,
        elapsed_ms: started.elapsed().as_millis(),
        model,
        arch: arch.trim().into(),
        os_version,
        serial_collected: false,
        result: result.into(),
        tests: rows.clone(),
    };
    let json = serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into());
    let mut text = format!(
        "suite=macos-hardware-v1\nci={ci}\nmodel={}\narch={}\nos={}\nelapsed-ms={}\nresult={result}\n",
        report.model, report.arch, report.os_version, report.elapsed_ms
    );
    text.push_str(&summary_table(&rows));
    for row in &rows {
        text.push_str(&format!("{}={} {}\n", row.id, row.status, row.detail));
    }
    let _ = fs::write(out.join("result.json"), json);
    let _ = fs::write(out.join("result.txt"), &text);
    print!("{text}");
    println!("Wrote {}", out.display());
    println!("Send result.json and result.txt from that folder.");
    if result == "fail" { 1 } else { 0 }
}

fn bin(root: &Path, rel: &str) -> PathBuf {
    root.join(rel)
}

fn spawn(program: &Path, args: &[&str], log: &Path, inherit: bool) -> i32 {
    let mut command = Command::new(program);
    command.args(args);
    if inherit {
        match command.status() {
            Ok(status) => status.code().unwrap_or(1),
            Err(error) => {
                let _ = fs::write(log, error.to_string());
                127
            }
        }
    } else {
        match command.output() {
            Ok(output) => {
                let mut body = output.stdout;
                body.extend_from_slice(&output.stderr);
                let _ = fs::write(log, body);
                output.status.code().unwrap_or(1)
            }
            Err(error) => {
                let _ = fs::write(log, error.to_string());
                127
            }
        }
    }
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn command_out(argv: &[&str]) -> String {
    let mut command = Command::new(argv[0]);
    if argv.len() > 1 {
        command.args(&argv[1..]);
    }
    command
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default()
}

fn sw_version() -> String {
    for line in command_out(&["sw_vers"]).lines() {
        if let Some(value) = line.strip_prefix("ProductVersion:") {
            return value.trim().to_string();
        }
    }
    String::new()
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}
