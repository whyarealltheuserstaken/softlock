//! Orchestrator, timer child, overlay child, and heartbeat child.

use std::fs;
use std::io::{self, BufRead, ErrorKind, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::args::{APP_NAP_SAMPLE_MS, Args, Mode};
use crate::drift::{APP_NAP_GAP_THRESHOLD_MS, DriftStats, nap_verdict, stats_from_gaps};
use crate::mac::ffi::{self, HeldAssertions};
use crate::plan::{self, SetupSkip};
use crate::report::{
    AssertionReport, CaffeinateReport, DISPLAY_ASSERTION, HardwareRow, KIT_ID, KitReport,
    SYSTEM_ASSERTION, TimerReport, exit_nonzero, expected_ticks, hardware_only_list, hardware_row,
    keep_awake_claim, laptop_lid_row, lid_external_row, pmset_shows, power_key_row, render,
    runner_hardware_row, slowed_sentence,
};
use crate::scrub::{contains_serial, filter_pmset};
use crate::sleep_detect::{
    classify_pmset_since, detected_sleep, detection_method, lid_close_seen, log_since,
    outcome_token,
};
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
        Ok(code) => code,
        Err(_) => {
            let _ = write_crash_report(&output_dir, "probe panicked");
            1
        }
    }
}

fn dispatch_inner(args: &Args) -> i32 {
    if args.mode.holds_assertions() {
        run_orchestrator(args)
    } else {
        match args.mode {
            Mode::Timer => run_timer(args),
            Mode::Overlay => run_overlay(args),
            Mode::Heartbeat => run_heartbeat(args),
            Mode::Runner | Mode::Kit => unreachable!("orchestrator already ran"),
        }
    }
}

fn run_orchestrator(args: &Args) -> i32 {
    install_stop_handler();
    if let Err(error) = fs::create_dir_all(&args.output_dir) {
        eprintln!("output dir: {error}");
        return 1;
    }
    let kit = args.mode == Mode::Kit;
    crate::report::write_stdout(crate::report::instructions(kit));
    for name in [
        "timer-down.json",
        "timer-up.json",
        "timer-app-nap.json",
        "overlay.json",
        "heartbeat.txt",
        "lid-on-power-heartbeat.txt",
        "lid-external-heartbeat.txt",
    ] {
        let _ = fs::remove_file(args.output_dir.join(name));
    }

    let version = read_version();
    let display_count = ffi::display_count();
    let names = ffi::assertion_names();
    let mut held = HeldAssertions::create();
    let pid = std::process::id();
    let pmset_held = filtered_pmset(pid, &held.reason);
    let shows_reason = pmset_shows(&pmset_held, &held.reason);
    let shows_system = pmset_shows(&pmset_held, SYSTEM_ASSERTION);
    let shows_display = pmset_shows(&pmset_held, DISPLAY_ASSERTION);
    let mut interrupted = stop_requested();
    let mut no_input = false;
    let exe = current_exe().ok();
    let down_ms = args.down_ms.unwrap_or(args.sample_ms);
    let down_child = if interrupted {
        None
    } else {
        exe.as_ref()
            .and_then(|exe| start_child_timer(exe, args, "timer-down.json", down_ms).ok())
    };
    let caffeinate = caffeinate_control(pid);
    interrupted = interrupted || stop_requested();
    let timer = if interrupted {
        if let Some(mut child) = down_child {
            let _ = child.kill();
            let _ = child.wait();
        }
        empty_timer(args, "interrupted before the timer sample")
    } else {
        match sample_timers(args, down_child) {
            Ok(timer) => timer,
            Err(error) => empty_timer(args, &error),
        }
    };
    interrupted = interrupted || stop_requested();
    let (lid_on_power, lid_external, power_key) = if interrupted {
        skipped_hardware(
            "interrupted",
            measure_held(pid, &held.reason),
            "interrupted",
        )
    } else {
        let rows = hardware_rows(args, pid, &held.reason);
        no_input = row_no_input(&rows.0) || row_no_input(&rows.1) || row_no_input(&rows.2);
        rows
    };
    interrupted = interrupted || stop_requested();
    if interrupted {
        no_input = false;
    }
    held.release();
    let pmset_after = filtered_pmset(pid, &held.reason);
    let after_shows = pmset_shows(&pmset_after, &held.reason);
    let assertions = AssertionReport {
        names_compiled: names,
        reason: held.reason.clone(),
        created: held.created,
        released: held.released,
        pmset_while_held: pmset_held.clone(),
        pmset_after_release: pmset_after.clone(),
        pmset_shows_reason: shows_reason,
        pmset_shows_system: shows_system,
        pmset_shows_display: shows_display,
        pmset_after_shows_reason: after_shows,
        error: held.error.clone(),
    };
    let reversible = held.released && !after_shows;
    let serial_collected = contains_serial(&pmset_held)
        || contains_serial(&pmset_after)
        || contains_serial(&version.hw_model);
    let report = KitReport {
        kit: KIT_ID.to_string(),
        mode: if kit { "kit" } else { "runner" }.to_string(),
        version,
        display_count,
        keep_awake_claim: keep_awake_claim(
            assertions.created && assertions.released && !interrupted,
            &lid_on_power,
        ),
        assertions,
        caffeinate,
        timer,
        lid_on_power,
        lid_external,
        power_key,
        hardware_only: hardware_only_list(),
        serial_collected,
        locked_session: false,
        reversible,
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

fn row_no_input(row: &HardwareRow) -> bool {
    row.status == "no-input" || row.detection_method == "no-input"
}

fn empty_timer(args: &Args, error: &str) -> TimerReport {
    let expected = expected_ticks(args.sample_ms, args.interval_ms);
    let stats = stats_from_gaps(0, expected, &[], &[]);
    let verdict = nap_verdict(&stats, args.sample_ms);
    TimerReport {
        interval_ms: args.interval_ms,
        duration_ms: args.sample_ms,
        overlay_down_count: None,
        overlay_up_count: None,
        overlay_ordered_front: false,
        overlay_opaque: true,
        overlay_display_count: 0,
        occlusion_visible_down: None,
        occlusion_visible_up: None,
        expected_ticks: expected,
        uncovered_expected_ticks: expected,
        app_nap_ms: if args.mode == Mode::Kit {
            APP_NAP_SAMPLE_MS
        } else {
            0
        },
        app_nap_count: None,
        app_nap_occlusion_visible: None,
        app_nap_ran: false,
        max_gap_ms: None,
        p95_gap_ms: None,
        mean_gap_ms: None,
        max_wake_late_ms: None,
        app_nap_threshold_ms: APP_NAP_GAP_THRESHOLD_MS,
        app_nap_verdict: verdict.token.to_string(),
        app_nap_reason: verdict.reason,
        slowed: None,
        sentence: error.to_string(),
        error: Some(error.to_string()),
    }
}

fn skipped_hardware(
    status: &str,
    held: bool,
    method: &str,
) -> (HardwareRow, HardwareRow, HardwareRow) {
    let note = if status == "no-input" {
        "stdin closed; results still written"
    } else {
        "skipped because the kit was interrupted"
    };
    (
        hardware_row(status.to_string(), None, None, held, note, method),
        hardware_row(status.to_string(), None, None, held, note, method),
        hardware_row(status.to_string(), None, None, held, note, method),
    )
}

fn sample_timers(args: &Args, down_child: Option<Child>) -> Result<TimerReport, String> {
    let exe = current_exe()?;
    let down_ms = args.down_ms.unwrap_or(args.sample_ms);
    let down = if let Some(child) = down_child {
        finish_child_timer(child, args, "timer-down.json", down_ms)?
    } else {
        run_child_timer(&exe, args, "timer-down.json", down_ms)?
    };
    if stop_requested() {
        return Err("interrupted".to_string());
    }
    let overlay_secs = (args.sample_ms / 1000).max(1);
    crate::report::write_stdout(&format!(
        "The screen will go black for {overlay_secs} seconds. That is expected."
    ));
    let (up, overlay) = run_child_timer_with_overlay(&exe, args, "timer-up.json", args.sample_ms)?;
    if stop_requested() {
        return Err("interrupted".to_string());
    }
    let expected = expected_ticks(args.sample_ms, args.interval_ms);
    let uncovered_expected = expected_ticks(down_ms, args.interval_ms);
    let (slowed, sentence) = match (down.ticks, up.ticks) {
        (Some(down_n), Some(up_n)) => {
            slowed_sentence(down_n, down_ms, up_n, args.sample_ms, args.interval_ms)
        }
        _ => (
            None,
            "Timer counts are incomplete, so slowing is not claimed.".to_string(),
        ),
    };
    let stats = up
        .drift
        .clone()
        .unwrap_or_else(|| stats_from_gaps(up.ticks.unwrap_or(0), expected, &[], &[]));
    let sample_for_nap = args.sample_ms;
    let verdict = nap_verdict(&stats, sample_for_nap);
    let app_nap_ran = args.mode == Mode::Kit && !stop_requested();
    Ok(TimerReport {
        interval_ms: args.interval_ms,
        duration_ms: args.sample_ms,
        overlay_down_count: down.ticks,
        overlay_up_count: up.ticks,
        overlay_ordered_front: overlay.ordered_front,
        overlay_opaque: overlay.opaque,
        overlay_display_count: overlay.display_count,
        occlusion_visible_down: down.occlusion_visible,
        occlusion_visible_up: up.occlusion_visible,
        expected_ticks: expected,
        uncovered_expected_ticks: uncovered_expected,
        app_nap_ms: if args.mode == Mode::Kit {
            APP_NAP_SAMPLE_MS
        } else {
            0
        },
        app_nap_count: if app_nap_ran { up.ticks } else { None },
        app_nap_occlusion_visible: if app_nap_ran {
            up.occlusion_visible
        } else {
            None
        },
        app_nap_ran,
        max_gap_ms: Some(stats.max_gap_ms),
        p95_gap_ms: Some(stats.p95_gap_ms),
        mean_gap_ms: Some(stats.mean_gap_ms),
        max_wake_late_ms: Some(stats.max_wake_late_ms),
        app_nap_threshold_ms: APP_NAP_GAP_THRESHOLD_MS,
        app_nap_verdict: if args.mode == Mode::Kit {
            verdict.token.to_string()
        } else {
            "not-run-on-runner".to_string()
        },
        app_nap_reason: verdict.reason,
        slowed,
        sentence,
        error: None,
    })
}

struct CountFile {
    ticks: Option<u64>,
    occlusion_visible: Option<bool>,
    drift: Option<DriftStats>,
}

struct OverlayState {
    ordered_front: bool,
    opaque: bool,
    display_count: i32,
}

fn start_child_timer(
    exe: &Path,
    args: &Args,
    count_file: &str,
    sample_ms: u64,
) -> Result<Child, String> {
    let out = args.output_dir.to_str().unwrap_or(".").to_string();
    let sample = sample_ms.to_string();
    let interval = args.interval_ms.to_string();
    spawn_role(
        exe,
        &[
            "--role",
            "timer",
            "--output-dir",
            &out,
            "--sample-ms",
            &sample,
            "--interval-ms",
            &interval,
            "--count-file",
            count_file,
        ],
    )
}

fn finish_child_timer(
    mut child: Child,
    args: &Args,
    count_file: &str,
    sample_ms: u64,
) -> Result<CountFile, String> {
    let status = wait_child(&mut child)?;
    if !status.success() {
        return Err(format!("timer {count_file} exited {status}"));
    }
    read_count(
        &args.output_dir.join(count_file),
        expected_ticks(sample_ms, args.interval_ms),
    )
}

fn run_child_timer(
    exe: &Path,
    args: &Args,
    count_file: &str,
    sample_ms: u64,
) -> Result<CountFile, String> {
    let child = start_child_timer(exe, args, count_file, sample_ms)?;
    finish_child_timer(child, args, count_file, sample_ms)
}

struct OverlayChild {
    child: Option<Child>,
}

impl OverlayChild {
    fn spawn(exe: &Path, extra: &[&str]) -> Result<Self, String> {
        Ok(Self {
            child: Some(spawn_role(exe, extra)?),
        })
    }
}

impl Drop for OverlayChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn run_child_timer_with_overlay(
    exe: &Path,
    args: &Args,
    count_file: &str,
    sample_ms: u64,
) -> Result<(CountFile, OverlayState), String> {
    let overlay_json = args.output_dir.join("overlay.json");
    let timer_json = args.output_dir.join(count_file);
    let out = args.output_dir.to_str().unwrap_or(".").to_string();
    let sample = sample_ms.to_string();
    let interval = args.interval_ms.to_string();
    let _overlay = OverlayChild::spawn(
        exe,
        &[
            "--role",
            "overlay",
            "--output-dir",
            &out,
            "--sample-ms",
            &sample,
        ],
    )?;
    thread::sleep(Duration::from_millis(400));
    let mut timer = spawn_role(
        exe,
        &[
            "--role",
            "timer",
            "--output-dir",
            &out,
            "--sample-ms",
            &sample,
            "--interval-ms",
            &interval,
            "--count-file",
            count_file,
        ],
    )?;
    crate::report::write_stdout(&format!("timer-pid={}", timer.id()));
    let timer_status = wait_child(&mut timer)?;
    if !timer_status.success() {
        return Err(format!(
            "timer {count_file} overlay-up exited {timer_status}"
        ));
    }
    let count = read_count(&timer_json, expected_ticks(sample_ms, args.interval_ms))?;
    Ok((count, read_overlay(&overlay_json)))
}

fn spawn_role(exe: &Path, extra: &[&str]) -> Result<Child, String> {
    Command::new(exe)
        .args(extra)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| error.to_string())
}

fn wait_child(child: &mut Child) -> Result<std::process::ExitStatus, String> {
    loop {
        match child.wait() {
            Ok(status) => return Ok(status),
            Err(error) if error.kind() == ErrorKind::Interrupted => {
                if stop_requested() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("interrupted".to_string());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn run_timer(args: &Args) -> i32 {
    install_stop_handler();
    let window = ffi::TimerWindow::open();
    if !window.ordered_front {
        eprintln!(
            "timer window: {}",
            window.error.as_deref().unwrap_or("not ordered front")
        );
        return 1;
    }
    let frame = FrameFile {
        x: window.frame.0,
        y: window.frame.1,
        w: window.frame.2,
        h: window.frame.3,
    };
    if let Err(error) = write_json(&args.output_dir.join("timer-frame.json"), &frame) {
        eprintln!("{error}");
        return 1;
    }
    let parent = unsafe { libc::getppid() };
    let started = Instant::now();
    let limit = Duration::from_millis(args.sample_ms);
    let step = Duration::from_millis(args.interval_ms.max(1));
    let mut ticks = 0_u64;
    let mut occlusion_visible = true;
    let mut gaps = Vec::new();
    let mut lates = Vec::new();
    let mut last_tick: Option<Instant> = None;
    while started.elapsed() < limit && !stop_requested() && parent_alive(parent) {
        let now = Instant::now();
        if let Some(previous) = last_tick {
            gaps.push(now.duration_since(previous).as_millis() as u64);
        }
        last_tick = Some(now);
        match ffi::TimerWindow::tick() {
            Ok((value, visible)) => {
                ticks = value;
                occlusion_visible = visible;
            }
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        }
        let sleep_started = Instant::now();
        thread::sleep(step);
        let slept = sleep_started.elapsed().as_millis() as u64;
        lates.push(slept.saturating_sub(args.interval_ms));
    }
    let expected = expected_ticks(args.sample_ms, args.interval_ms);
    let stats = stats_from_gaps(ticks, expected, &gaps, &lates);
    let name = args.count_file.as_deref().unwrap_or("timer-down.json");
    let path = args.output_dir.join(name);
    if let Err(error) = write_json(
        &path,
        &TickFile {
            ticks,
            occlusion_visible,
            max_gap_ms: stats.max_gap_ms,
            p95_gap_ms: stats.p95_gap_ms,
            mean_gap_ms: stats.mean_gap_ms,
            max_wake_late_ms: stats.max_wake_late_ms,
        },
    ) {
        eprintln!("{error}");
        return 1;
    }
    if stop_requested() { 1 } else { 0 }
}

fn run_overlay(args: &Args) -> i32 {
    install_stop_handler();
    let parent = unsafe { libc::getppid() };
    let overlay = ffi::Overlay::open_rect(args.cover_frame);
    let path = args.output_dir.join("overlay.json");
    let marker = OverlayFile {
        ordered_front: overlay.ordered_front,
        opaque: overlay.opaque,
        display_count: overlay.display_count,
        error: overlay.error.clone(),
    };
    if let Err(error) = write_json(&path, &marker) {
        eprintln!("{error}");
        return 1;
    }
    if !overlay.ordered_front {
        return 1;
    }
    let started = Instant::now();
    let limit = Duration::from_millis(args.sample_ms + 500);
    while started.elapsed() < limit && !stop_requested() && parent_alive(parent) {
        ffi::Overlay::pump(50);
    }
    if stop_requested() { 1 } else { 0 }
}

fn run_heartbeat(args: &Args) -> i32 {
    install_stop_handler();
    let parent = unsafe { libc::getppid() };
    let name = args.count_file.as_deref().unwrap_or("heartbeat.txt");
    let path = args.output_dir.join(name);
    let started = Instant::now();
    let limit = Duration::from_millis(args.sample_ms.max(30_000));
    while started.elapsed() < limit && !stop_requested() && parent_alive(parent) {
        let now = wall_ms();
        if let Err(error) = append_line(&path, &now.to_string()) {
            eprintln!("{error}");
            return 1;
        }
        thread::sleep(Duration::from_millis(500));
    }
    0
}

fn parent_alive(parent: libc::pid_t) -> bool {
    if parent <= 1 {
        return false;
    }
    unsafe { libc::getppid() == parent }
}

fn measure_held(pid: u32, reason: &str) -> bool {
    let excerpt = filtered_pmset(pid, reason);
    pmset_shows(&excerpt, reason)
        && pmset_shows(&excerpt, SYSTEM_ASSERTION)
        && pmset_shows(&excerpt, DISPLAY_ASSERTION)
}

fn hardware_rows(args: &Args, pid: u32, reason: &str) -> (HardwareRow, HardwareRow, HardwareRow) {
    let held = measure_held(pid, reason);
    if args.mode == Mode::Runner {
        return (
            runner_hardware_row("lid close on power", held),
            runner_hardware_row("lid close with an external display", held),
            runner_hardware_row("power key", held),
        );
    }
    if stop_requested() {
        return skipped_hardware("interrupted", measure_held(pid, reason), "interrupted");
    }
    let mut lid = record_physical(
        args,
        PhysicalSpec {
            stamp: "lid-on-power",
            preset: args.lid_on_power.as_deref(),
            instruction: "The Mac will sleep and lock. Close the lid on power, unlock, then press Enter. Type skip to skip.",
            parse: laptop_lid_row,
            held,
            timeout: Duration::from_secs(25),
            kind: LidKind::OnPower,
        },
    );
    lid.assertion_held = measure_held(pid, reason);
    if stop_requested() {
        let rest = skipped_hardware("interrupted", lid.assertion_held, "interrupted");
        return (lid, rest.1, rest.2);
    }
    if lid.status == "no-input" {
        let rest = skipped_hardware("no-input", lid.assertion_held, "no-input");
        return (lid, rest.1, rest.2);
    }
    let mut external = record_physical(
        args,
        PhysicalSpec {
            stamp: "lid-external",
            preset: args.lid_external.as_deref(),
            instruction: "Plug in a display, close the lid (the Mac sleeps and locks), unlock, then press Enter. Type skip to skip.",
            parse: lid_external_row,
            held: lid.assertion_held,
            timeout: Duration::from_secs(25),
            kind: LidKind::External,
        },
    );
    external.assertion_held = measure_held(pid, reason);
    if stop_requested() {
        let rest = skipped_hardware("interrupted", external.assertion_held, "interrupted");
        return (lid, external, rest.2);
    }
    if external.status == "no-input" {
        let rest = skipped_hardware("no-input", external.assertion_held, "no-input");
        return (lid, external, rest.2);
    }
    let mut power = record_physical(
        args,
        PhysicalSpec {
            stamp: "power-key",
            preset: args.power_key.as_deref(),
            instruction: "Tap the Touch ID (power) key once; do not hold it. Press Enter when that is done. Type skip to skip.",
            parse: power_key_row,
            held: external.assertion_held,
            timeout: Duration::from_secs(12),
            kind: LidKind::PowerKey,
        },
    );
    power.assertion_held = measure_held(pid, reason);
    (lid, external, power)
}

#[derive(Clone, Copy)]
enum LidKind {
    OnPower,
    External,
    PowerKey,
}

struct PhysicalSpec<'a> {
    stamp: &'a str,
    preset: Option<&'a str>,
    instruction: &'a str,
    parse: fn(&str) -> Result<String, String>,
    held: bool,
    timeout: Duration,
    kind: LidKind,
}

fn record_physical(args: &Args, spec: PhysicalSpec<'_>) -> HardwareRow {
    let PhysicalSpec {
        stamp,
        preset,
        instruction,
        parse,
        held,
        timeout,
        kind,
    } = spec;
    if preset.is_none() && !stdin_is_terminal() {
        return hardware_row(
            "no-input".to_string(),
            Some("no-input".to_string()),
            None,
            held,
            "stdin closed before the prompt; results still written",
            "no-input",
        );
    }
    if preset.is_none()
        && let Some(skip) = setup_skip(kind)
    {
        return hardware_row(
            "recorded".to_string(),
            Some(skip.outcome.to_string()),
            None,
            held,
            skip.note,
            "pre-prompt",
        );
    }
    let since = match local_stamp() {
        Some(stamp) => stamp,
        None => {
            return hardware_row(
                "error".to_string(),
                Some("not-detected".to_string()),
                None,
                held,
                "no clock stamp before the prompt",
                "none",
            );
        }
    };
    let watch = if preset.is_none() {
        Some(ffi::PowerWatch::start())
    } else {
        None
    };
    let watch_ok = watch.as_ref().is_some_and(|(_, ok)| *ok);
    drain_stdin();
    crate::report::write_stdout(instruction);
    if stop_requested() {
        return hardware_row(
            "interrupted".to_string(),
            None,
            None,
            held,
            "interrupted before the row",
            "interrupted",
        );
    }
    let beat = start_heartbeat(args, stamp);
    let choice = match preset {
        Some(value) => Choice {
            outcome: value.to_string(),
            method: "preset".to_string(),
        },
        None => wait_for_sleep_or_skip(Wait {
            timeout,
            kind,
            since: &since,
            instruction,
            watch_ok,
        }),
    };
    drop(watch);
    let gap = stop_heartbeat(beat);
    if choice.outcome == "interrupted" || stop_requested() {
        return hardware_row(
            "interrupted".to_string(),
            None,
            gap,
            held,
            "operator interrupted",
            "interrupted",
        );
    }
    if choice.outcome == "no-input" {
        return hardware_row(
            "no-input".to_string(),
            Some("no-input".to_string()),
            gap,
            held,
            "stdin closed; results still written",
            "no-input",
        );
    }
    match parse(&choice.outcome) {
        Ok(outcome) => hardware_row(
            "recorded".to_string(),
            Some(outcome),
            gap,
            held,
            "sleep detection plus heartbeat gap if the process paused",
            &choice.method,
        ),
        Err(error) => hardware_row("error".to_string(), None, gap, held, &error, &choice.method),
    }
}

struct Choice {
    outcome: String,
    method: String,
}

struct HeartbeatChild {
    child: Option<Child>,
    path: PathBuf,
}

fn start_heartbeat(args: &Args, stamp: &str) -> Option<HeartbeatChild> {
    let exe = current_exe().ok()?;
    let beat_name = format!("{stamp}-heartbeat.txt");
    let path = args.output_dir.join(&beat_name);
    let _ = fs::remove_file(&path);
    let child = Command::new(&exe)
        .arg("--role")
        .arg("heartbeat")
        .arg("--output-dir")
        .arg(&args.output_dir)
        .arg("--sample-ms")
        .arg("900000")
        .arg("--count-file")
        .arg(&beat_name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    Some(HeartbeatChild { child, path })
}

fn stop_heartbeat(beat: Option<HeartbeatChild>) -> Option<u64> {
    let mut beat = beat?;
    if let Some(mut child) = beat.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    heartbeat_gap(&beat.path)
}

struct Wait<'a> {
    timeout: Duration,
    kind: LidKind,
    since: &'a str,
    instruction: &'a str,
    watch_ok: bool,
}

fn wait_for_sleep_or_skip(wait: Wait<'_>) -> Choice {
    if stop_requested() {
        return Choice {
            outcome: "interrupted".to_string(),
            method: "interrupted".to_string(),
        };
    }
    drain_stdin();
    crate::report::write_stdout(">");
    let mut pending = String::new();
    let mut saw_closed = false;
    loop {
        if stop_requested() {
            return Choice {
                outcome: "interrupted".to_string(),
                method: "interrupted".to_string(),
            };
        }
        if ffi::clamshell_closed() == 1 {
            saw_closed = true;
        }
        ffi::PowerWatch::pump(50);
        match poll_line(&mut pending, 150) {
            LineRead::Pending => {}
            LineRead::Eof => {
                return Choice {
                    outcome: "no-input".to_string(),
                    method: "no-input".to_string(),
                };
            }
            LineRead::Interrupted => {
                return Choice {
                    outcome: "interrupted".to_string(),
                    method: "interrupted".to_string(),
                };
            }
            LineRead::Line(line) => {
                let word = line.trim().to_ascii_lowercase();
                if word == "skip" {
                    return Choice {
                        outcome: "skip".to_string(),
                        method: "operator-skip".to_string(),
                    };
                }
                if word.is_empty() || word == "enter" || word == "y" || word == "yes" {
                    return watch_after_prompt(
                        wait.timeout,
                        wait.kind,
                        wait.since,
                        saw_closed,
                        wait.watch_ok,
                    );
                }
                crate::report::write_stdout(wait.instruction);
                crate::report::write_stdout(">");
            }
        }
    }
}

fn setup_skip(kind: LidKind) -> Option<SetupSkip> {
    let model = command_text("sysctl", &["-n", "hw.model"]);
    let registry = match ffi::has_lid() {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    };
    let laptop = plan::has_lid(&model, registry);
    match kind {
        LidKind::OnPower => {
            let batt = command_text("pmset", &["-g", "batt"]);
            plan::lid_on_power(laptop, plan::ac_power(&batt))
        }
        LidKind::External => plan::lid_external(laptop, ffi::display_count()),
        LidKind::PowerKey => plan::power_key(plan::touch_id_sensor(ffi::touch_id())),
    }
}

fn command_text(program: &str, args: &[&str]) -> String {
    command_result(program, args).0
}

fn command_result(program: &str, args: &[&str]) -> (String, bool) {
    match Command::new(program).args(args).output() {
        Ok(output) => (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            output.status.success(),
        ),
        Err(_) => (String::new(), false),
    }
}

fn drain_stdin() {
    unsafe {
        libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
    }
}

fn sleep_since(since: &str) -> Option<Choice> {
    let raw = pmset_log();
    let pmset = classify_pmset_since(&raw, since);
    let event = detected_sleep(
        ffi::PowerWatch::system_slept(),
        ffi::PowerWatch::display_slept(),
        pmset,
    )?;
    let method = detection_method(
        ffi::PowerWatch::system_slept(),
        ffi::PowerWatch::display_slept(),
        pmset,
    );
    Some(Choice {
        outcome: outcome_token(event).to_string(),
        method: method.to_string(),
    })
}

fn lid_method(since: &str, saw_closed: bool) -> Option<&'static str> {
    let window = log_since(&pmset_log(), since);
    if !lid_close_seen(ffi::PowerWatch::system_slept(), saw_closed, &window) {
        return None;
    }
    if ffi::PowerWatch::system_slept() {
        Some("iokit-power-notification")
    } else if crate::sleep_detect::clamshell_sleep(&window) {
        Some("pmset-log")
    } else {
        Some("clamshell-state")
    }
}

fn lid_choice(kind: LidKind, since: &str, saw_closed: bool) -> Option<Choice> {
    let method = lid_method(since, saw_closed)?;
    let outcome = if kind == LidKind::External && !ffi::PowerWatch::system_slept() {
        "closed"
    } else {
        "slept"
    };
    Some(Choice {
        outcome: outcome.to_string(),
        method: method.to_string(),
    })
}

fn watch_after_prompt(
    timeout: Duration,
    kind: LidKind,
    since: &str,
    mut saw_closed: bool,
    watch_ok: bool,
) -> Choice {
    ffi::PowerWatch::pump(200);
    if ffi::clamshell_closed() == 1 {
        saw_closed = true;
    }
    if matches!(kind, LidKind::OnPower | LidKind::External) {
        if let Some(choice) = lid_choice(kind, since, saw_closed) {
            return choice;
        }
    } else if let Some(choice) = sleep_since(since) {
        return choice;
    }
    let started = Instant::now();
    let mut log_ok = false;
    loop {
        if stop_requested() {
            return Choice {
                outcome: "interrupted".to_string(),
                method: "interrupted".to_string(),
            };
        }
        if ffi::clamshell_closed() == 1 {
            saw_closed = true;
        }
        let raw = pmset_log();
        if !raw.is_empty() {
            log_ok = true;
        }
        if matches!(kind, LidKind::OnPower | LidKind::External) {
            if let Some(choice) = lid_choice(kind, since, saw_closed) {
                return choice;
            }
        } else {
            let pmset = classify_pmset_since(&raw, since);
            if let Some(event) = detected_sleep(
                ffi::PowerWatch::system_slept(),
                ffi::PowerWatch::display_slept(),
                pmset,
            ) {
                let method = detection_method(
                    ffi::PowerWatch::system_slept(),
                    ffi::PowerWatch::display_slept(),
                    pmset,
                );
                return Choice {
                    outcome: outcome_token(event).to_string(),
                    method: method.to_string(),
                };
            }
        }
        if started.elapsed() >= timeout {
            break;
        }
        ffi::PowerWatch::pump(200);
    }
    let method = if watch_ok {
        "iokit-power-notification"
    } else if log_ok {
        "pmset-log"
    } else {
        "none"
    };
    if method == "none" {
        return Choice {
            outcome: "not-detected".to_string(),
            method: "none".to_string(),
        };
    }
    let outcome = if matches!(kind, LidKind::PowerKey) {
        "nothing"
    } else {
        "awake"
    };
    Choice {
        outcome: outcome.to_string(),
        method: method.to_string(),
    }
}

fn local_stamp() -> Option<String> {
    let output = Command::new("date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.len() >= 19 && crate::sleep_detect::line_stamp(&text).is_some() {
        Some(text)
    } else {
        None
    }
}

enum LineRead {
    Line(String),
    Pending,
    Eof,
    Interrupted,
}

fn poll_line(pending: &mut String, wait_ms: i32) -> LineRead {
    if let Some(line) = next_line(pending) {
        return LineRead::Line(line);
    }
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let rc = unsafe { libc::poll(&mut fd, 1, wait_ms) };
    if rc < 0 {
        return LineRead::Pending;
    }
    if rc == 0 {
        return LineRead::Pending;
    }
    if (fd.revents & libc::POLLIN) == 0 {
        return LineRead::Eof;
    }
    let mut buf = [0u8; 256];
    let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return LineRead::Pending;
    }
    if n == 0 {
        return LineRead::Eof;
    }
    pending.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
    if let Some(line) = next_line(pending) {
        LineRead::Line(line)
    } else {
        LineRead::Pending
    }
}

fn next_line(pending: &mut String) -> Option<String> {
    let end = pending.find('\n')?;
    let line = pending.drain(..=end).collect::<String>();
    Some(line.trim().to_string())
}

fn read_line_blocking() -> LineRead {
    let mut line = String::new();
    loop {
        if stop_requested() {
            return LineRead::Interrupted;
        }
        match io::stdin().lock().read_line(&mut line) {
            Ok(0) => return LineRead::Eof,
            Ok(_) => return LineRead::Line(line.trim().to_string()),
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

fn stdin_is_terminal() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

fn heartbeat_gap(path: &Path) -> Option<u64> {
    let text = fs::read_to_string(path).ok()?;
    let mut times = Vec::new();
    for line in text.lines() {
        if let Ok(value) = line.trim().parse::<u64>() {
            times.push(value);
        }
    }
    let mut gap = 0_u64;
    for pair in times.windows(2) {
        gap = gap.max(pair[1].saturating_sub(pair[0]));
    }
    if let Some(last) = times.last() {
        gap = gap.max(wall_ms().saturating_sub(*last));
    }
    Some(gap)
}

fn caffeinate_control(pid: u32) -> CaffeinateReport {
    let present = Command::new("which")
        .arg("caffeinate")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !present {
        return CaffeinateReport {
            present: false,
            ran: false,
            pmset_showed_caffeinate: false,
            error: None,
        };
    }
    let child = Command::new("caffeinate")
        .args(["-i", "-t", "5", "-w", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            thread::sleep(Duration::from_millis(400));
            let raw = pmset_raw();
            let showed = raw.to_ascii_lowercase().contains("caffeinate");
            let _ = wait_child(&mut child);
            CaffeinateReport {
                present: true,
                ran: true,
                pmset_showed_caffeinate: showed,
                error: None,
            }
        }
        Err(error) => CaffeinateReport {
            present: true,
            ran: false,
            pmset_showed_caffeinate: false,
            error: Some(error.to_string()),
        },
    }
}

fn pmset_raw() -> String {
    match Command::new("pmset").args(["-g", "assertions"]).output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            if !output.stderr.is_empty() {
                text.push('\n');
                text.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            text
        }
        Err(error) => format!("pmset failed: {error}"),
    }
}

fn pmset_log() -> String {
    match Command::new("pmset").args(["-g", "log"]).output() {
        Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

fn filtered_pmset(pid: u32, reason: &str) -> String {
    filter_pmset(&pmset_raw(), pid, reason)
}

fn read_version() -> crate::report::VersionReport {
    let sw = Command::new("sw_vers")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let uname = Command::new("uname")
        .arg("-m")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let mut version = parse_sw_vers(&sw, &uname);
    version.hw_model = ffi::hw_model();
    version
}

fn current_exe() -> Result<std::path::PathBuf, String> {
    std::env::current_exe().map_err(|error| error.to_string())
}

fn read_count(path: &Path, expected: u64) -> Result<CountFile, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let file: TickFile = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    Ok(CountFile {
        ticks: Some(file.ticks),
        occlusion_visible: Some(file.occlusion_visible),
        drift: Some(DriftStats {
            tick_count: file.ticks,
            expected_ticks: expected,
            max_gap_ms: file.max_gap_ms,
            p95_gap_ms: file.p95_gap_ms,
            mean_gap_ms: file.mean_gap_ms,
            max_wake_late_ms: file.max_wake_late_ms,
        }),
    })
}

fn read_overlay(path: &Path) -> OverlayState {
    let Ok(text) = fs::read_to_string(path) else {
        return OverlayState {
            ordered_front: false,
            opaque: false,
            display_count: 0,
        };
    };
    let Ok(file) = serde_json::from_str::<OverlayFile>(&text) else {
        return OverlayState {
            ordered_front: false,
            opaque: false,
            display_count: 0,
        };
    };
    OverlayState {
        ordered_front: file.ordered_front,
        opaque: file.opaque,
        display_count: file.display_count,
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    if contains_serial(&text) {
        return Err("refusing to write a serial number".to_string());
    }
    fs::write(path, text).map_err(|error| error.to_string())
}

fn write_outputs(dir: &Path, report: &KitReport, text: &str) -> Result<(), String> {
    write_json(&dir.join("result.json"), report)?;
    if contains_serial(text) {
        return Err("refusing to write a serial number".to_string());
    }
    fs::write(dir.join("result.txt"), text).map_err(|error| error.to_string())
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
    });
    fs::write(
        dir.join("result.json"),
        serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        dir.join("result.txt"),
        format!("probe-crash={message}\nresult=fail\n"),
    )
    .map_err(|error| error.to_string())
}

fn append_line(path: &Path, line: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    writeln!(file, "{line}").map_err(|error| error.to_string())
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
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

#[derive(Serialize, serde::Deserialize)]
struct TickFile {
    ticks: u64,
    occlusion_visible: bool,
    max_gap_ms: u64,
    p95_gap_ms: u64,
    mean_gap_ms: u64,
    max_wake_late_ms: u64,
}

#[derive(Serialize, serde::Deserialize)]
struct FrameFile {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Serialize, serde::Deserialize)]
struct OverlayFile {
    ordered_front: bool,
    opaque: bool,
    display_count: i32,
    error: Option<String>,
}
