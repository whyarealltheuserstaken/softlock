//! Orchestrator, timer child, overlay child, and heartbeat child.

use std::fs;
use std::io::{self, BufRead, ErrorKind, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::args::{APP_NAP_SAMPLE_MS, Args, Mode};
use crate::mac::ffi::{self, HeldAssertions};
use crate::report::{
    AssertionReport, CaffeinateReport, DISPLAY_ASSERTION, HardwareRow, KIT_ID, KitReport,
    SYSTEM_ASSERTION, TimerReport, expected_ticks, failure_reasons, hardware_only_list,
    hardware_row, keep_awake_claim, laptop_lid_row, lid_external_row, pmset_shows, power_key_row,
    render, runner_hardware_row, slowed_sentence,
};
use crate::scrub::{contains_serial, filter_pmset};
use crate::version::parse_sw_vers;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_stop(_signal: i32) {
    STOP.store(true, Ordering::Relaxed);
}

fn stop_requested() -> bool {
    STOP.load(Ordering::Relaxed)
}

pub fn dispatch(args: Args) -> i32 {
    if args.mode.holds_assertions() {
        run_orchestrator(&args)
    } else {
        match args.mode {
            Mode::Timer => run_timer(&args),
            Mode::Overlay => run_overlay(&args),
            Mode::Heartbeat => run_heartbeat(&args),
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
    println!("{}", crate::report::instructions(kit));
    let _ = io::stdout().flush();

    let version = read_version();
    let display_count = ffi::display_count();
    let names = ffi::assertion_names();
    let mut held = HeldAssertions::create();
    let pid = std::process::id();
    let pmset_held = filtered_pmset(pid, &held.reason);
    let shows_reason = pmset_shows(&pmset_held, &held.reason);
    let shows_system = pmset_shows(&pmset_held, SYSTEM_ASSERTION);
    let shows_display = pmset_shows(&pmset_held, DISPLAY_ASSERTION);
    let caffeinate = caffeinate_control(pid);
    let mut interrupted = stop_requested();
    let timer = if interrupted {
        empty_timer(args, "interrupted before the timer sample")
    } else {
        match sample_timers(args) {
            Ok(timer) => timer,
            Err(error) => empty_timer(args, &error),
        }
    };
    interrupted = interrupted || stop_requested();
    let (lid_on_power, lid_external, power_key) = if interrupted {
        skipped_hardware("interrupted", measure_held(pid, &held.reason))
    } else {
        hardware_rows(args, pid, &held.reason)
    };
    interrupted = interrupted || stop_requested();
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
        probe_crash: None,
    };
    let text = render(&report);
    println!("{text}");
    let _ = io::stdout().flush();
    let path = args.output_dir.join("m07.json");
    if let Err(error) = write_json(&path, &report) {
        eprintln!("{error}");
        return 1;
    }
    if interrupted || !failure_reasons(&report).is_empty() {
        1
    } else {
        0
    }
}

fn empty_timer(args: &Args, error: &str) -> TimerReport {
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
        expected_ticks: expected_ticks(args.sample_ms, args.interval_ms),
        app_nap_ms: if args.mode == Mode::Kit {
            APP_NAP_SAMPLE_MS
        } else {
            0
        },
        app_nap_count: None,
        app_nap_occlusion_visible: None,
        app_nap_ran: false,
        slowed: false,
        sentence: error.to_string(),
        error: Some(error.to_string()),
    }
}

fn skipped_hardware(status: &str, held: bool) -> (HardwareRow, HardwareRow, HardwareRow) {
    (
        hardware_row(
            status.to_string(),
            None,
            None,
            held,
            "skipped because the kit was interrupted",
        ),
        hardware_row(
            status.to_string(),
            None,
            None,
            held,
            "skipped because the kit was interrupted",
        ),
        hardware_row(
            status.to_string(),
            None,
            None,
            held,
            "skipped because the kit was interrupted",
        ),
    )
}

fn sample_timers(args: &Args) -> Result<TimerReport, String> {
    let exe = current_exe()?;
    let down = run_child_timer(&exe, args, "timer-down.json")?;
    if stop_requested() {
        return Err("interrupted".to_string());
    }
    let (up, overlay) = run_child_timer_with_overlay(&exe, args, "timer-up.json", args.sample_ms)?;
    if stop_requested() {
        return Err("interrupted".to_string());
    }
    let expected = expected_ticks(args.sample_ms, args.interval_ms);
    let (slowed, sentence) = match (down.ticks, up.ticks) {
        (Some(down_n), Some(up_n)) => slowed_sentence(down_n, up_n, expected),
        _ => (
            false,
            "Timer counts are incomplete, so slowing is not claimed.".to_string(),
        ),
    };
    let mut app_nap_count = None;
    let mut app_nap_occlusion = None;
    let mut app_nap_ran = false;
    if args.mode == Mode::Kit && !stop_requested() {
        println!(
            "STEP hands-off App Nap: --kit covers only the timer window for 300 s (the overlay-down/up pair is 60 s each). Activity Monitor stays visible. Do not touch the Mac. Check the App Nap column for the timer process. Control-C ends it any time; Terminal still has keyboard focus behind the black screen."
        );
        let _ = io::stdout().flush();
        let nap = run_app_nap_timer(&exe, args, "timer-app-nap.json", APP_NAP_SAMPLE_MS)?;
        app_nap_count = nap.0.ticks;
        app_nap_occlusion = nap.0.occlusion_visible;
        app_nap_ran = true;
    }
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
        expected_ticks: expected_ticks(args.sample_ms, args.interval_ms),
        app_nap_ms: if args.mode == Mode::Kit {
            APP_NAP_SAMPLE_MS
        } else {
            0
        },
        app_nap_count,
        app_nap_occlusion_visible: app_nap_occlusion,
        app_nap_ran,
        slowed,
        sentence,
        error: None,
    })
}

struct CountFile {
    ticks: Option<u64>,
    occlusion_visible: Option<bool>,
}

struct OverlayState {
    ordered_front: bool,
    opaque: bool,
    display_count: i32,
}

fn run_child_timer(exe: &Path, args: &Args, count_file: &str) -> Result<CountFile, String> {
    let path = args.output_dir.join(count_file);
    let out = args.output_dir.to_str().unwrap_or(".").to_string();
    let sample = args.sample_ms.to_string();
    let interval = args.interval_ms.to_string();
    let mut child = spawn_role(
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
    let status = wait_child(&mut child)?;
    if !status.success() {
        return Err(format!("timer {count_file} exited {status}"));
    }
    read_count(&path)
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
    let timer_status = wait_child(&mut timer)?;
    if !timer_status.success() {
        return Err(format!(
            "timer {count_file} overlay-up exited {timer_status}"
        ));
    }
    let count = read_count(&timer_json)?;
    Ok((count, read_overlay(&overlay_json)))
}

fn run_app_nap_timer(
    exe: &Path,
    args: &Args,
    count_file: &str,
    sample_ms: u64,
) -> Result<(CountFile, OverlayState), String> {
    let overlay_json = args.output_dir.join("overlay.json");
    let timer_json = args.output_dir.join(count_file);
    let frame_path = args.output_dir.join("timer-frame.json");
    let _ = fs::remove_file(&frame_path);
    let out = args.output_dir.to_str().unwrap_or(".").to_string();
    let sample = sample_ms.to_string();
    let interval = args.interval_ms.to_string();
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
    let frame = match wait_for_frame(&frame_path) {
        Ok(frame) => frame,
        Err(error) => {
            let _ = timer.kill();
            let _ = timer.wait();
            return Err(error);
        }
    };
    let cover = format!("{},{},{},{}", frame.0, frame.1, frame.2, frame.3);
    let _overlay = match OverlayChild::spawn(
        exe,
        &[
            "--role",
            "overlay",
            "--output-dir",
            &out,
            "--sample-ms",
            &sample,
            "--cover-frame",
            &cover,
        ],
    ) {
        Ok(overlay) => overlay,
        Err(error) => {
            let _ = timer.kill();
            let _ = timer.wait();
            return Err(error);
        }
    };
    let timer_status = wait_child(&mut timer)?;
    if !timer_status.success() {
        return Err(format!("timer {count_file} App Nap exited {timer_status}"));
    }
    let count = read_count(&timer_json)?;
    Ok((count, read_overlay(&overlay_json)))
}

fn wait_for_frame(path: &Path) -> Result<(i32, i32, i32, i32), String> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if stop_requested() {
            return Err("interrupted".to_string());
        }
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(file) = serde_json::from_str::<FrameFile>(&text) {
                return Ok((file.x as i32, file.y as i32, file.w as i32, file.h as i32));
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err("timer window frame was not published".to_string())
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
    let started = Instant::now();
    let limit = Duration::from_millis(args.sample_ms);
    let step = Duration::from_millis(args.interval_ms.max(1));
    let mut ticks = 0_u64;
    let mut occlusion_visible = true;
    while started.elapsed() < limit && !stop_requested() {
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
        thread::sleep(step);
    }
    let name = args.count_file.as_deref().unwrap_or("timer-down.json");
    let path = args.output_dir.join(name);
    if let Err(error) = write_json(
        &path,
        &TickFile {
            ticks,
            occlusion_visible,
        },
    ) {
        eprintln!("{error}");
        return 1;
    }
    if stop_requested() { 1 } else { 0 }
}

fn run_overlay(args: &Args) -> i32 {
    install_stop_handler();
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
    while started.elapsed() < limit && !stop_requested() {
        ffi::Overlay::pump(50);
        if ffi::Overlay::escape_seen() {
            break;
        }
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
        return skipped_hardware("interrupted", measure_held(pid, reason));
    }
    let mut lid = record_lid(
        args,
        "lid-on-power",
        args.lid_on_power.as_deref(),
        &[
            "STEP lid-closed-on-power",
            "Leave the Mac on power. Unplug the external display if one is attached and you can do that safely.",
            "Stay next to the machine. Close the lid. After about 20 seconds, open it.",
            "Both idle assertions are still held. This kit does not change the lid-close action and does not lock the session.",
            "Type slept, awake, or skip. Do not type no-lid.",
        ],
        laptop_lid_row,
        &["slept", "awake", "skip"],
        900_000,
        held,
    );
    lid.assertion_held = measure_held(pid, reason);
    if stop_requested() {
        let rest = skipped_hardware("interrupted", lid.assertion_held);
        return (lid, rest.1, rest.2);
    }
    let mut external = record_lid(
        args,
        "lid-external",
        args.lid_external.as_deref(),
        &[
            "STEP lid-closed-with-external-display",
            "If you use an external display, plug it in and wait until it shows the desktop.",
            "Close the lid, wait about 20 seconds, then open it. Clamshell may stay awake; that is the measurement.",
            "Both idle assertions are still held.",
            "Type slept, awake, no-external-display, or skip.",
        ],
        lid_external_row,
        &["slept", "awake", "no-external-display", "skip"],
        900_000,
        lid.assertion_held,
    );
    external.assertion_held = measure_held(pid, reason);
    if stop_requested() {
        let rest = skipped_hardware("interrupted", external.assertion_held);
        return (lid, external, rest.2);
    }
    let mut power = record_choice(
        args.power_key.as_deref(),
        &[
            "STEP power-key",
            "On this 2020 MacBook Pro the power key is the Touch ID button. A tap may sleep the display or lock the screen: unlock with Touch ID or your password, then answer.",
            "Tap it once. Do not hold it. Holding it can force a shutdown.",
            "Both idle assertions are still held. They are released after this row.",
            "Type slept, restart-panel, nothing, or skip.",
        ],
        power_key_row,
        &["slept", "restart-panel", "nothing", "skip"],
        "power key tap",
        external.assertion_held,
    );
    power.assertion_held = measure_held(pid, reason);
    (lid, external, power)
}

fn record_lid(
    args: &Args,
    stamp: &str,
    preset: Option<&str>,
    steps: &[&str],
    parse: fn(&str) -> Result<String, String>,
    allowed: &[&str],
    wait_ms: u64,
    held: bool,
) -> HardwareRow {
    for step in steps {
        println!("{step}");
    }
    let _ = io::stdout().flush();
    if stop_requested() {
        return hardware_row(
            "interrupted".to_string(),
            None,
            None,
            held,
            "interrupted before the lid row",
        );
    }
    let exe = match current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            return hardware_row("error".to_string(), None, None, held, &error);
        }
    };
    let beat_name = format!("{stamp}-heartbeat.txt");
    let beat = args.output_dir.join(&beat_name);
    let _ = fs::remove_file(&beat);
    let mut child = Command::new(&exe)
        .arg("--role")
        .arg("heartbeat")
        .arg("--output-dir")
        .arg(&args.output_dir)
        .arg("--sample-ms")
        .arg(wait_ms.to_string())
        .arg("--count-file")
        .arg(&beat_name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    let choice = match preset {
        Some(value) => value.to_string(),
        None => read_choice(allowed),
    };
    if let Some(mut child) = child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if choice == "interrupted" || stop_requested() {
        return hardware_row(
            "interrupted".to_string(),
            None,
            heartbeat_gap(&beat),
            held,
            "operator interrupted",
        );
    }
    let gap = heartbeat_gap(&beat);
    match parse(&choice) {
        Ok(outcome) => hardware_row(
            "recorded".to_string(),
            Some(outcome),
            gap,
            held,
            "operator row plus heartbeat gap if the process paused",
        ),
        Err(error) => hardware_row("error".to_string(), None, gap, held, &error),
    }
}

fn record_choice(
    preset: Option<&str>,
    steps: &[&str],
    parse: fn(&str) -> Result<String, String>,
    allowed: &[&str],
    note: &str,
    held: bool,
) -> HardwareRow {
    for step in steps {
        println!("{step}");
    }
    let _ = io::stdout().flush();
    if stop_requested() {
        return hardware_row(
            "interrupted".to_string(),
            None,
            None,
            held,
            "interrupted before the prompt",
        );
    }
    let choice = match preset {
        Some(value) => value.to_string(),
        None => read_choice(allowed),
    };
    if choice == "interrupted" || stop_requested() {
        return hardware_row(
            "interrupted".to_string(),
            None,
            None,
            held,
            "operator interrupted",
        );
    }
    match parse(&choice) {
        Ok(outcome) => hardware_row("recorded".to_string(), Some(outcome), None, held, note),
        Err(error) => hardware_row("error".to_string(), None, None, held, &error),
    }
}

fn read_choice(allowed: &[&str]) -> String {
    if stop_requested() {
        return "interrupted".to_string();
    }
    if !stdin_is_terminal() {
        return "needs-terminal".to_string();
    }
    print!("> ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    loop {
        if stop_requested() {
            return "interrupted".to_string();
        }
        match io::stdin().lock().read_line(&mut line) {
            Ok(0) => return "skip".to_string(),
            Ok(_) => {
                let value = line.trim();
                if allowed.contains(&value) {
                    return value.to_string();
                }
                return "skip".to_string();
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {
                if stop_requested() {
                    return "interrupted".to_string();
                }
                line.clear();
            }
            Err(_) => return "skip".to_string(),
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

fn read_count(path: &Path) -> Result<CountFile, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let file: TickFile = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    Ok(CountFile {
        ticks: Some(file.ticks),
        occlusion_visible: Some(file.occlusion_visible),
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
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
    }
}

#[derive(Serialize, serde::Deserialize)]
struct TickFile {
    ticks: u64,
    occlusion_visible: bool,
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
