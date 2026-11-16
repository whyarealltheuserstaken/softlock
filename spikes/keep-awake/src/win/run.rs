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
use crate::clock::{deadline_reached, sleep_aware_now_ms};
use crate::guard::RunGuard;
use crate::logfile::LogFile;
use crate::logline::format_header;
use crate::plan::{
    format_battery_percent, format_execution_state, format_lid_action, format_yes_no,
    holding_lines, ready_line, refuse_reason, requests_label, spike_power_plan, status_report,
};
use crate::session::RunReport;
use crate::stop::StopReason;
use crate::timefmt::{format_unix_millis, unix_now_ms};

use super::heat::HeatReader;
use super::overlay::{self, Overlay, STOP_MESSAGE, mark_loop_finished, request_stop, stop_reason};
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
    let heat = HeatReader::open();
    let mut guard = RunGuard::default();
    let header = header_text(spec, &machine, probe.mode());
    let mut log = LogFile::create(&spec.log_path(), &header)
        .map_err(|error| format!("Could not write {}: {error}", spec.log_path().display()))?;
    let started = Instant::now();
    let start_mono = sleep_aware_now_ms();
    let duration_ms = spec.duration_secs.saturating_mul(1000);
    let _ctrl = install_ctrl_handler();
    let _finish = FinishGuard;
    spawn_watchdog(start_mono, duration_ms);
    let mut ticks = 0usize;
    let mut last_tick = Instant::now();
    note_guard(
        &mut guard,
        &write_tick(&mut log, &probe, &heat, &hold, &mut ticks, started)?,
    );
    println!("Logging to: {}", spec.log_path().display());
    println!("{}", ready_line(spec.scenario, spec.duration_secs));
    println!("The run stops by itself at 20% battery or if a heat guard trips.");
    // SAFETY: the timer belongs to the overlay window and is killed before the window is destroyed.
    let timer = unsafe { SetTimer(Some(overlay.hwnd()), 1, 250, None) };
    if timer == 0 {
        return Err("Could not start the timer, so the run was stopped.".to_string());
    }

    let mut message = unsafe { MaybeUninit::<MSG>::zeroed().assume_init() };
    loop {
        // SAFETY: this thread created the window, so it owns the message queue.
        let rc = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if rc.0 == 0 {
            break;
        }
        if rc.0 < 0 {
            let _ = log.end(unix_now_ms(), StopReason::Window.token());
            return Err(StopReason::Window.sentence().to_string());
        }
        if message.message == STOP_MESSAGE {
            break;
        }
        if message.message == WM_TIMER {
            if deadline_reached(start_mono, sleep_aware_now_ms(), duration_ms) {
                request_stop(StopReason::Timer);
            } else if last_tick.elapsed() >= Duration::from_secs(spec.interval_secs) {
                note_guard(
                    &mut guard,
                    &write_tick(&mut log, &probe, &heat, &hold, &mut ticks, started)?,
                );
                last_tick = Instant::now();
            }
        }
        // SAFETY: the message was returned by GetMessageW on this thread.
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    // SAFETY: the timer id was set on this window above.
    let _ = unsafe { KillTimer(Some(overlay.hwnd()), 1) };
    let reason = stop_reason();
    log.end(unix_now_ms(), reason.token())
        .map_err(|error| format!("Could not finish the log: {error}"))?;
    println!("{}", reason.sentence());
    Ok(RunReport {
        ticks,
        overlay_shown: overlay.is_shown(),
        power_held: hold.is_held(),
        stop: reason.token().to_string(),
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
            "Smoke check failed. ticks={} overlay={} power={} gaps={}\n{}\n{raw}",
            report.ticks, report.overlay_shown, report.power_held, summary.gaps, summary.text
        ));
    }
    Ok(())
}

fn note_guard(guard: &mut RunGuard, signals: &crate::signals::PowerSignals) {
    if let Some(reason) = guard.observe(
        signals.battery_percent,
        signals.passive_limit,
        signals.throttle_reasons,
        signals.zone_dc,
        signals.battery_dc,
        sleep_aware_now_ms(),
    ) {
        request_stop(reason);
    }
}

fn write_tick(
    log: &mut LogFile,
    probe: &Probe,
    heat: &HeatReader,
    hold: &PowerHold,
    ticks: &mut usize,
    started: Instant,
) -> Result<crate::signals::PowerSignals, String> {
    let at = unix_now_ms();
    let mut signals = power::read_signals();
    heat.sample_into(&mut signals);
    let (kind, ok) = probe.sample();
    hold.refresh();
    log.tick(at, ok, kind, signals)
        .map_err(|error| format!("Could not write a log line: {error}"))?;
    *ticks += 1;
    println!(
        "+{} {} {} ui={}",
        format_elapsed(started.elapsed()),
        format_unix_millis(at),
        signals.progress_note(),
        if ok { "ok" } else { "fail" }
    );
    Ok(signals)
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

fn spawn_watchdog(start_ms: u64, duration_ms: u64) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(200));
            if overlay::loop_finished() {
                return;
            }
            let now = sleep_aware_now_ms();
            if deadline_reached(start_ms, now, duration_ms) {
                request_stop(StopReason::Timer);
            }
            if deadline_reached(start_ms, now, duration_ms.saturating_add(10_000)) {
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
        // SAFETY: the handler was added on this thread when the guard was created.
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(ctrl_handler), false);
        }
    }
}

fn install_ctrl_handler() -> Option<CtrlGuard> {
    // SAFETY: the handler only posts a stop message. It does not unwind.
    if unsafe { SetConsoleCtrlHandler(Some(ctrl_handler), true) }.is_ok() {
        Some(CtrlGuard)
    } else {
        eprintln!("Could not listen for Ctrl+C. Any key still stops the run.");
        None
    }
}

unsafe extern "system" fn ctrl_handler(_ctrl: u32) -> BOOL {
    request_stop(StopReason::CtrlC);
    BOOL(1)
}

struct DirCleanup(std::path::PathBuf);

impl Drop for DirCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
