//! Orchestrator, launchd child, and the root tap child.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::args::{Args, Mode};
use crate::guard::service_install_allowed;
use crate::mac::{ffi, launchd};
use crate::png::png_size;
use crate::report::{
    CaptureReport, JobMarker, ProofReport, TapReport, VersionReport, WindowPlan,
    capture_unavailable, failure_reasons, job_window_plan, render, tap_sudo_failed,
};
use crate::version::parse_sw_vers;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_stop(_signal: i32) {
    STOP.store(true, Ordering::Relaxed);
}

pub fn dispatch(args: Args) -> i32 {
    match args.mode {
        Mode::Job => run_job(&args),
        Mode::TapOnly => run_tap(&args),
        Mode::Orchestrator => run_orchestrator(&args),
    }
}

fn run_orchestrator(args: &Args) -> i32 {
    if !args.mode.installs_daemon() {
        eprintln!("only the orchestrator installs a daemon");
        return 2;
    }
    if let Err(error) = service_install_allowed(
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
        args.allow_local_service,
    ) {
        eprintln!("{error}");
        return 2;
    }
    if let Err(error) = fs::create_dir_all(&args.output_dir) {
        eprintln!("output dir: {error}");
        return 1;
    }
    let version = read_version();
    eprintln!("macos desktop proof: window");
    let window = ffi::prove_window();
    eprintln!("macos desktop proof: event taps");
    let session = ffi::prove_tap("kCGSessionEventTap", "user");
    let hid_user = ffi::prove_tap("kCGHIDEventTap", "user");
    let exe = match current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let hid_root = root_hid_tap(&exe, &args.output_dir);
    eprintln!("macos desktop proof: capture");
    let captures = vec![
        ffi::prove_cg_window_list(),
        ffi::prove_cg_display(),
        prove_screencapture(&args.output_dir),
        ffi::prove_screen_capture_kit(),
    ];
    eprintln!("macos desktop proof: launchd");
    let jobs = launchd::exercise(&exe, &args.output_dir);
    let report = ProofReport {
        runner_image: args.runner_image.clone(),
        version,
        window: Some(window),
        taps: vec![session, hid_user, hid_root],
        captures,
        jobs,
    };
    let text = render(&report);
    println!("{text}");
    let _ = io::stdout().flush();
    let path = args.output_dir.join("proof.json");
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

fn run_job(args: &Args) -> i32 {
    install_stop_handler();
    let kind = args.job_kind.as_deref().unwrap_or("agent");
    let info = ffi::session_info();
    let uid = unsafe { libc::getuid() };
    let pid = std::process::id();
    let graphic = info.status == 0 && info.graphic;
    let (window_ordered_front, window_error) = match job_window_plan(graphic, info.status) {
        WindowPlan::Skip { error } => (false, Some(error)),
        WindowPlan::Prove => {
            let window = ffi::prove_window();
            (window.ordered_front, window.error)
        }
    };
    let marker = JobMarker {
        pid,
        uid,
        graphic_session: graphic,
        window_ordered_front,
        window_error,
    };
    let path = args.output_dir.join(format!("{kind}-job.json"));
    if let Err(error) = write_json(&path, &marker) {
        eprintln!("{error}");
        return 1;
    }
    wait_for_stop();
    0
}

fn run_tap(args: &Args) -> i32 {
    let Some(location) = args.tap_location.as_deref() else {
        eprintln!("--tap-only needs a value");
        return 2;
    };
    let privilege = if unsafe { libc::geteuid() } == 0 {
        "root"
    } else {
        "user"
    };
    let tap = ffi::prove_tap(location, privilege);
    let path = args.output_dir.join("root-tap.json");
    if let Err(error) = write_json(&path, &tap) {
        eprintln!("{error}");
        return 1;
    }
    0
}

fn root_hid_tap(exe: &Path, output_dir: &Path) -> TapReport {
    let destination = output_dir.join("root-tap.json");
    let _ = fs::remove_file(&destination);
    let output = Command::new("sudo")
        .arg("-n")
        .arg(exe)
        .arg("--tap-only")
        .arg("kCGHIDEventTap")
        .arg("--output-dir")
        .arg(output_dir)
        .output();
    match output {
        Err(error) => tap_sudo_failed("kCGHIDEventTap", &error.to_string()),
        Ok(output) if !output.status.success() => {
            let detail = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
            tap_sudo_failed("kCGHIDEventTap", &detail)
        }
        Ok(_) => match fs::read_to_string(&destination) {
            Ok(text) => match serde_json::from_str::<TapReport>(&text) {
                Ok(mut tap) => {
                    tap.location = "kCGHIDEventTap".to_string();
                    tap.privilege = "root".to_string();
                    tap
                }
                Err(error) => {
                    tap_sudo_failed("kCGHIDEventTap", &format!("bad tap report: {error}"))
                }
            },
            Err(error) => tap_sudo_failed("kCGHIDEventTap", &format!("no tap report: {error}")),
        },
    }
}

fn prove_screencapture(dir: &Path) -> CaptureReport {
    let path = dir.join("screencapture.png");
    let _ = fs::remove_file(&path);
    let output = Command::new("screencapture")
        .args(["-x", "-t", "png"])
        .arg(&path)
        .output();
    let report = match output {
        Err(error) => capture_unavailable("screencapture", &format!("failed to spawn: {error}")),
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output.status.code().unwrap_or(-1);
            capture_unavailable(
                "screencapture",
                &ffi::prompt_after_capture(&format!("exit {code}: {stderr}")),
            )
        }
        Ok(_) => match fs::read(&path) {
            Err(error) => capture_unavailable("screencapture", &format!("no file: {error}")),
            Ok(bytes) => match png_size(&bytes) {
                Some((width, height)) => {
                    crate::report::capture_opened("screencapture", width, height)
                }
                None => capture_unavailable("screencapture", "file is not a PNG"),
            },
        },
    };
    let _ = fs::remove_file(&path);
    report
}

fn read_version() -> VersionReport {
    let sw = Command::new("sw_vers").output();
    let uname = Command::new("uname").arg("-m").output();
    let parsed = sw
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| parse_sw_vers(&String::from_utf8_lossy(&output.stdout)).ok());
    let uname_m = uname
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default();
    VersionReport {
        product_name: parsed
            .as_ref()
            .map(|value| value.product_name.clone())
            .unwrap_or_default(),
        product_version: parsed
            .as_ref()
            .map(|value| value.product_version.clone())
            .unwrap_or_default(),
        build_version: parsed.map(|value| value.build_version).unwrap_or_default(),
        uname_m,
    }
}

fn current_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|error| format!("current exe: {error}"))
}

fn install_stop_handler() {
    unsafe {
        let handler = handle_stop as *const () as libc::sighandler_t;
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGINT, handler);
    }
}

fn wait_for_stop() {
    let start = Instant::now();
    while !STOP.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(90) {
        thread::sleep(Duration::from_millis(200));
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, text)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| format!("rename {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o644));
    }
    Ok(())
}
