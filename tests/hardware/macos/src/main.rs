//! Runs the macOS v1 kits and writes one suite-output folder.

use std::fs;
use std::io::{self, BufRead, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use macos_hardware_suite::{
    Config, SCHEMA, SuiteResult, budget_skip, decided_skip, exit_row, fail_closed, keep_awake_rows,
    lock_exercise_row, lock_read_only_row, over_budget, restored_from, scrub_model, smoke_row,
    suite_exit, summary_of,
};

const BUDGET_MS: u64 = 300_000;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

fn main() {
    let code = run();
    std::process::exit(code);
}

fn run() -> i32 {
    install_interrupt();
    let ci = std::env::args().any(|arg| arg == "--ci");
    let root = std::env::var("SUITE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let started_at = SystemTime::now();
    let started = std::time::Instant::now();
    let build_ms = std::env::var("SUITE_BUILD_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let stamp = iso_utc(started_at).replace(':', "");
    let out = root.join("suite-output").join(&stamp);
    if let Err(error) = fs::create_dir_all(&out) {
        eprintln!("output dir: {error}");
        return 1;
    }
    println!("macOS hardware suite. result.txt reports build time and test time.");
    println!(
        "Automated checks run first. The screen goes black for about 60 seconds during App Nap."
    );
    if ci {
        println!("--ci skips lid, Touch ID, and lock exercises.");
    } else {
        println!(
            "Then: lid on power, external display plus lid, Touch ID tap, unlock with your password. Enter after each step. Type skip to skip."
        );
    }

    let arch = command_out(&["uname", "-m"]);
    let os_version = sw_version();
    let model = scrub_model(&command_out(&["sysctl", "-n", "hw.model"]));
    let mut rows = Vec::new();

    let read_only = spawn(
        &release_bin("macos-lock-kit"),
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
    if stop_for_interrupt(&mut rows) {
        return finish(
            rows, ci, started_at, started, build_ms, &out, arch, os_version, model,
        );
    }

    if !access_ready(ci) {
        rows.push(decided_skip(
            "m03-posted-input",
            "Posted input",
            "not-applicable",
            "grant Accessibility and Input Monitoring, then restart Terminal",
            false,
        ));
        rows.push(decided_skip(
            "m04-sharing",
            "Sharing probe",
            "not-applicable",
            "grant Accessibility and Input Monitoring, then restart Terminal",
            false,
        ));
    } else {
        let m03 = spawn(
            &desktop_bin(),
            &["--m03", "--output-dir", &out.join("m03").to_string_lossy()],
            &out.join("m03.log"),
            false,
        );
        rows.push(exit_row(
            "m03-posted-input",
            "Posted input",
            m03 == 0,
            false,
            &format!("exit {m03}"),
        ));
        let sharing = spawn(
            &desktop_bin(),
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
            "Sharing probe",
            sharing == 0,
            false,
            &format!("exit {sharing}"),
        ));
    }
    if stop_for_interrupt(&mut rows) {
        return finish(
            rows, ci, started_at, started, build_ms, &out, arch, os_version, model,
        );
    }
    let smoke_code = run_smoke(&out.join("m11"), &out.join("m11.log"), arch.trim());
    rows.push(smoke_row(
        &read_text(&out.join("m11.log")),
        smoke_code,
        arch.trim(),
    ));
    if stop_for_interrupt(&mut rows) {
        return finish(
            rows, ci, started_at, started, build_ms, &out, arch, os_version, model,
        );
    }

    let past_budget = over_budget(build_ms + started.elapsed().as_millis() as u64, BUDGET_MS);
    if !ci && past_budget {
        for (id, name, physical) in [
            ("m07-assertions", "Idle-sleep assertions", false),
            ("m07-app-nap", "App Nap sample", false),
            ("m07-lid-on-power", "Lid close on power", true),
            (
                "m07-lid-external",
                "Lid close with an external display",
                true,
            ),
            ("m07-power-key", "Touch ID / power key", true),
        ] {
            rows.push(budget_skip(id, name, physical));
        }
    } else {
        let keep_dir = out.join("m07").to_string_lossy().to_string();
        let keep = if ci {
            spawn(
                &release_bin("macos-keep-awake"),
                &["--runner", "--output-dir", &keep_dir],
                &out.join("m07.log"),
                false,
            )
        } else {
            spawn(
                &release_bin("macos-keep-awake"),
                &["--kit", "--down-ms", "8000", "--output-dir", &keep_dir],
                &out.join("m07.log"),
                true,
            )
        };
        rows.extend(keep_awake_rows(&read_text(
            &out.join("m07").join("result.txt"),
        )));
        if keep != 0 && rows.iter().all(|row| row.status != "fail") && !interrupted() {
            rows.push(exit_row(
                "m07-process",
                "Keep-awake process",
                false,
                false,
                &format!("exit {keep}"),
            ));
        }
    }

    let interrupted_now = interrupted();
    if ci {
        rows.push(lock_exercise_row("m08-private", "", false));
        rows.push(lock_exercise_row("m08-documented", "", false));
    } else if interrupted_now {
        rows.push(decided_skip(
            "m08-private",
            "Private lock",
            "interrupted",
            "skipped before the prompt because the run was interrupted",
            true,
        ));
        rows.push(decided_skip(
            "m08-documented",
            "Documented lock",
            "interrupted",
            "skipped before the prompt because the run was interrupted",
            true,
        ));
    } else if over_budget(build_ms + started.elapsed().as_millis() as u64, BUDGET_MS) {
        rows.push(budget_skip("m08-private", "Private lock", true));
        rows.push(budget_skip("m08-documented", "Documented lock", true));
    } else {
        println!("The Mac will lock. Unlock with your password.");
        let private_code = spawn(
            &release_bin("macos-lock-kit"),
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
        let leave =
            private_row.status == "fail" || private_row.skip_reason.as_deref() == Some("no-input");
        rows.push(private_row);
        if leave || interrupted() {
            let reason = if interrupted() {
                "interrupted"
            } else {
                "not-applicable"
            };
            rows.push(decided_skip(
                "m08-documented",
                "Documented lock",
                reason,
                "skipped before the prompt so a failed lock stays locked",
                true,
            ));
        } else {
            println!("The Mac may lock again. Unlock with your password.");
            let documented_code = spawn(
                &release_bin("macos-lock-kit"),
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

    finish(
        rows, ci, started_at, started, build_ms, &out, arch, os_version, model,
    )
}

fn finish(
    rows: Vec<macos_hardware_suite::Row>,
    ci: bool,
    started_at: SystemTime,
    started: std::time::Instant,
    build_ms: u64,
    out: &Path,
    arch: String,
    os_version: String,
    model: String,
) -> i32 {
    let mut rows = rows;
    fill_interrupted(&mut rows);
    let interrupted_run = interrupted();
    let session_locked = rows
        .iter()
        .any(|row| row.id.starts_with("m08-") && row.physical && row.status == "pass");
    let summary = summary_of(&rows);
    let plan = fail_closed(summary.fail > 0, ci, interrupted_run, session_locked);
    let restored = restored_from(&rows, plan.lock);
    if plan.lock {
        let _ = spawn(
            &release_bin("macos-lock-kit"),
            &[
                "--fail-closed",
                "--output-dir",
                &out.join("fail-closed").to_string_lossy(),
            ],
            &out.join("fail-closed.log"),
            false,
        );
    }
    let report = SuiteResult {
        schema: SCHEMA.into(),
        suite: "macos".into(),
        started_utc: iso_utc(started_at),
        finished_utc: iso_utc(SystemTime::now()),
        duration_ms: build_ms + started.elapsed().as_millis() as u64,
        mode: if ci { "ci" } else { "hardware" }.into(),
        config: Config {
            os: "macOS".into(),
            os_version,
            arch: arch.trim().into(),
            model,
            ci,
        },
        tests: rows,
        summary,
        fail_closed: plan.lock,
        serial_collected: false,
        restored,
        interrupted: interrupted_run,
        state: plan.state.into(),
        build_ms,
        test_ms: started.elapsed().as_millis() as u64,
    };
    let json = report.to_json();
    let text = report.to_txt();
    let _ = fs::write(out.join("result.json"), &json);
    let _ = fs::write(out.join("result.txt"), &text);
    print!("{text}");
    println!("Wrote {}", out.display());
    println!("Send result.json and result.txt from that folder.");
    suite_exit(&report.summary, interrupted_run)
}

fn target_dir() -> PathBuf {
    std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/hardware-suite"))
}

fn release_bin(name: &str) -> PathBuf {
    target_dir().join("release").join(name)
}

fn desktop_bin() -> PathBuf {
    std::env::var("SUITE_DESKTOP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| release_bin("macos-desktop-proof"))
}

fn run_smoke(dir: &Path, log: &Path, arch: &str) -> i32 {
    if arch != "x86_64" {
        return 0;
    }
    let fat = std::env::var("SUITE_SMOKE_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| desktop_bin());
    let fat_text = fat.to_string_lossy().to_string();
    let dir_text = dir.to_string_lossy().to_string();
    spawn(
        Path::new("arch"),
        &["-x86_64", &fat_text, "--smoke", "--output-dir", &dir_text],
        log,
        false,
    )
}

fn access_ready(ci: bool) -> bool {
    if access_granted() {
        return true;
    }
    if ci || !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return false;
    }
    println!(
        "Grant Accessibility and Input Monitoring for this terminal in System Settings, then press Enter."
    );
    match read_line_interruptible() {
        LineRead::Line(_) => !interrupted() && access_granted(),
        LineRead::Interrupted => false,
        LineRead::Eof => false,
    }
}

enum LineRead {
    Line(String),
    Eof,
    Interrupted,
}

fn read_line_interruptible() -> LineRead {
    let mut line = String::new();
    loop {
        if interrupted() {
            return LineRead::Interrupted;
        }
        match io::stdin().lock().read_line(&mut line) {
            Ok(0) => return LineRead::Eof,
            Ok(_) => return LineRead::Line(line.trim().to_string()),
            Err(error) if error.kind() == ErrorKind::Interrupted => {
                if interrupted() {
                    return LineRead::Interrupted;
                }
                line.clear();
            }
            Err(_) => return LineRead::Eof,
        }
    }
}

fn access_granted() -> bool {
    #[cfg(target_os = "macos")]
    {
        unsafe { AXIsProcessTrusted() != 0 && CGPreflightListenEventAccess() != 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

fn stop_for_interrupt(_rows: &mut Vec<macos_hardware_suite::Row>) -> bool {
    interrupted()
}

fn fill_interrupted(rows: &mut Vec<macos_hardware_suite::Row>) {
    if !interrupted() {
        return;
    }
    for (id, name, physical) in [
        ("m03-posted-input", "Posted input", false),
        ("m04-sharing", "Sharing probe", false),
        ("m11-smoke", "Native Intel smoke", false),
        ("m07-assertions", "Idle-sleep assertions", false),
        ("m07-app-nap", "App Nap sample", false),
        ("m07-lid-on-power", "Lid close on power", true),
        (
            "m07-lid-external",
            "Lid close with an external display",
            true,
        ),
        ("m07-power-key", "Touch ID / power key", true),
        ("m08-private", "Private lock", true),
        ("m08-documented", "Documented lock", true),
    ] {
        if rows.iter().any(|row| row.id == id) {
            continue;
        }
        rows.push(decided_skip(
            id,
            name,
            "interrupted",
            "stopped before this step",
            physical,
        ));
    }
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

fn iso_utc(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { y + 1 } else { y } as i32;
    (year, month, day)
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> u8;
}

fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

#[cfg(target_os = "macos")]
fn install_interrupt() {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_sigint as *const () as usize;
        action.sa_flags = 0;
        libc::sigemptyset(&mut action.sa_mask);
        let _ = libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
    }
}

#[cfg(target_os = "macos")]
extern "C" fn on_sigint(_: i32) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

#[cfg(not(target_os = "macos"))]
fn install_interrupt() {}
