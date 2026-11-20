//! Overlay kit orchestrator. Any stop path closes the covering windows.

use std::fs;
use std::io::{self, BufRead};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::args::{Args, Mode};
use crate::claim::SHARING_APIS;
use crate::mac::ffi::{self, OverlayGuard, TargetWindow};
use crate::report::{
    CaptureSample, ClickFocus, KIT_ID, KitReport, OperatorRow, PRESENTATION_RECORDED_NOTE,
    PresentationRow, claims_from_samples, exit_nonzero, operator_row, render, row_is_failure,
    runner_operator_row, score_hotplug,
};
use crate::scrub::{contains_serial, scrub};
use crate::version::parse_sw_vers;
use crate::wait::{
    HARD_EXIT_GRACE_MS, POLL_MS, WaitOutcome, hard_exit_due, poll_stdin, spawn_watchdog, wait_slice,
};

static STOP: AtomicBool = AtomicBool::new(false);
static TIMED_OUT: AtomicBool = AtomicBool::new(false);
static TORN_DOWN: AtomicBool = AtomicBool::new(false);
static SIGINT_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static TIMEOUT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(600_000);

extern "C" fn handle_stop(signal: i32) {
    if signal == libc::SIGINT {
        let n = SIGINT_COUNT.fetch_add(1, Ordering::Relaxed);
        if n >= 1 {
            unsafe {
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                libc::raise(libc::SIGINT);
            }
            return;
        }
    }
    STOP.store(true, Ordering::Relaxed);
}

fn stop_requested() -> bool {
    STOP.load(Ordering::Relaxed)
}

pub fn dispatch(args: Args) -> i32 {
    let output_dir = args.output_dir.clone();
    match catch_unwind(AssertUnwindSafe(|| dispatch_inner(&args))) {
        Ok(code) => {
            tear_overlay_down();
            code
        }
        Err(_) => {
            tear_overlay_down();
            let _ = write_crash_report(&output_dir, "probe panicked");
            1
        }
    }
}

fn dispatch_inner(args: &Args) -> i32 {
    install_stop_handler();
    TIMEOUT_MS.store(args.timeout_ms, Ordering::Relaxed);
    spawn_watchdog_onto_stop(args.timeout_ms);
    if let Err(error) = fs::create_dir_all(&args.output_dir) {
        eprintln!("output dir: {error}");
        return 1;
    }
    let kit = args.mode == Mode::Kit;
    crate::report::write_stdout(crate::report::instructions(kit));
    let mut version = read_version();
    version.hw_model = scrub(&ffi::hw_model());
    let screen_recording = ffi::screen_capture_access();
    let screens_before = ffi::display_count();
    let mut overlay = OverlayGuard::open();
    OverlayGuard::set_banner("m06 overlay. Esc needs Accessibility. Ctrl-C or the timeout quits.");
    let _ = OverlayGuard::wait_visible(2000);
    let covered_before = overlay.window.covered_count;
    let mut max_screens = screens_before.max(overlay.window.display_count);
    let mut samples = capture_phase("default", &args.output_dir);
    overlay.set_none();
    ffi::control_open();
    let _ = OverlayGuard::wait_visible(2000);
    samples.extend(capture_phase("none", &args.output_dir));
    OverlayGuard::order_out();
    let _ = OverlayGuard::wait_hidden(2000);
    samples.extend(capture_phase("baseline", &args.output_dir));
    overlay = OverlayGuard::open();
    overlay.set_none();
    OverlayGuard::set_banner("m06 overlay. Esc needs Accessibility. Ctrl-C or the timeout quits.");
    let _ = OverlayGuard::wait_visible(2000);
    let target = TargetWindow::open();
    let mut click_focus = TargetWindow::posted_click_focus();
    click_focus.accessibility = click_focus.accessibility || target.accessibility;
    let accessibility = click_focus.accessibility;
    let mut presentation = presentation_rows(
        &ffi::PresentationBits {
            app_active: false,
            process_switching: false,
            force_quit: false,
            session_termination: false,
        },
        kit,
    );
    let mut no_input = false;
    let mut interrupted = stop_requested() || OverlayGuard::escape_seen();
    let (spaces, fullscreen, hotplug) = if interrupted || timed_out_flag() {
        skipped_rows(if timed_out_flag() {
            "timeout"
        } else {
            "interrupted"
        })
    } else if kit {
        fill_click_from_args(args, &mut click_focus);
        let rows = kit_rows(args, &mut overlay, screens_before, &mut max_screens);
        no_input = row_no_input(&rows.0) || row_no_input(&rows.1) || row_no_input(&rows.2);
        rows
    } else {
        (
            runner_operator_row("a Space switch"),
            runner_operator_row("a full-screen app"),
            runner_operator_row("an external display"),
        )
    };
    overlay.sync();
    max_screens = max_screens
        .max(ffi::display_count())
        .max(overlay.window.display_count);
    let screens_after = ffi::display_count().max(overlay.window.display_count);
    let covered_after = overlay.window.covered_count;
    if kit && !interrupted && !timed_out_flag() {
        fill_presentation_from_args(args, &mut presentation, false);
    }
    tear_overlay_down();
    let _ = OverlayGuard::wait_hidden(2000);
    if kit && !interrupted && !timed_out_flag() {
        fill_power_key_last(args, &mut presentation);
    }
    interrupted = interrupted || stop_requested() || OverlayGuard::escape_seen();
    let timed_out = timed_out_flag();
    if interrupted || timed_out {
        no_input = false;
        interrupted = interrupted && !timed_out;
    }
    let claims = claims_from_samples(&samples, overlay.window.window_area);
    let serial_collected = contains_serial(&version.hw_model);
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
        screens_before,
        screens_after,
        covered_before,
        covered_after,
        max_screens,
        screen_recording,
        accessibility,
        timed_out,
        timeout_ms: args.timeout_ms,
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

fn kit_rows(
    args: &Args,
    overlay: &mut OverlayGuard,
    screens_before: i32,
    max_screens: &mut i32,
) -> (OperatorRow, OperatorRow, OperatorRow) {
    let spaces = score_named_row(prompt_row(
        args.spaces.as_deref(),
        "Switch to another Space, look at every display, then type stayed-up, covered-by-space, miss, or skip.",
        &["stayed-up", "covered-by-space", "miss", "skip"],
        "spaces",
    ));
    overlay.sync();
    *max_screens = (*max_screens)
        .max(ffi::display_count())
        .max(overlay.window.display_count);
    let fullscreen = score_named_row(prompt_row(
        args.fullscreen.as_deref(),
        "Open one full-screen app, look at the overlay, then type stayed-up, covered-by-fullscreen, miss, or skip.",
        &["stayed-up", "covered-by-fullscreen", "miss", "skip"],
        "fullscreen",
    ));
    overlay.sync();
    *max_screens = (*max_screens)
        .max(ffi::display_count())
        .max(overlay.window.display_count);
    let typed_hotplug = prompt_row(
        args.hotplug.as_deref(),
        "Plug in an external display if you have one, wait for a covering window, then type covered, miss, no-external-display, or skip.",
        &["covered", "miss", "no-external-display", "skip"],
        "hotplug",
    );
    overlay.sync();
    *max_screens = (*max_screens)
        .max(ffi::display_count())
        .max(overlay.window.display_count);
    let hotplug = match typed_hotplug.outcome.as_deref() {
        Some("skip" | "no-input" | "interrupted" | "timeout") => typed_hotplug,
        Some(typed) => score_hotplug(
            typed,
            screens_before,
            overlay.window.display_count.max(ffi::display_count()),
            overlay.window.covered_count,
            *max_screens,
        ),
        None => typed_hotplug,
    };
    (spaces, fullscreen, hotplug)
}

fn score_named_row(row: OperatorRow) -> OperatorRow {
    if row_is_failure(row.outcome.as_deref()) {
        operator_row("fail", row.outcome.as_deref(), &row.note)
    } else {
        row
    }
}

fn fill_presentation_from_args(args: &Args, rows: &mut [PresentationRow], power_key: bool) {
    let values = [
        args.command_tab.as_deref(),
        args.force_quit.as_deref(),
        args.power_key.as_deref(),
    ];
    let prompts = [
        "Press Command-Tab. Type swallowed, presentation-option-worked, desktop-reachable, or skip.",
        "Press Command-Option-Escape. Type swallowed, presentation-option-worked, desktop-reachable, or skip.",
        "POWER KEY LAST: overlay is gone. A Touch ID MacBook Pro may lock or sleep. Tap the power key once only if you accept that. Type swallowed, presentation-option-worked, desktop-reachable, or skip.",
    ];
    for (index, row) in rows.iter_mut().enumerate() {
        let is_power = row.name == "power-key";
        if is_power != power_key {
            continue;
        }
        if stop_requested() || timed_out_flag() {
            row.outcome = if timed_out_flag() {
                "timeout".to_string()
            } else {
                "interrupted".to_string()
            };
            continue;
        }
        let apply_options = !is_power;
        if apply_options {
            let bits = ffi::presentation_apply();
            row.applied_while_inactive = match row.name.as_str() {
                "command-tab" => bits.process_switching && !bits.app_active,
                "force-quit" => bits.force_quit && !bits.app_active,
                _ => false,
            };
            row.app_was_active = bits.app_active;
        }
        if let Some(value) = values[index] {
            row.outcome = value.to_string();
            if row.outcome == "desktop-reachable" {
                row.note = PRESENTATION_RECORDED_NOTE.to_string();
            }
            if apply_options {
                ffi::presentation_clear();
            }
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
        if row.outcome == "desktop-reachable" {
            row.note = PRESENTATION_RECORDED_NOTE.to_string();
        }
        if apply_options {
            ffi::presentation_clear();
        }
    }
}

fn fill_power_key_last(args: &Args, rows: &mut [PresentationRow]) {
    fill_presentation_from_args(args, rows, true);
}

fn fill_click_from_args(args: &Args, click: &mut ClickFocus) {
    OverlayGuard::set_banner(&format!(
        "{}\nEsc needs Accessibility. Ctrl-C / timeout quits.",
        "Click the window under the overlay, then type fell-through, blocked, or skip."
    ));
    if let Some(value) = args.click.as_deref() {
        click.typed_click = value.to_string();
    } else {
        let answer = prompt_value(
            "Click the window under the overlay. Type fell-through, blocked, or skip.",
            &["fell-through", "blocked", "skip"],
        );
        if answer != "no-input" && answer != "interrupted" && answer != "timeout" {
            click.typed_click = answer;
        }
    }
    OverlayGuard::set_banner(
        "Did keyboard focus stay on the window under the overlay? Type held, moved, or skip.\nEsc needs Accessibility. Ctrl-C / timeout quits.",
    );
    if let Some(value) = args.focus.as_deref() {
        click.typed_focus = value.to_string();
    } else {
        let answer = prompt_value(
            "Did keyboard focus stay on the window under the overlay? Type held, moved, or skip.",
            &["held", "moved", "skip"],
        );
        if answer != "no-input" && answer != "interrupted" && answer != "timeout" {
            click.typed_focus = answer;
        }
    }
    if !click.typed_click.is_empty() {
        click.click_outcome = click.typed_click.clone();
    }
    if !click.typed_focus.is_empty() {
        click.focus_outcome = click.typed_focus.clone();
    }
}

fn prompt_row(
    preset: Option<&str>,
    instruction: &str,
    allowed: &[&str],
    label: &str,
) -> OperatorRow {
    if timed_out_flag() {
        return operator_row("timeout", Some("timeout"), "timeout");
    }
    if stop_requested() {
        return operator_row("interrupted", Some("interrupted"), "interrupted");
    }
    OverlayGuard::set_banner(&format!(
        "{instruction}\nEsc needs Accessibility. Ctrl-C / timeout quits."
    ));
    if let Some(value) = preset {
        return operator_row("recorded", Some(value), "preset");
    }
    let answer = prompt_value(instruction, allowed);
    match answer.as_str() {
        "no-input" => operator_row("no-input", Some("no-input"), "stdin closed"),
        "interrupted" => operator_row("interrupted", Some("interrupted"), "interrupted"),
        "timeout" => operator_row("timeout", Some("timeout"), "timeout"),
        "skip" => operator_row("skip", Some("skip"), label),
        other => operator_row("recorded", Some(other), "operator"),
    }
}

fn prompt_value(instruction: &str, allowed: &[&str]) -> String {
    if timed_out_flag() {
        return "timeout".to_string();
    }
    if stop_requested() {
        return "interrupted".to_string();
    }
    if !stdin_is_terminal() {
        return "no-input".to_string();
    }
    drain_stdin();
    OverlayGuard::set_banner(&format!(
        "{instruction}\nEsc needs Accessibility. Ctrl-C / timeout quits."
    ));
    crate::report::write_stdout(instruction);
    crate::report::write_stdout("Esc needs Accessibility. Ctrl-C or the timeout quits.");
    loop {
        match wait_for_line() {
            WaitOutcome::Eof => return "no-input".to_string(),
            WaitOutcome::Interrupted => return "interrupted".to_string(),
            WaitOutcome::Timeout => return "timeout".to_string(),
            WaitOutcome::Escape => return "interrupted".to_string(),
            WaitOutcome::Line => {
                let mut line = String::new();
                match io::stdin().lock().read_line(&mut line) {
                    Ok(0) => return "no-input".to_string(),
                    Ok(_) => {
                        let value = line.trim().to_ascii_lowercase();
                        if value.is_empty() {
                            crate::report::write_stdout(instruction);
                            continue;
                        }
                        if allowed.contains(&value.as_str()) {
                            return value;
                        }
                        crate::report::write_stdout(&format!(
                            "Type one of: {}",
                            allowed.join(", ")
                        ));
                    }
                    Err(_) => return "no-input".to_string(),
                }
            }
        }
    }
}

fn wait_for_line() -> WaitOutcome {
    let started = Instant::now();
    let timeout_ms = TIMEOUT_MS.load(Ordering::Relaxed);
    loop {
        OverlayGuard::pump(0);
        let stop = stop_requested();
        let escape = OverlayGuard::escape_seen();
        if TIMED_OUT.load(Ordering::Relaxed) {
            return WaitOutcome::Timeout;
        }
        if let Some(outcome) = wait_slice(started, timeout_ms, stop, escape, poll_stdin(POLL_MS)) {
            if outcome == WaitOutcome::Interrupted && TIMED_OUT.load(Ordering::Relaxed) {
                return WaitOutcome::Timeout;
            }
            return outcome;
        }
    }
}

fn timed_out_flag() -> bool {
    TIMED_OUT.load(Ordering::Relaxed)
}

fn spawn_watchdog_onto_stop(timeout_ms: u64) {
    let stop = Arc::new(AtomicBool::new(false));
    let timed = Arc::new(AtomicBool::new(false));
    spawn_watchdog(timeout_ms, stop.clone(), timed.clone());
    std::thread::spawn(move || {
        let slice = Duration::from_millis(POLL_MS as u64);
        loop {
            if timed.load(Ordering::Relaxed) {
                TIMED_OUT.store(true, Ordering::Relaxed);
                STOP.store(true, Ordering::Relaxed);
                break;
            }
            if STOP.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(slice);
        }
        let grace_deadline = Instant::now() + Duration::from_millis(HARD_EXIT_GRACE_MS);
        while Instant::now() < grace_deadline {
            if TORN_DOWN.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(slice);
        }
        if hard_exit_due(
            TORN_DOWN.load(Ordering::Relaxed),
            TIMED_OUT.load(Ordering::Relaxed),
            true,
        ) {
            unsafe { libc::_exit(3) };
        }
    });
}

fn tear_overlay_down() {
    ffi::OverlayGuard::close();
    ffi::presentation_clear();
    ffi::TargetWindow::close();
    TORN_DOWN.store(true, Ordering::Relaxed);
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
