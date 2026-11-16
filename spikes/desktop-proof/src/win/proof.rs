use std::fs;
use std::path::Path;

use crate::report::{self, InteractiveReport, ProofReport};

use super::args::Args;
use super::capture;
use super::diag;
use super::hook;
use super::service;
use super::util::{self, log_line, read_text, write_json};
use super::window;

pub fn dispatch(args: Args) -> i32 {
    if args.service {
        service::run_as_service()
    } else if args.interactive {
        run_interactive(&args)
    } else {
        run_orchestrator(&args)
    }
}

fn run_interactive(args: &Args) -> i32 {
    let log = args.output_dir.join("interactive.log");
    log_line(&log, "interactive proof starting");
    window::per_monitor_dpi();
    let identity = diag::identity();
    let screen_w = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN,
        )
    };
    let screen_h = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN,
        )
    };
    log_line(
        &log,
        &format!(
            "session={} station={} desktop={} user={} screen={screen_w}x{screen_h}",
            identity.session_id, identity.window_station, identity.desktop, identity.username
        ),
    );
    let hook_report = hook::prove();
    log_line(
        &log,
        &format!(
            "hook installed={} key={} click={} inserted={} error={}",
            hook_report.installed,
            hook_report.key_injected,
            hook_report.click_injected,
            hook_report.inserted,
            hook_report.error.as_deref().unwrap_or("none")
        ),
    );
    let mut shots = Vec::new();
    let mut topmost = false;
    let mut topmost_hit = false;
    let mut adjacent_hit = false;
    let mut topmost_above = "none".to_string();
    let mut error = None;
    match window::show_markers() {
        Ok(markers) => {
            window::pump(200);
            window::nudge(&markers);
            let status = window::topmost_status(&markers);
            topmost = status.style;
            topmost_hit = status.hit;
            adjacent_hit = status.adjacent_hit;
            topmost_above = status.above;
            log_line(
                &log,
                &format!(
                    "topmost={topmost} topmost-hit={topmost_hit} adjacent-hit={adjacent_hit} topmost-above={topmost_above}"
                ),
            );
            if let Err(failure) = window::set_excluded(&markers, false) {
                error = Some(failure);
            }
            window::pump(100);
            shots.extend(capture::capture_all(&markers, &args.output_dir, false));
            if let Err(failure) = window::set_excluded(&markers, true) {
                error = Some(match error {
                    Some(existing) => format!("{existing}; {failure}"),
                    None => failure,
                });
            }
            window::nudge(&markers);
            window::pump(150);
            shots.extend(capture::capture_all(&markers, &args.output_dir, true));
            window::destroy(&markers);
        }
        Err(failure) => {
            error = Some(failure);
        }
    }
    for shot in &shots {
        log_line(
            &log,
            &format!(
                "{} exclusion={} verdict={} marker={} control={} marker-region={} adjacent-control={} under-marker-control={} error={}",
                shot.method,
                shot.exclusion,
                shot.verdict,
                shot.marker_pixels,
                shot.control_pixels,
                shot.marker_region_pixels,
                shot.adjacent_control_pixels,
                shot.marker_region_control_pixels,
                shot.error.as_deref().unwrap_or("none")
            ),
        );
    }
    let report = InteractiveReport {
        runner_image: args.runner_image.clone(),
        session_id: identity.session_id,
        pid: identity.pid,
        window_station: identity.window_station,
        desktop: identity.desktop,
        username: identity.username,
        os_product: identity.os_product,
        os_build: identity.os_build,
        installation_type: identity.installation_type,
        screen_w,
        screen_h,
        hook: hook_report,
        topmost,
        topmost_hit,
        adjacent_hit,
        topmost_above,
        shots,
        error,
    };
    let path = args.output_dir.join("interactive.json");
    if let Err(failure) = write_json(&path, &report) {
        log_line(&log, &format!("write report: {failure}"));
        eprintln!("{failure}");
        return 1;
    }
    log_line(&log, "interactive proof wrote its report");
    0
}

fn run_orchestrator(args: &Args) -> i32 {
    if let Err(error) = crate::guard::service_install_allowed(
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
        args.allow_local_service,
    ) {
        eprintln!("{error}");
        return 2;
    }
    let _ = fs::create_dir_all(&args.output_dir);
    clear_previous(&args.output_dir);
    let exercise = service::exercise(args);
    if let Some(error) = exercise.error.as_ref() {
        eprintln!("orchestrator-error={error}");
    }
    let service_report = read_json(&args.output_dir.join("service.json"));
    let interactive = read_json(&args.output_dir.join("interactive.json"));
    let report = ProofReport {
        runner_image: args.runner_image.clone(),
        service_registered: exercise.registered,
        service_started: exercise.started,
        service_stopped: exercise.stopped,
        service_deleted: exercise.deleted,
        service: service_report,
        interactive,
    };
    let rendered = report::render(&report);
    println!("{rendered}");
    let report_path = args.output_dir.join("proof-report.json");
    if let Err(error) = write_json(&report_path, &report) {
        eprintln!("proof report: {error}");
    }
    let reasons = report::failure_reasons(&report);
    if !reasons.is_empty() {
        print_log(&args.output_dir.join("service.log"));
        print_log(&args.output_dir.join("interactive.log"));
        1
    } else {
        0
    }
}

fn clear_previous(output: &Path) {
    for name in [
        "service.json",
        "interactive.json",
        "proof-report.json",
        "service.log",
        "interactive.log",
    ] {
        let _ = fs::remove_file(output.join(name));
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = read_text(path)?;
    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        Err(error) => {
            eprintln!("read {}: {error}", path.display());
            None
        }
    }
}

fn print_log(path: &Path) {
    let Some(text) = util::read_text(path) else {
        return;
    };
    eprintln!("----- {} -----", path.display());
    eprintln!("{text}");
}
