//! Orchestrator, launchd child, and the root tap child.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::args::{Args, Mode};
use crate::guard::service_install_allowed;
use crate::labels::{CAPTURE_APIS, SCREEN_CAPTURE_ENTRY};
use crate::mac::{ffi, launchd};
use crate::png::png_size;
use crate::report::{
    CaptureReport, ClassifyReport, JobMarker, ProofReport, SharingReport, SharingWindowReport,
    TapReport, VersionReport, WindowPlan, WindowReport, capture_unavailable, failure_reasons,
    job_window_plan, render, sharing_sample, tap_process_miss, tap_sudo_failed,
    window_process_miss,
};
use crate::text::one_line;
use crate::version::parse_sw_vers;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_stop(_signal: i32) {
    STOP.store(true, Ordering::Relaxed);
}

pub fn dispatch(args: Args) -> i32 {
    match args.mode {
        Mode::Job => run_job(&args),
        Mode::TapOnly => run_tap(&args),
        Mode::WindowOnly => run_window(&args),
        Mode::CaptureOnly => run_capture(&args),
        Mode::ActiveTap => run_active(&args),
        Mode::M03 => run_m03(&args),
        Mode::ClassifyTarget => ffi::classify_target(&args.output_dir),
        Mode::ClassifyPost => {
            let Some(source) = args.post_source.as_deref() else {
                eprintln!("--classify-post needs a value");
                return 2;
            };
            ffi::classify_post(source, &args.output_dir)
        }
        Mode::Smoke => crate::smoke::run(&args),
        Mode::SharingOnly => run_sharing(&args),
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
    let exe = match current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    eprintln!("macos desktop proof: window");
    let desktop = child_desktop(&exe, &args.output_dir);
    eprintln!("macos desktop proof: event taps");
    let (session, session_crash) = child_tap(&exe, &args.output_dir, "kCGSessionEventTap", "user");
    let (hid_user, hid_crash) = child_tap(&exe, &args.output_dir, "kCGHIDEventTap", "user");
    let (hid_root, root_crash) = root_hid_tap(&exe, &args.output_dir);
    eprintln!("macos desktop proof: active tap");
    let (active, active_crash) = child_active(&exe, &args.output_dir);
    eprintln!("macos desktop proof: posted events, tap disable, marker");
    let (classify, classify_crash) = child_m03(&exe, &args.output_dir);
    eprintln!("macos desktop proof: sharingType");
    let (sharing, sharing_crash) = child_sharing(&exe, &args.output_dir);
    eprintln!("macos desktop proof: launchd");
    let jobs = launchd::exercise(&exe, &args.output_dir);
    let mut crashes = Vec::new();
    for crash in [
        desktop.crash,
        sharing_crash,
        session_crash,
        hid_crash,
        root_crash,
        active_crash,
        classify_crash,
    ]
    .into_iter()
    .flatten()
    {
        crashes.push(crash);
    }
    let report = ProofReport {
        runner_image: args.runner_image.clone(),
        version,
        window: Some(desktop.window),
        taps: vec![session, hid_user, hid_root, active],
        captures: desktop.captures,
        jobs,
        classify: Some(classify),
        sharing: Some(sharing),
        probe_crash: if crashes.is_empty() {
            None
        } else {
            Some(crashes.join("; "))
        },
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
    let plan = job_window_plan(graphic, info.status);
    let (window_ordered_front, window_error) = match &plan {
        WindowPlan::Skip { error } => (false, Some(error.clone())),
        WindowPlan::Prove => (false, Some("NSWindow proof started".to_string())),
    };
    let mut marker = JobMarker {
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
    if matches!(plan, WindowPlan::Prove) {
        eprintln!("stage=NSWindow");
        let window = ffi::prove_window();
        marker.window_ordered_front = window.ordered_front;
        marker.window_error = window.error;
        if let Err(error) = write_json(&path, &marker) {
            eprintln!("{error}");
            return 1;
        }
    }
    wait_for_stop();
    0
}

fn run_window(args: &Args) -> i32 {
    let window = ffi::window_open();
    let path = args.output_dir.join("window.json");
    if let Err(error) = write_json(&path, &window) {
        eprintln!("{error}");
        ffi::window_close();
        return 1;
    }
    for api in CAPTURE_APIS {
        eprintln!("stage={api}");
        let report = capture_api(api, &args.output_dir);
        let path = args.output_dir.join(format!("capture-{api}.json"));
        if let Err(error) = write_json(&path, &report) {
            eprintln!("{error}");
            ffi::window_close();
            return 1;
        }
    }
    ffi::window_close();
    0
}

fn capture_api(api: &str, output_dir: &Path) -> CaptureReport {
    match api {
        "CGWindowListCreateImage" => ffi::prove_cg_window_list(),
        "CGDisplayCreateImage" => ffi::prove_cg_display(),
        "screencapture" => prove_screencapture(output_dir),
        "ScreenCaptureKit" => ffi::prove_screen_capture_kit(),
        other => capture_unavailable(other, "unknown capture api"),
    }
}

fn run_sharing(args: &Args) -> i32 {
    eprintln!("stage=sharing-window");
    let mut window = ffi::sharing_open();
    let mut samples = capture_sharing("default", &args.output_dir);
    eprintln!("stage=sharing-none");
    window.sharing_none = ffi::sharing_set_none();
    samples.extend(capture_sharing("none", &args.output_dir));
    ffi::sharing_close();
    let report = SharingReport { window, samples };
    let path = args.output_dir.join("sharing.json");
    if let Err(error) = write_json(&path, &report) {
        eprintln!("{error}");
        return 1;
    }
    0
}

fn capture_sharing(sharing: &str, output_dir: &Path) -> Vec<crate::report::SharingSample> {
    let mut samples = Vec::new();
    for api in CAPTURE_APIS {
        eprintln!("stage=sharing-{sharing}-{api}");
        let report = capture_api(api, output_dir);
        samples.push(sharing_sample(sharing, &report));
    }
    samples
}

fn child_sharing(exe: &Path, output_dir: &Path) -> (SharingReport, Option<String>) {
    let path = output_dir.join("sharing.json");
    let _ = fs::remove_file(&path);
    let dir = path_arg(output_dir);
    let outcome = run_limited(exe, &["--sharing-only", "--output-dir", &dir], 90);
    show_stderr(&outcome.stderr);
    if let Some(report) = read_json::<SharingReport>(&path) {
        let crash = if outcome.success {
            None
        } else {
            Some(format!(
                "sharing probe process exited {}: {}",
                outcome.code,
                one_line(&outcome.stderr)
            ))
        };
        return (report, crash);
    }
    (
        SharingReport {
            window: SharingWindowReport {
                ordered_front: false,
                level: String::new(),
                collection: String::new(),
                ignores_mouse_events: false,
                shielding_called: false,
                sharing_default: String::new(),
                sharing_none: String::new(),
                display_image_api: String::new(),
                screen_capture_entry: SCREEN_CAPTURE_ENTRY.to_string(),
                error: Some(format!(
                    "NSWindow sharing probe process exited {}: {}",
                    outcome.code,
                    one_line(&outcome.stderr)
                )),
            },
            samples: Vec::new(),
        },
        Some(format!("sharing probe process exited {}", outcome.code)),
    )
}

fn run_capture(args: &Args) -> i32 {
    let Some(api) = args.capture_api.as_deref() else {
        eprintln!("--capture-only needs a value");
        return 2;
    };
    eprintln!("stage={api}");
    let report = capture_api(api, &args.output_dir);
    let path = args.output_dir.join(format!("capture-{api}.json"));
    if let Err(error) = write_json(&path, &report) {
        eprintln!("{error}");
        return 1;
    }
    0
}

struct Desktop {
    window: WindowReport,
    captures: Vec<CaptureReport>,
    crash: Option<String>,
}

fn child_desktop(exe: &Path, output_dir: &Path) -> Desktop {
    let window_path = output_dir.join("window.json");
    let _ = fs::remove_file(&window_path);
    for api in CAPTURE_APIS {
        let _ = fs::remove_file(output_dir.join(format!("capture-{api}.json")));
    }
    let dir = path_arg(output_dir);
    let outcome = run_limited(exe, &["--window-only", "--output-dir", &dir], 70);
    show_stderr(&outcome.stderr);
    let window = read_json::<WindowReport>(&window_path)
        .unwrap_or_else(|| window_process_miss(outcome.code, &outcome.stderr));
    let mut captures = Vec::new();
    for api in CAPTURE_APIS {
        let path = output_dir.join(format!("capture-{api}.json"));
        if let Some(report) = read_json::<CaptureReport>(&path) {
            captures.push(report);
        } else {
            captures.push(capture_unavailable(
                api,
                &format!("process exited {}: {}", outcome.code, outcome.stderr),
            ));
        }
    }
    let crash = if outcome.success {
        None
    } else {
        Some(format!(
            "desktop probe process exited {}: {}",
            outcome.code,
            one_line(&outcome.stderr)
        ))
    };
    Desktop {
        window,
        captures,
        crash,
    }
}

fn child_tap(
    exe: &Path,
    output_dir: &Path,
    location: &str,
    privilege: &str,
) -> (TapReport, Option<String>) {
    let path = output_dir.join(format!("tap-{location}.json"));
    let _ = fs::remove_file(&path);
    let dir = path_arg(output_dir);
    let outcome = run_limited(exe, &["--tap-only", location, "--output-dir", &dir], 20);
    show_stderr(&outcome.stderr);
    if outcome.success
        && let Some(mut tap) = read_json::<TapReport>(&path)
    {
        tap.location = location.to_string();
        tap.privilege = privilege.to_string();
        return (tap, None);
    }
    (
        tap_process_miss(location, privilege, outcome.code, &outcome.stderr),
        Some(format!(
            "{location} {privilege} process exited {}",
            outcome.code
        )),
    )
}

fn child_active(exe: &Path, output_dir: &Path) -> (TapReport, Option<String>) {
    let path = output_dir.join("tap-active.json");
    let _ = fs::remove_file(&path);
    let dir = path_arg(output_dir);
    let outcome = run_limited(exe, &["--active-tap", "--output-dir", &dir], 20);
    show_stderr(&outcome.stderr);
    if outcome.success
        && let Some(tap) = read_json::<TapReport>(&path)
    {
        return (tap, None);
    }
    let mut tap = crate::report::active_tap(
        crate::report::TapBits {
            created: false,
            callback_fired: false,
            accessibility_trusted: None,
            listen_preflight: None,
            prompt: None,
        },
        None,
        None,
        None,
        "",
    );
    tap.error = Some(format!(
        "kCGEventTapOptionDefault process exited {}: {}",
        outcome.code,
        one_line(&outcome.stderr)
    ));
    (
        tap,
        Some(format!("active tap process exited {}", outcome.code)),
    )
}

fn show_stderr(stderr: &str) {
    if stderr.is_empty() {
        return;
    }
    if stderr.ends_with('\n') {
        eprint!("{stderr}");
    } else {
        eprintln!("{stderr}");
    }
}

struct ChildOutcome {
    code: i32,
    success: bool,
    stderr: String,
}

fn run_limited(exe: &Path, args: &[&str], seconds: u64) -> ChildOutcome {
    let mut child = match Command::new(exe)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return ChildOutcome {
                code: -1,
                success: false,
                stderr: error.to_string(),
            };
        }
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                if let Some(mut pipe) = child.stdout.take() {
                    let mut stdout = String::new();
                    let _ = pipe.read_to_string(&mut stdout);
                }
                return ChildOutcome {
                    code: status.code().unwrap_or(139),
                    success: status.success(),
                    stderr,
                };
            }
            Ok(None) if start.elapsed() >= Duration::from_secs(seconds) => {
                let _ = child.kill();
                let _ = child.wait();
                return ChildOutcome {
                    code: 124,
                    success: false,
                    stderr: "timed out".to_string(),
                };
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(error) => {
                return ChildOutcome {
                    code: -1,
                    success: false,
                    stderr: error.to_string(),
                };
            }
        }
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
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
    eprintln!("stage=CGEventTapCreate {location}");
    let tap = ffi::prove_tap(location, privilege);
    let path = args.output_dir.join(format!("tap-{location}.json"));
    if let Err(error) = write_json(&path, &tap) {
        eprintln!("{error}");
        return 1;
    }
    0
}

fn run_m03(args: &Args) -> i32 {
    let exe = match current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    eprintln!("stage=m03");
    let classify = ffi::prove_m03(&exe, &args.output_dir);
    let path = args.output_dir.join("m03.json");
    if let Err(error) = write_json(&path, &classify) {
        eprintln!("{error}");
        return 1;
    }
    0
}

fn child_m03(exe: &Path, output_dir: &Path) -> (ClassifyReport, Option<String>) {
    let path = output_dir.join("m03.json");
    let dir = path_arg(output_dir);
    let outcome = run_limited(exe, &["--m03", "--output-dir", &dir], 45);
    show_stderr(&outcome.stderr);
    if outcome.success
        && let Some(classify) = read_json::<ClassifyReport>(&path)
    {
        return (classify, None);
    }
    (
        ClassifyReport {
            posts: Vec::new(),
            slow: crate::report::SlowObservation {
                disable_event: "none".to_string(),
                sleep_ms: 0,
                disable_wait_ms: 0,
                key_reached_during: false,
                key_reached_after: false,
                reenabled: false,
                error: Some(format!(
                    "m03 process exited {}: {}",
                    outcome.code,
                    one_line(&outcome.stderr)
                )),
            },
            marker: crate::report::MarkerObservation {
                dead_ms: 0,
                dead_misses: 0,
                healthy_ms: 0,
                healthy_seen: false,
                reenabled: false,
                error: Some(format!("m03 process exited {}", outcome.code)),
            },
            end_state: crate::classify::fail_closed_end().to_string(),
            tap_status: "missing".to_string(),
        },
        Some(format!("m03 process exited {}", outcome.code)),
    )
}

fn run_active(args: &Args) -> i32 {
    let tap = ffi::prove_active_tap();
    let path = args.output_dir.join("tap-active.json");
    if let Err(error) = write_json(&path, &tap) {
        eprintln!("{error}");
        return 1;
    }
    0
}

fn root_hid_tap(exe: &Path, output_dir: &Path) -> (TapReport, Option<String>) {
    let root_dir = output_dir.join("root-tap");
    if let Err(error) = fs::create_dir_all(&root_dir) {
        return (
            tap_sudo_failed("kCGHIDEventTap", &format!("output dir: {error}")),
            None,
        );
    }
    let destination = root_dir.join("tap-kCGHIDEventTap.json");
    let _ = fs::remove_file(&destination);
    let output = Command::new("sudo")
        .arg("-n")
        .arg(exe)
        .arg("--tap-only")
        .arg("kCGHIDEventTap")
        .arg("--output-dir")
        .arg(&root_dir)
        .output();
    match output {
        Err(error) => (tap_sudo_failed("kCGHIDEventTap", &error.to_string()), None),
        Ok(output) if !output.status.success() => {
            let detail = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
            show_stderr(&detail);
            let code = output.status.code().unwrap_or(139);
            if output.status.code().is_none() || code == 139 || code == 134 || code == 126 {
                (
                    tap_process_miss("kCGHIDEventTap", "root", code, &detail),
                    Some(format!("kCGHIDEventTap root process exited {code}")),
                )
            } else {
                (tap_sudo_failed("kCGHIDEventTap", &detail), None)
            }
        }
        Ok(output) => {
            show_stderr(&String::from_utf8_lossy(&output.stderr));
            match fs::read_to_string(&destination) {
                Ok(text) => match serde_json::from_str::<TapReport>(&text) {
                    Ok(mut tap) => {
                        tap.location = "kCGHIDEventTap".to_string();
                        tap.privilege = "root".to_string();
                        (tap, None)
                    }
                    Err(error) => (
                        tap_sudo_failed("kCGHIDEventTap", &format!("bad tap report: {error}")),
                        None,
                    ),
                },
                Err(error) => (
                    tap_sudo_failed("kCGHIDEventTap", &format!("no tap report: {error}")),
                    None,
                ),
            }
        }
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
                Some((width, height)) => match ffi::inspect_png(&path) {
                    Some((nonblack, marker)) => crate::report::capture_opened(
                        "screencapture",
                        width,
                        height,
                        nonblack,
                        marker,
                    ),
                    None => {
                        let mut report =
                            crate::report::capture_opened("screencapture", width, height, 0, 0);
                        report.nonblack_pixels = None;
                        report.marker_pixels = None;
                        report
                    }
                },
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
