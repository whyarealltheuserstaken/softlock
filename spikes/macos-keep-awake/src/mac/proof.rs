//! Orchestrator, timer child, overlay child, and heartbeat child.

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::args::{Args, Mode};
use crate::mac::ffi::{self, HeldAssertions};
use crate::report::{
    AssertionReport, CaffeinateReport, DISPLAY_ASSERTION, HardwareRow, KIT_ID, KitReport,
    SYSTEM_ASSERTION, TimerReport, failure_reasons, hardware_only_list, hardware_row,
    keep_awake_claim, laptop_lid_row, lid_external_row, pmset_shows, power_key_row, render,
    runner_hardware_row, slowed_sentence,
};
use crate::scrub::{contains_serial, scrub};
use crate::version::parse_sw_vers;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_stop(_signal: i32) {
    STOP.store(true, Ordering::Relaxed);
}

pub fn dispatch(args: Args) -> i32 {
    match args.mode {
        Mode::Timer => run_timer(&args),
        Mode::Overlay => run_overlay(&args),
        Mode::Heartbeat => run_heartbeat(&args),
        Mode::Runner | Mode::Kit => run_orchestrator(&args),
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
    let pmset_held = pmset_assertions();
    let shows_reason = pmset_shows(&pmset_held, &held.reason);
    let shows_system = pmset_shows(&pmset_held, SYSTEM_ASSERTION);
    let shows_display = pmset_shows(&pmset_held, DISPLAY_ASSERTION);
    let caffeinate = caffeinate_control();
    let timer = match sample_timers(args) {
        Ok(timer) => timer,
        Err(error) => TimerReport {
            interval_ms: args.interval_ms,
            duration_ms: args.sample_ms,
            overlay_down_count: None,
            overlay_up_count: None,
            overlay_ordered_front: false,
            overlay_display_count: display_count,
            slowed: false,
            sentence: error.clone(),
            error: Some(error),
        },
    };
    held.release();
    let pmset_after = pmset_assertions();
    let after_shows = pmset_shows(&pmset_after, &held.reason);
    let assertions = AssertionReport {
        names_compiled: names,
        reason: held.reason.clone(),
        created: held.created,
        released: held.released,
        pmset_while_held: scrub(&pmset_held),
        pmset_after_release: scrub(&pmset_after),
        pmset_shows_reason: shows_reason,
        pmset_shows_system: shows_system,
        pmset_shows_display: shows_display,
        pmset_after_shows_reason: after_shows,
        error: held.error.clone(),
    };
    let (lid_on_power, lid_external, power_key) = hardware_rows(args);
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
            assertions.created && assertions.released,
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
    if failure_reasons(&report).is_empty() {
        0
    } else {
        1
    }
}

fn sample_timers(args: &Args) -> Result<TimerReport, String> {
    let exe = current_exe()?;
    let down = run_child_timer(&exe, args)?;
    let (up, overlay_front, overlay_displays) = run_child_timer_with_overlay(&exe, args)?;
    let (slowed, sentence) = match (down, up) {
        (Some(down), Some(up)) => slowed_sentence(down, up),
        _ => (
            false,
            "Timer counts are incomplete, so slowing is not claimed.".to_string(),
        ),
    };
    Ok(TimerReport {
        interval_ms: args.interval_ms,
        duration_ms: args.sample_ms,
        overlay_down_count: down,
        overlay_up_count: up,
        overlay_ordered_front: overlay_front,
        overlay_display_count: overlay_displays,
        slowed,
        sentence,
        error: None,
    })
}

fn run_child_timer(exe: &Path, args: &Args) -> Result<Option<u64>, String> {
    let path = args.output_dir.join("timer-down.json");
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
            "timer-down.json",
        ],
    )?;
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("timer overlay-down exited {status}"));
    }
    read_count(&path)
}

fn run_child_timer_with_overlay(
    exe: &Path,
    args: &Args,
) -> Result<(Option<u64>, bool, i32), String> {
    let overlay_json = args.output_dir.join("overlay.json");
    let timer_json = args.output_dir.join("timer-up.json");
    let out = args.output_dir.to_str().unwrap_or(".").to_string();
    let sample = args.sample_ms.to_string();
    let interval = args.interval_ms.to_string();
    let mut overlay = spawn_role(
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
            "timer-up.json",
        ],
    )?;
    let timer_status = timer.wait().map_err(|error| error.to_string())?;
    let _ = overlay.kill();
    let _ = overlay.wait();
    if !timer_status.success() {
        return Err(format!("timer overlay-up exited {timer_status}"));
    }
    let count = read_count(&timer_json)?;
    let (front, displays) = read_overlay(&overlay_json);
    Ok((count, front, displays))
}

fn spawn_role(exe: &Path, extra: &[&str]) -> Result<Child, String> {
    Command::new(exe)
        .args(extra)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| error.to_string())
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
    let started = Instant::now();
    let limit = Duration::from_millis(args.sample_ms);
    let step = Duration::from_millis(args.interval_ms.max(1));
    let mut ticks = 0_u64;
    while started.elapsed() < limit && !STOP.load(Ordering::Relaxed) {
        match ffi::TimerWindow::tick() {
            Ok(value) => ticks = value,
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        }
        thread::sleep(step);
    }
    let name = args.count_file.as_deref().unwrap_or("timer-down.json");
    let path = args.output_dir.join(name);
    if let Err(error) = write_json(&path, &TickFile { ticks }) {
        eprintln!("{error}");
        return 1;
    }
    0
}

fn run_overlay(args: &Args) -> i32 {
    install_stop_handler();
    let overlay = ffi::Overlay::open();
    let path = args.output_dir.join("overlay.json");
    let marker = OverlayFile {
        ordered_front: overlay.ordered_front,
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
    while started.elapsed() < limit && !STOP.load(Ordering::Relaxed) {
        ffi::Overlay::pump(50);
        if ffi::Overlay::escape_seen() {
            break;
        }
    }
    0
}

fn run_heartbeat(args: &Args) -> i32 {
    install_stop_handler();
    let name = args.count_file.as_deref().unwrap_or("heartbeat.txt");
    let path = args.output_dir.join(name);
    let started = Instant::now();
    let limit = Duration::from_millis(args.sample_ms.max(30_000));
    while started.elapsed() < limit && !STOP.load(Ordering::Relaxed) {
        let now = wall_ms();
        if let Err(error) = append_line(&path, &now.to_string()) {
            eprintln!("{error}");
            return 1;
        }
        thread::sleep(Duration::from_millis(500));
    }
    0
}

fn hardware_rows(args: &Args) -> (HardwareRow, HardwareRow, HardwareRow) {
    if args.mode == Mode::Runner {
        return (
            runner_hardware_row("lid close on power"),
            runner_hardware_row("lid close with an external display"),
            runner_hardware_row("power key"),
        );
    }
    let lid = record_lid(
        args,
        "lid-on-power",
        args.lid_on_power.as_deref(),
        &[
            "STEP lid-closed-on-power",
            "Leave the Mac on power. Unplug the external display if one is attached and you can do that safely.",
            "Stay next to the machine. Close the lid. After about 20 seconds, open it.",
            "This kit does not change the lid-close action and does not lock the session.",
            "Type slept, awake, or skip. Do not type no-lid.",
        ],
        laptop_lid_row,
        900_000,
    );
    let external = record_lid(
        args,
        "lid-external",
        args.lid_external.as_deref(),
        &[
            "STEP lid-closed-with-external-display",
            "If you use an external display, plug it in and wait until it shows the desktop.",
            "Close the lid, wait about 20 seconds, then open it. Clamshell may stay awake; that is the measurement.",
            "Type slept, awake, no-external-display, or skip.",
        ],
        lid_external_row,
        900_000,
    );
    let power = record_choice(
        args.power_key.as_deref(),
        &[
            "STEP power-key",
            "Tap the power key once. Do not hold it. Holding it can force a shutdown.",
            "Type slept, restart-panel, nothing, or skip.",
        ],
        power_key_row,
        "power key tap",
    );
    (lid, external, power)
}

fn record_lid(
    args: &Args,
    stamp: &str,
    preset: Option<&str>,
    steps: &[&str],
    parse: fn(&str) -> Result<String, String>,
    wait_ms: u64,
) -> HardwareRow {
    for step in steps {
        println!("{step}");
    }
    let _ = io::stdout().flush();
    let exe = match current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            return hardware_row("error".to_string(), None, None, &error);
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
    let started = Instant::now();
    let choice = match preset {
        Some(value) => value.to_string(),
        None => read_choice(&["slept", "awake", "no-external-display", "skip"]),
    };
    if let Some(mut child) = child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let gap = heartbeat_gap(&beat, started.elapsed());
    match parse(&choice) {
        Ok(outcome) => hardware_row(
            "recorded".to_string(),
            Some(outcome),
            gap,
            "operator row plus heartbeat gap if the process paused",
        ),
        Err(error) => hardware_row("error".to_string(), None, gap, &error),
    }
}

fn record_choice(
    preset: Option<&str>,
    steps: &[&str],
    parse: fn(&str) -> Result<String, String>,
    note: &str,
) -> HardwareRow {
    for step in steps {
        println!("{step}");
    }
    let _ = io::stdout().flush();
    let choice = match preset {
        Some(value) => value.to_string(),
        None => read_choice(&["slept", "restart-panel", "nothing", "skip"]),
    };
    match parse(&choice) {
        Ok(outcome) => hardware_row("recorded".to_string(), Some(outcome), None, note),
        Err(error) => hardware_row("error".to_string(), None, None, &error),
    }
}

fn read_choice(allowed: &[&str]) -> String {
    if !stdin_is_terminal() {
        return "needs-terminal".to_string();
    }
    print!("> ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) => "skip".to_string(),
        Ok(_) => {
            let value = line.trim().to_string();
            if allowed.contains(&value.as_str())
                || value == "needs-terminal"
                || value == "not-run-on-runner"
            {
                value
            } else {
                value
            }
        }
        Err(_) => "skip".to_string(),
    }
}

fn stdin_is_terminal() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

fn heartbeat_gap(path: &Path, elapsed: Duration) -> Option<u64> {
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
    let sample = elapsed.as_millis() as u64;
    if gap > sample.saturating_add(1500) {
        Some(gap)
    } else if gap > 2_000 {
        Some(gap)
    } else {
        Some(gap)
    }
}

fn caffeinate_control() -> CaffeinateReport {
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
        .arg("-i")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            thread::sleep(Duration::from_millis(400));
            let excerpt = pmset_assertions();
            let showed = excerpt.to_ascii_lowercase().contains("caffeinate");
            let _ = child.kill();
            let _ = child.wait();
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

fn pmset_assertions() -> String {
    match Command::new("pmset").args(["-g", "assertions"]).output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            if !output.stderr.is_empty() {
                text.push('\n');
                text.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            scrub(&text)
        }
        Err(error) => format!("pmset failed: {error}"),
    }
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

fn read_count(path: &Path) -> Result<Option<u64>, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let file: TickFile = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    Ok(Some(file.ticks))
}

fn read_overlay(path: &Path) -> (bool, i32) {
    let Ok(text) = fs::read_to_string(path) else {
        return (false, 0);
    };
    let Ok(file) = serde_json::from_str::<OverlayFile>(&text) else {
        return (false, 0);
    };
    (file.ordered_front, file.display_count)
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
        let handler = handle_stop as *const () as libc::sighandler_t;
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGINT, handler);
    }
}

#[derive(Serialize, serde::Deserialize)]
struct TickFile {
    ticks: u64,
}

#[derive(Serialize, serde::Deserialize)]
struct OverlayFile {
    ordered_front: bool,
    display_count: i32,
    error: Option<String>,
}
