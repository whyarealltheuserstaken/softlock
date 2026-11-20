//! Overlay kit orchestrator. Any stop path closes the covering windows.

use std::fs;
use std::io::{self, BufRead, ErrorKind};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::args::{Args, Mode};
use crate::claim::SHARING_APIS;
use crate::mac::ffi::{self, OverlayGuard, TargetWindow};
use crate::report::{
    CaptureSample, ClickFocus, KIT_ID, KitReport, OperatorRow, PresentationRow,
    claims_from_samples, exit_nonzero, operator_row, render, runner_operator_row,
};
use crate::scrub::{contains_serial, scrub};
use crate::version::parse_sw_vers;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_stop(_signal: i32) {
    STOP.store(true, Ordering::Relaxed);
}

fn stop_requested() -> bool {
    STOP.load(Ordering::Relaxed)
}

pub fn dispatch(args: Args) -> i32 {
    let output_dir = args.output_dir.clone();
    match catch_unwind(AssertUnwindSafe(|| dispatch_inner(&args))) {
        Ok(code) => {
            ffi::OverlayGuard::close();
            ffi::presentation_clear();
            ffi::TargetWindow::close();
            code
        }
        Err(_) => {
            ffi::OverlayGuard::close();
            ffi::presentation_clear();
            ffi::TargetWindow::close();
            let _ = write_crash_report(&output_dir, "probe panicked");
            1
        }
    }
}

fn dispatch_inner(args: &Args) -> i32 {
    install_stop_handler();
    if let Err(error) = fs::create_dir_all(&args.output_dir) {
        eprintln!("output dir: {error}");
        return 1;
    }
    let kit = args.mode == Mode::Kit;
    crate::report::write_stdout(crate::report::instructions(kit));
    let mut version = read_version();
    version.hw_model = scrub(&ffi::hw_model());
    let mut overlay = OverlayGuard::open();
    let mut samples = capture_phase("default", &args.output_dir);
    overlay.set_none();
    ffi::control_open();
    OverlayGuard::pump(400);
    samples.extend(capture_phase("none", &args.output_dir));
    OverlayGuard::order_out();
    OverlayGuard::pump(300);
    samples.extend(capture_phase("baseline", &args.output_dir));
    overlay = OverlayGuard::open();
    overlay.set_none();
    let _target = TargetWindow::open();
    let mut click_focus = TargetWindow::posted_click_focus();
    let bits = ffi::presentation_apply();
    let mut presentation = presentation_rows(&bits, kit);
    let mut no_input = false;
    let mut interrupted = stop_requested() || OverlayGuard::escape_seen();
    let (spaces, fullscreen, hotplug) = if interrupted {
        skipped_rows("interrupted")
    } else if kit {
        fill_click_from_args(args, &mut click_focus);
        fill_presentation_from_args(args, &mut presentation);
        let rows = kit_rows(args, &mut overlay);
        no_input = row_no_input(&rows.0) || row_no_input(&rows.1) || row_no_input(&rows.2);
        rows
    } else {
        (
            runner_operator_row("a Space switch"),
            runner_operator_row("a full-screen app"),
            runner_operator_row("an external display"),
        )
    };
    interrupted = interrupted || stop_requested() || OverlayGuard::escape_seen();
    if interrupted {
        no_input = false;
    }
    let claims = claims_from_samples(&samples, overlay.window.window_area);
    let serial_collected = contains_serial(&version.hw_model);
    OverlayGuard::close();
    ffi::presentation_clear();
    TargetWindow::close();
    let report = KitReport {
        kit: KIT_ID.to_string(),
        mode: if kit { "kit" } else { "runner" }.to_string(),
        version,
        overlay: overlay.window.clone(),
        samples,
        claims,
        click_focus,
        presentation,
        spaces,
        fullscreen,
        hotplug,
        dismissible: true,
        overlay_torn_down: true,
        serial_collected,
        locked_session: false,
        interrupted,
        no_input,
        probe_crash: None,
    };
    let text = render(&report);
    crate::report::write_stdout(&text);
    if let Err(error) = write_outputs(&args.output_dir, &report, &text) {
        eprintln!("{error}");
        return 1;
    }
    crate::report::write_stdout(&format!("Wrote {}", args.output_dir.display()));
    crate::report::write_stdout("Send result.json and result.txt from that folder.");
    if exit_nonzero(&report) { 1 } else { 0 }
}

fn capture_phase(sharing: &str, _output_dir: &Path) -> Vec<CaptureSample> {
    SHARING_APIS
        .iter()
        .map(|api| {
            let mut sample = ffi::capture(api);
            sample.sharing = sharing.to_string();
            sample
        })
        .collect()
}

fn presentation_rows(bits: &ffi::PresentationBits, kit: bool) -> Vec<PresentationRow> {
    let outcome = if kit {
        String::new()
    } else {
        "not-run-on-runner".to_string()
    };
    vec![
        PresentationRow {
            name: "command-tab".to_string(),
            option_bit: "disableProcessSwitching".to_string(),
            applied_while_inactive: bits.process_switching && !bits.app_active,
            app_was_active: bits.app_active,
            outcome: outcome.clone(),
            note: String::new(),
        },
        PresentationRow {
            name: "force-quit".to_string(),
            option_bit: "disableForceQuit".to_string(),
            applied_while_inactive: bits.force_quit && !bits.app_active,
            app_was_active: bits.app_active,
            outcome: outcome.clone(),
            note: String::new(),
        },
        PresentationRow {
            name: "power-key".to_string(),
            option_bit: "disableSessionTermination".to_string(),
            applied_while_inactive: bits.session_termination && !bits.app_active,
            app_was_active: bits.app_active,
            outcome,
            note: String::new(),
        },
    ]
}

fn kit_rows(args: &Args, overlay: &mut OverlayGuard) -> (OperatorRow, OperatorRow, OperatorRow) {
    let spaces = prompt_row(
        args.spaces.as_deref(),
        "Switch to another Space, look at every display, then type stayed-up, covered-by-space, miss, or skip.",
        &["stayed-up", "covered-by-space", "miss", "skip"],
        "spaces",
    );
    overlay.sync();
    let fullscreen = prompt_row(
        args.fullscreen.as_deref(),
        "Open one full-screen app, look at the overlay, then type stayed-up, covered-by-fullscreen, miss, or skip.",
        &["stayed-up", "covered-by-fullscreen", "miss", "skip"],
        "fullscreen",
    );
    overlay.sync();
    let hotplug = prompt_row(
        args.hotplug.as_deref(),
        "Plug in an external display if you have one, wait for a covering window, then type covered, miss, no-external-display, or skip.",
        &["covered", "miss", "no-external-display", "skip"],
        "hotplug",
    );
    overlay.sync();
    (spaces, fullscreen, hotplug)
}

fn fill_presentation_from_args(args: &Args, rows: &mut [PresentationRow]) {
    let values = [
        args.command_tab.as_deref(),
        args.force_quit.as_deref(),
        args.power_key.as_deref(),
    ];
    let prompts = [
        "Press Command-Tab. Type swallowed, presentation-option-worked, desktop-reachable, or skip.",
        "Press Command-Option-Escape. Type swallowed, presentation-option-worked, desktop-reachable, or skip.",
        "Tap the power key once (Touch ID button). Type swallowed, presentation-option-worked, desktop-reachable, or skip.",
    ];
    for (index, row) in rows.iter_mut().enumerate() {
        if stop_requested() {
            row.outcome = "interrupted".to_string();
            continue;
        }
        if let Some(value) = values[index] {
            row.outcome = value.to_string();
            continue;
        }
        let answer = prompt_value(
            prompts[index],
            &[
                "swallowed",
                "presentation-option-worked",
                "desktop-reachable",
                "skip",
            ],
        );
        row.outcome = answer;
    }
}

fn fill_click_from_args(args: &Args, click: &mut ClickFocus) {
    if let Some(value) = args.click.as_deref() {
        click.click_outcome = value.to_string();
    } else {
        let answer = prompt_value(
            "Click the window under the overlay. Type fell-through, blocked, or skip.",
            &["fell-through", "blocked", "skip"],
        );
        if answer != "no-input" && answer != "interrupted" {
            click.click_outcome = answer;
        }
    }
    if let Some(value) = args.focus.as_deref() {
        click.focus_outcome = value.to_string();
    } else {
        let answer = prompt_value(
            "Did keyboard focus stay on the window under the overlay? Type held, moved, or skip.",
            &["held", "moved", "skip"],
        );
        if answer != "no-input" && answer != "interrupted" {
            click.focus_outcome = answer;
        }
    }
}

fn prompt_row(
    preset: Option<&str>,
    instruction: &str,
    allowed: &[&str],
    label: &str,
) -> OperatorRow {
    if stop_requested() {
        return operator_row("interrupted", Some("interrupted"), "interrupted");
    }
    if let Some(value) = preset {
        return operator_row("recorded", Some(value), "preset");
    }
    let answer = prompt_value(instruction, allowed);
    match answer.as_str() {
        "no-input" => operator_row("no-input", Some("no-input"), "stdin closed"),
        "interrupted" => operator_row("interrupted", Some("interrupted"), "interrupted"),
        "skip" => operator_row("skip", Some("skip"), label),
        other => operator_row("recorded", Some(other), "operator"),
    }
}

fn prompt_value(instruction: &str, allowed: &[&str]) -> String {
    if stop_requested() {
        return "interrupted".to_string();
    }
    if !stdin_is_terminal() {
        return "no-input".to_string();
    }
    drain_stdin();
    crate::report::write_stdout(instruction);
    loop {
        if stop_requested() {
            return "interrupted".to_string();
        }
        match read_line() {
            LineRead::Eof => return "no-input".to_string(),
            LineRead::Interrupted => return "interrupted".to_string(),
            LineRead::Line(line) => {
                let value = line.trim().to_ascii_lowercase();
                if value.is_empty() {
                    crate::report::write_stdout(instruction);
                    continue;
                }
                if allowed.contains(&value.as_str()) {
                    return value;
                }
                crate::report::write_stdout(&format!("Type one of: {}", allowed.join(", ")));
            }
        }
    }
}

enum LineRead {
    Line(String),
    Eof,
    Interrupted,
}

fn read_line() -> LineRead {
    let mut line = String::new();
    loop {
        if stop_requested() {
            return LineRead::Interrupted;
        }
        match io::stdin().lock().read_line(&mut line) {
            Ok(0) => return LineRead::Eof,
            Ok(_) => return LineRead::Line(line),
            Err(error) if error.kind() == ErrorKind::Interrupted => {
                if stop_requested() {
                    return LineRead::Interrupted;
                }
                line.clear();
            }
            Err(_) => return LineRead::Eof,
        }
    }
}

fn drain_stdin() {
    unsafe {
        libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
    }
}

fn stdin_is_terminal() -> bool {
    std::io::IsTerminal::is_terminal(&io::stdin())
}

fn skipped_rows(status: &str) -> (OperatorRow, OperatorRow, OperatorRow) {
    let row = operator_row(status, Some(status), status);
    (row.clone(), row.clone(), row)
}

fn row_no_input(row: &OperatorRow) -> bool {
    row.status == "no-input" || row.outcome.as_deref() == Some("no-input")
}

fn read_version() -> crate::report::VersionReport {
    let sw = command_out(&["sw_vers"]);
    let uname = command_out(&["uname", "-m"]);
    parse_sw_vers(&sw, &uname)
}

fn command_out(argv: &[&str]) -> String {
    let mut command = Command::new(argv[0]);
    if argv.len() > 1 {
        command.args(&argv[1..]);
    }
    command
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .unwrap_or_default()
}

fn write_outputs(dir: &Path, report: &KitReport, text: &str) -> Result<(), String> {
    write_json(&dir.join("result.json"), report)?;
    if contains_serial(text) {
        return Err("refusing to write a serial number".to_string());
    }
    fs::write(dir.join("result.txt"), text).map_err(|error| error.to_string())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let body = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    if contains_serial(&body) {
        return Err("refusing to write a serial number".to_string());
    }
    fs::write(path, body + "\n").map_err(|error| error.to_string())
}

fn write_crash_report(dir: &Path, message: &str) -> Result<(), String> {
    let _ = fs::create_dir_all(dir);
    let payload = serde_json::json!({
        "kit": KIT_ID,
        "probe_crash": message,
        "interrupted": false,
        "no_input": false,
        "serial_collected": false,
        "locked_session": false,
        "overlay_torn_down": true,
        "dismissible": true,
    });
    fs::write(
        dir.join("result.json"),
        serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        dir.join("result.txt"),
        format!("probe-crash={message}\noverlay-torn-down=true\nresult=fail\n"),
    )
    .map_err(|error| error.to_string())
}

fn install_stop_handler() {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handle_stop as *const () as usize;
        action.sa_flags = 0;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGHUP, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
    }
}
