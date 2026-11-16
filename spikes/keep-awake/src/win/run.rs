//! One run: report the machine, hold the power requests, show the overlay, log ticks.

use std::mem::MaybeUninit;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, MSG, SetTimer, TranslateMessage, WM_TIMER,
};
use windows::core::BOOL;

use crate::analyze::summarize_dir_with;
use crate::cli::RunSpec;
use crate::logfile::LogFile;
use crate::logline::format_header;
use crate::plan::{
    format_battery_percent, format_execution_state, format_lid_action, format_yes_no,
    holding_lines, ready_line, refuse_reason, requests_label, spike_power_plan, status_report,
};
use crate::session::RunReport;
use crate::timefmt::{format_unix_millis, unix_now_ms};

use super::overlay::{
    self, Overlay, STOP_CTRL, STOP_KEY, STOP_MESSAGE, STOP_TIMER, mark_loop_finished, request_stop,
    stop_reason,
};
use super::power::{self, PowerHold};
use super::probe::Probe;

pub fn run(spec: &RunSpec) -> Result<RunReport, String> {
    overlay::reset_stop_state();
    let machine = power::read_machine();
    println!("{}", banner());
    println!("{}", status_report(&machine));
    if let Some(reason) = refuse_reason(spec.scenario, &machine, spec.allow_no_modern_standby) {
        return Err(reason);
    }
    if spec.scenario.is_measurement() && spec.allow_no_modern_standby && machine.aoac != Some(true)
    {
        println!(
            "Modern Standby was not reported. Continuing because --allow-no-modern-standby was set."
        );
    }
    if spec.scenario.is_measurement() && spec.duration_secs != 3600 {
        println!("This is a short run, not the 60 minute test.");
    }

    let hold = PowerHold::acquire()?;
    println!("{}", holding_lines(&spike_power_plan()));
    let overlay = Overlay::show(spec.look)?;
    let probe = Probe::start();
    let header = header_text(spec, &machine, probe.mode());
    let mut log = LogFile::create(&spec.log_path(), &header)
        .map_err(|error| format!("Could not write {}: {error}", spec.log_path().display()))?;
    let started = Instant::now();
    let started_ms = unix_now_ms();
    let deadline_ms = started_ms.saturating_add(spec.duration_secs.saturating_mul(1000));
    let _ctrl = install_ctrl_handler();
    let _finish = FinishGuard;
    spawn_watchdog(deadline_ms);
    let mut ticks = 0usize;
    let mut last_tick = Instant::now();
    write_tick(&mut log, &probe, &hold, &mut ticks, started)?;
    println!("Logging to: {}", spec.log_path().display());
    println!("{}", ready_line(spec.scenario, spec.duration_secs));
    let timer = unsafe { SetTimer(Some(overlay.hwnd()), 1, 250, None) };
    if timer == 0 {
        return Err("Could not start the timer, so the run was stopped.".to_string());
    }

    let mut message = unsafe { MaybeUninit::<MSG>::zeroed().assume_init() };
    loop {
        let rc = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if rc.0 == 0 {
            break;
        }
        if rc.0 < 0 {
            let _ = log.end(unix_now_ms(), "window");
            return Err("The window loop failed, so the run was stopped.".to_string());
        }
        if message.message == STOP_MESSAGE {
            break;
        }
        if message.message == WM_TIMER {
            let now = unix_now_ms();
            if now >= deadline_ms {
                request_stop(STOP_TIMER);
            } else if last_tick.elapsed() >= Duration::from_secs(spec.interval_secs) {
                write_tick(&mut log, &probe, &hold, &mut ticks, started)?;
                last_tick = Instant::now();
            }
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    let _ = unsafe { KillTimer(Some(overlay.hwnd()), 1) };
    let reason = stop_token(stop_reason());
    log.end(unix_now_ms(), reason)
        .map_err(|error| format!("Could not finish the log: {error}"))?;
    println!("{}", stop_sentence(reason));
    Ok(RunReport {
        ticks,
        overlay_shown: true,
        power_held: true,
        stop: reason.to_string(),
        log_path: spec.log_path(),
    })
}

pub fn smoke() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!(
        "softlock-spike-keep-awake-smoke-{}",
        std::process::id()
    ));
    let _cleanup = DirCleanup(dir.clone());
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let spec = RunSpec::smoke(dir.clone());
    let report = match run(&spec) {
        Ok(report) => report,
        Err(error) => {
            let raw = std::fs::read_to_string(spec.log_path()).unwrap_or_default();
            return Err(format!("{error}\n{raw}"));
        }
    };
    let summary = summarize_dir_with(&dir, &["smoke"]).map_err(|error| error.to_string())?;
    println!("{}", summary.text);
    if !report.power_held || !report.overlay_shown || report.ticks < 2 || summary.gaps != 0 {
        let raw = std::fs::read_to_string(spec.log_path()).unwrap_or_default();
        return Err(format!(
            "Smoke check failed. ticks={} overlay={} gaps={}\n{}\n{raw}",
            report.ticks, report.overlay_shown, summary.gaps, summary.text
        ));
    }
    Ok(())
}

fn write_tick(
    log: &mut LogFile,
    probe: &Probe,
    hold: &PowerHold,
    ticks: &mut usize,
    started: Instant,
) -> Result<(), String> {
    let at = unix_now_ms();
    let signals = power::read_signals();
    let (kind, ok) = probe.sample();
    hold.refresh();
    log.tick(at, ok, kind, signals)
        .map_err(|error| format!("Could not write a log line: {error}"))?;
    *ticks += 1;
    let battery = signals
        .battery_percent
        .map(|percent| percent.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let cooling = signals
        .cooling
        .map(|mode| match mode {
            crate::signals::Cooling::Active => "active",
            crate::signals::Cooling::Passive => "passive",
            crate::signals::Cooling::Invalid => "invalid",
        })
        .unwrap_or("unknown");
    let throttle = match (signals.speed_limited(), signals.execution_throttled) {
        (Some(true), _) | (_, Some(true)) => "yes",
        (Some(false), Some(false)) => "no",
        _ => "unknown",
    };
    println!(
        "+{} {} battery={battery}% cooling={cooling} throttle={throttle} ui={}",
        format_elapsed(started.elapsed()),
        format_unix_millis(at),
        if ok { "ok" } else { "fail" }
    );
    Ok(())
}

fn header_text(spec: &RunSpec, machine: &crate::plan::MachineView, probe: &str) -> String {
    let aoac = format_yes_no(machine.aoac);
    let lid = format_yes_no(machine.lid_present);
    let battery = format_battery_percent(machine.battery_percent);
    let ac = format_lid_action(machine.lid_action_ac);
    let dc = format_lid_action(machine.lid_action_dc);
    let plan = spike_power_plan();
    let execution = format_execution_state(plan.execution_state);
    let interval = spec.interval_secs.to_string();
    let planned = spec.duration_secs.to_string();
    format_header(&[
        ("scenario", spec.scenario.id()),
        ("aoac", aoac),
        ("lid_present", lid),
        ("power_source", machine.power_source.log_value()),
        ("battery_percent", &battery),
        ("lid_action_ac", &ac),
        ("lid_action_dc", &dc),
        ("requests", requests_label(&plan)),
        ("execution_state", &execution),
        ("overlay", spec.look.label()),
        ("probe", probe),
        ("interval_secs", &interval),
        ("planned_secs", &planned),
    ])
}

fn banner() -> String {
    format!("{} keep-awake spike", softlock_core::PRODUCT_DISPLAY_NAME)
}

fn format_elapsed(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    format!("{:02}:{:02}", total / 60, total % 60)
}

fn stop_token(code: usize) -> &'static str {
    match code {
        STOP_KEY => "key",
        STOP_CTRL => "ctrl-c",
        STOP_TIMER => "timer",
        _ => "stopped",
    }
}

fn stop_sentence(reason: &str) -> &'static str {
    match reason {
        "key" => "Stopped because a key was pressed.",
        "ctrl-c" => "Stopped because Ctrl+C was pressed.",
        "timer" => "The run finished.",
        _ => "The run stopped.",
    }
}

fn spawn_watchdog(deadline_ms: u64) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(200));
            if overlay::loop_finished() {
                return;
            }
            let now = unix_now_ms();
            if now >= deadline_ms {
                request_stop(STOP_TIMER);
            }
            if now >= deadline_ms.saturating_add(10_000) {
                eprintln!("The run did not stop when its time was up.");
                std::process::exit(1);
            }
        }
    });
}

struct FinishGuard;

impl Drop for FinishGuard {
    fn drop(&mut self) {
        mark_loop_finished();
    }
}

struct CtrlGuard;

impl Drop for CtrlGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(ctrl_handler), false);
        }
    }
}

fn install_ctrl_handler() -> Option<CtrlGuard> {
    if unsafe { SetConsoleCtrlHandler(Some(ctrl_handler), true) }.is_ok() {
        Some(CtrlGuard)
    } else {
        eprintln!("Could not listen for Ctrl+C. Any key still stops the run.");
        None
    }
}

unsafe extern "system" fn ctrl_handler(_ctrl: u32) -> BOOL {
    request_stop(STOP_CTRL);
    BOOL(1)
}

struct DirCleanup(std::path::PathBuf);

impl Drop for DirCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
