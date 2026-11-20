//! Orchestrator. Children never hold the tap.

use std::fs;
use std::io::{ErrorKind, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::args::{Args, Mode};
use crate::classify::{POST_SOURCES, classification_verdict, disable_event_name};
use crate::mac::ffi::{self, TapGuard};
use crate::plan;
use crate::report::{
    EventRow, FallbackRow, KIT_ID, KitReport, SecureRow, event_row, exit_nonzero, recorded_event,
    render, runner_skip,
};
use crate::scrub::{contains_serial, scrub_model};
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
            ffi::tap_release();
            code
        }
        Err(_) => {
            ffi::tap_release();
            let _ = write_crash_report(&output_dir, "probe panicked");
            1
        }
    }
}

fn dispatch_inner(args: &Args) -> i32 {
    match args.mode {
        Mode::ClassifyTarget => ffi::classify_target(&args.output_dir),
        Mode::ClassifyPost => {
            let Some(source) = args.post_source.as_deref() else {
                eprintln!("--classify-post needs a value");
                return 2;
            };
            ffi::classify_post(source, &args.output_dir)
        }
        Mode::Runner | Mode::Kit => run_orchestrator(args),
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
    let version = read_version();
    let inventory = ffi::hid_inventory();
    let mut interrupted = stop_requested();
    let mut no_input = false;
    let mut probe_crash = None;

    let tap = if interrupted {
        None
    } else {
        match TapGuard::create() {
            Ok(guard) => Some(guard),
            Err(error) => {
                probe_crash = Some(error);
                None
            }
        }
    };
    let tap_created = tap.is_some();

    let exe = current_exe().ok();
    let mut target = if tap_created && !interrupted {
        exe.as_ref().and_then(|path| start_target(path, args).ok())
    } else {
        None
    };
    wait_target_ready(&args.output_dir);

    let physical_key;
    let physical_click;
    let physical_scroll;
    let usb_keyboard;
    let secure;
    if kit && tap_created && !interrupted {
        physical_key = wait_physical(
            args,
            "key",
            "Type one key on the built-in keyboard. Type skip to skip.",
            plan::physical_key(inventory.keyboards),
        );
        physical_click = wait_physical(
            args,
            "click",
            "Click the trackpad or mouse once. Type skip to skip.",
            plan::physical_click(inventory.pointing),
        );
        physical_scroll = wait_physical(
            args,
            "scroll",
            "Scroll the trackpad once. Type skip to skip.",
            plan::physical_scroll(
                plan::trackpad_present(inventory.trackpad),
                inventory.pointing,
            ),
        );
        no_input = is_no_input(&physical_key)
            || is_no_input(&physical_click)
            || is_no_input(&physical_scroll);
        interrupted = interrupted || stop_requested();
        secure = if interrupted || no_input {
            skipped_secure(if interrupted {
                "interrupted"
            } else {
                "no-input"
            })
        } else {
            wait_secure(args)
        };
        usb_keyboard = if interrupted || no_input {
            event_row(
                if interrupted {
                    "interrupted"
                } else {
                    "no-input"
                },
                Some(if interrupted {
                    "interrupted"
                } else {
                    "no-input"
                }),
                "usb",
                "skipped",
                if interrupted {
                    "interrupted"
                } else {
                    "no-input"
                },
            )
        } else {
            wait_usb(args, inventory.keyboards)
        };
        no_input = no_input || is_no_input(&usb_keyboard) || secure.detection_method == "no-input";
    } else if kit && !tap_created {
        physical_key = event_row(
            "not-applicable",
            Some("no-tap"),
            "key",
            "session tap was not created",
            "pre-prompt",
        );
        physical_click = event_row(
            "not-applicable",
            Some("no-tap"),
            "click",
            "session tap was not created",
            "pre-prompt",
        );
        physical_scroll = event_row(
            "not-applicable",
            Some("no-tap"),
            "scroll",
            "session tap was not created",
            "pre-prompt",
        );
        usb_keyboard = event_row(
            "not-applicable",
            Some("no-tap"),
            "usb",
            "session tap was not created",
            "pre-prompt",
        );
        secure = skipped_secure("no-tap");
    } else {
        physical_key = runner_skip("key", "A physical key");
        physical_click = runner_skip("click", "A physical click");
        physical_scroll = runner_skip("scroll", "A trackpad scroll");
        usb_keyboard = runner_skip("usb", "A spare USB keyboard");
        secure = skipped_secure("not-run-on-runner");
    }

    interrupted = interrupted || stop_requested();
    let posts = if tap_created && !interrupted {
        if let Some(exe) = exe.as_ref() {
            second_process_posts(exe, args)
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let (slow, marker) = if tap_created {
        (
            FallbackRow {
                status: "not-applicable".into(),
                outcome: Some("not-run".into()),
                disable_event: None,
                dead_ms: None,
                healthy_ms: None,
                note: "m03 already measured timeout when a tap can be created".into(),
            },
            FallbackRow {
                status: "not-applicable".into(),
                outcome: Some("not-run".into()),
                disable_event: None,
                dead_ms: None,
                healthy_ms: None,
                note: "m03 already measured the marker when a tap can be created".into(),
            },
        )
    } else {
        (
            FallbackRow {
                status: "recorded".into(),
                outcome: Some("no-tap".into()),
                disable_event: Some("none".into()),
                dead_ms: None,
                healthy_ms: None,
                note: "tap was not created; timeout and marker stay on this kit".into(),
            },
            FallbackRow {
                status: "recorded".into(),
                outcome: Some("no-tap".into()),
                disable_event: None,
                dead_ms: None,
                healthy_ms: None,
                note: "tap was not created; timeout and marker stay on this kit".into(),
            },
        )
    };

    stop_target(&mut target, &args.output_dir);
    drop(tap);
    ffi::tap_release();
    let tap_released = ffi::tap_released();

    let physical_pairs: Vec<(i64, i64)> = [&physical_key, &physical_click, &physical_scroll]
        .into_iter()
        .filter(|row| row.status == "recorded" && row.outcome.as_deref() == Some("recorded"))
        .filter_map(|row| Some((row.state_id?, row.source_pid?)))
        .collect();
    let posted_pairs: Vec<(i64, i64)> = posts
        .iter()
        .filter_map(|row| Some((row.state_id?, row.source_pid?)))
        .collect();
    let (classification, classification_rule) =
        classification_verdict(&physical_pairs, &posted_pairs);
    let m14_named = classification == "pending-physical" || classification_rule.contains("m14");
    let serial_collected = contains_serial(&version.hw_model);
    let report = KitReport {
        kit: KIT_ID.to_string(),
        mode: if kit { "kit" } else { "runner" }.to_string(),
        version,
        tap_created,
        tap_released,
        tap_option: "listen-only".into(),
        keyboard_count: inventory.keyboards,
        pointing_count: inventory.pointing,
        trackpad: plan::trackpad_present(inventory.trackpad),
        physical_key,
        physical_click,
        physical_scroll,
        posts,
        secure,
        usb_keyboard,
        slow,
        marker,
        classification: classification.to_string(),
        classification_rule: classification_rule.to_string(),
        m14_named,
        serial_collected,
        locked_session: false,
        restored: tap_released,
        interrupted,
        no_input,
        probe_crash,
    };
    let text = render(&report);
    crate::report::write_stdout(&text);
    if let Err(error) = write_outputs(&args.output_dir, &report, &text) {
        eprintln!("{error}");
        ffi::tap_release();
        return 1;
    }
    crate::report::write_stdout(&format!("Wrote {}", args.output_dir.display()));
    crate::report::write_stdout("Send result.json and result.txt from that folder.");
    if exit_nonzero(&report) { 1 } else { 0 }
}

fn is_no_input(row: &EventRow) -> bool {
    row.status == "no-input" || row.outcome.as_deref() == Some("no-input")
}

fn skipped_secure(reason: &str) -> SecureRow {
    SecureRow {
        status: reason.to_string(),
        outcome: Some(reason.to_string()),
        disable_event: "none".into(),
        secure_enabled: "unread".into(),
        window_received: None,
        note: reason.to_string(),
        detection_method: reason.to_string(),
    }
}

fn wait_physical(
    args: &Args,
    kind: &str,
    instruction: &str,
    skip: Option<plan::SetupSkip>,
) -> EventRow {
    if stop_requested() {
        return event_row(
            "interrupted",
            Some("interrupted"),
            kind,
            "interrupted",
            "interrupted",
        );
    }
    if let Some(skip) = skip {
        return event_row(
            "recorded",
            Some(skip.outcome),
            kind,
            skip.note,
            "pre-prompt",
        );
    }
    if !stdin_is_terminal() {
        return event_row(
            "no-input",
            Some("no-input"),
            kind,
            "stdin closed before the prompt; results still written",
            "no-input",
        );
    }
    crate::report::write_stdout(instruction);
    crate::report::write_stdout(">");
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut pending = String::new();
    loop {
        if stop_requested() {
            return event_row(
                "interrupted",
                Some("interrupted"),
                kind,
                "interrupted",
                "interrupted",
            );
        }
        match poll_line(&mut pending, 50) {
            LineRead::Eof => {
                return event_row(
                    "no-input",
                    Some("no-input"),
                    kind,
                    "stdin closed; results still written",
                    "no-input",
                );
            }
            LineRead::Line(line) => {
                if line.eq_ignore_ascii_case("skip") {
                    return event_row("skip", Some("skip"), kind, "operator skip", "operator-skip");
                }
            }
            LineRead::Pending => {}
        }
        let wait = ffi::tap_wait(kind, 200);
        if wait.saw {
            let before = ffi::read_target_count(&args.output_dir);
            ffi::pump(150);
            let after = ffi::read_target_count(&args.output_dir);
            return recorded_event(
                kind,
                wait.state_id,
                wait.source_pid,
                after > before && before >= 0,
                true,
                None,
                None,
            );
        }
        if wait.disable_type != 0 {
            return event_row(
                "fail",
                Some("tap-disabled"),
                kind,
                disable_event_name(wait.disable_type),
                "disable-event",
            );
        }
        if Instant::now() >= deadline {
            return event_row(
                "not-detected",
                Some("not-detected"),
                kind,
                "no matching event arrived before the timeout",
                "listen-only-tap",
            );
        }
    }
}

fn wait_secure(args: &Args) -> SecureRow {
    crate::report::write_stdout(
        "Focus a password field (Safari, System Settings, or any secure field). Type one character there. Type skip if you cannot.",
    );
    crate::report::write_stdout(">");
    let enabled = match ffi::secure_enabled() {
        Some(true) => "true",
        Some(false) => "false",
        None => "unread",
    };
    if !stdin_is_terminal() {
        return skipped_secure("no-input");
    }
    let before = ffi::read_target_count(&args.output_dir);
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut pending = String::new();
    let mut quiet = true;
    loop {
        if stop_requested() {
            return skipped_secure("interrupted");
        }
        match poll_line(&mut pending, 50) {
            LineRead::Eof => return skipped_secure("no-input"),
            LineRead::Line(line) if line.eq_ignore_ascii_case("skip") => {
                return skipped_secure("skip");
            }
            LineRead::Line(_) | LineRead::Pending => {}
        }
        let wait = ffi::tap_wait("any-key", 200);
        if wait.disable_type != 0 {
            let name = disable_event_name(wait.disable_type);
            let after = ffi::read_target_count(&args.output_dir);
            return SecureRow {
                status: "recorded".into(),
                outcome: Some("disabled-by-user-input".into()),
                disable_event: name.into(),
                secure_enabled: enabled.into(),
                window_received: Some(after > before && before >= 0),
                note: name.into(),
                detection_method: "disable-event".into(),
            };
        }
        if wait.saw {
            quiet = false;
            let after = ffi::read_target_count(&args.output_dir);
            return SecureRow {
                status: "recorded".into(),
                outcome: Some("still-seeing".into()),
                disable_event: "none".into(),
                secure_enabled: enabled.into(),
                window_received: Some(after > before && before >= 0),
                note: "tap still saw a key while a password field was requested".into(),
                detection_method: "listen-only-tap".into(),
            };
        }
        if Instant::now() >= deadline {
            let after = ffi::read_target_count(&args.output_dir);
            return SecureRow {
                status: "recorded".into(),
                outcome: Some(if quiet { "quiet" } else { "not-detected" }),
                disable_event: "none".into(),
                secure_enabled: enabled.into(),
                window_received: Some(after > before && before >= 0),
                note: "no key and no disable event during the wait".into(),
                detection_method: "listen-only-tap".into(),
            };
        }
    }
}

fn wait_usb(args: &Args, keyboards_now: i32) -> EventRow {
    if let Some(skip) = plan::usb_keyboard(keyboards_now, keyboards_now) {
        crate::report::write_stdout(
            "If you have a spare USB keyboard, plug it in now and type skip if you do not.",
        );
        crate::report::write_stdout(">");
        if !stdin_is_terminal() {
            return event_row(
                "recorded",
                Some(skip.outcome),
                "usb",
                skip.note,
                "pre-prompt",
            );
        }
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut pending = String::new();
        loop {
            if stop_requested() {
                return event_row(
                    "interrupted",
                    Some("interrupted"),
                    "usb",
                    "interrupted",
                    "interrupted",
                );
            }
            match poll_line(&mut pending, 50) {
                LineRead::Eof => {
                    return event_row(
                        "recorded",
                        Some(skip.outcome),
                        "usb",
                        skip.note,
                        "pre-prompt",
                    );
                }
                LineRead::Line(line) if line.eq_ignore_ascii_case("skip") || line.is_empty() => {
                    return event_row(
                        "recorded",
                        Some(skip.outcome),
                        "usb",
                        skip.note,
                        "pre-prompt",
                    );
                }
                LineRead::Line(_) | LineRead::Pending => {}
            }
            let later = ffi::hid_inventory();
            if later.keyboards >= 2 {
                break;
            }
            if Instant::now() >= deadline {
                return event_row(
                    "recorded",
                    Some(skip.outcome),
                    "usb",
                    skip.note,
                    "pre-prompt",
                );
            }
            ffi::pump(200);
        }
    }
    wait_physical(
        args,
        "key",
        "Type on the USB keyboard while the tap is up. Type skip to skip.",
        None,
    )
}

fn second_process_posts(exe: &Path, args: &Args) -> Vec<EventRow> {
    let mut rows = Vec::new();
    for source in POST_SOURCES {
        let kind = match *source {
            "kCGEventSourceStatePrivate" => "post-private",
            "kCGEventSourceStateCombinedSessionState" => "post-combined",
            _ => "post-hid",
        };
        let mut built = None;
        for _ in 0..5 {
            let last = args.output_dir.join("last-post.txt");
            let _ = fs::remove_file(&last);
            let before = ffi::read_target_count(&args.output_dir);
            ffi::clear_wait();
            match Command::new(exe)
                .args([
                    "--classify-post",
                    source,
                    "--output-dir",
                    &args.output_dir.to_string_lossy(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    let start = Instant::now();
                    loop {
                        ffi::pump(50);
                        if ffi::wait_saw() {
                            break;
                        }
                        match child.try_wait() {
                            Ok(Some(_)) => {
                                let until = Instant::now() + Duration::from_millis(600);
                                while Instant::now() < until && !ffi::wait_saw() {
                                    ffi::pump(50);
                                }
                                break;
                            }
                            Ok(None) if start.elapsed() > Duration::from_secs(5) => {
                                let _ = child.kill();
                                let _ = child.wait();
                                break;
                            }
                            Ok(None) => {}
                            Err(_) => {
                                let _ = child.wait();
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    built = Some(event_row(
                        "fail",
                        Some("error"),
                        kind,
                        &error.to_string(),
                        "second-process",
                    ));
                    break;
                }
            }
            let wait = ffi::last_event();
            ffi::pump(150);
            let after = ffi::read_target_count(&args.output_dir);
            let (state_id, source_pid, poster_pid) = read_last_post(&last);
            let tap_saw = wait.saw;
            let state = if tap_saw {
                wait.state_id
            } else {
                state_id.unwrap_or(0)
            };
            let pid = if tap_saw {
                wait.source_pid
            } else {
                source_pid.unwrap_or(0)
            };
            built = Some(recorded_event(
                kind,
                state,
                pid,
                after > before && before >= 0,
                tap_saw,
                Some("second-process"),
                poster_pid,
            ));
            if tap_saw {
                break;
            }
        }
        if let Some(row) = built {
            rows.push(row);
        }
    }
    rows
}

fn read_last_post(path: &Path) -> (Option<i64>, Option<i64>, Option<i64>) {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut state = None;
    let mut pid = None;
    let mut poster = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("state_id=") {
            state = value.parse().ok();
        }
        if let Some(value) = line.strip_prefix("source_pid=") {
            pid = value.parse().ok();
        }
        if let Some(value) = line.strip_prefix("poster_pid=") {
            poster = value.parse().ok();
        }
    }
    (state, pid, poster)
}

fn start_target(exe: &Path, args: &Args) -> Result<Child, String> {
    let _ = fs::remove_file(args.output_dir.join("target.stop"));
    let _ = fs::remove_file(args.output_dir.join("target-state.txt"));
    Command::new(exe)
        .args([
            "--classify-target",
            "--output-dir",
            &args.output_dir.to_string_lossy(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())
}

fn wait_target_ready(dir: &Path) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if ffi::read_target_count(dir) >= 0 {
            return;
        }
        ffi::pump(50);
    }
}

fn stop_target(child: &mut Option<Child>, dir: &Path) {
    let _ = fs::write(dir.join("target.stop"), "stop\n");
    if let Some(child) = child.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn install_stop_handler() {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handle_stop as *const () as usize;
        action.sa_flags = 0;
        libc::sigemptyset(&mut action.sa_mask);
        let _ = libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
        let _ = libc::sigaction(libc::SIGHUP, &action, std::ptr::null_mut());
        let _ = libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
    }
}

enum LineRead {
    Line(String),
    Pending,
    Eof,
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

fn stdin_is_terminal() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
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
    version.hw_model = scrub_model(&ffi::hw_model());
    version
}

fn current_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|error| error.to_string())
}

fn write_outputs(dir: &Path, report: &KitReport, text: &str) -> Result<(), String> {
    fs::write(dir.join("result.txt"), text).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(report).map_err(|error| error.to_string())?;
    fs::write(dir.join("result.json"), json + "\n").map_err(|error| error.to_string())?;
    Ok(())
}

fn write_crash_report(dir: &Path, message: &str) -> Result<(), String> {
    let _ = fs::create_dir_all(dir);
    ffi::tap_release();
    let body = format!(
        "kit={KIT_ID}\nmode=unknown\ntap-released={}\nprobe-crash={message}\nresult=fail\n",
        ffi::tap_released()
    );
    fs::write(dir.join("result.txt"), body).map_err(|error| error.to_string())?;
    Ok(())
}

#[allow(dead_code)]
fn drain_stdin() {
    unsafe {
        libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
    }
}

#[allow(dead_code)]
fn ignore_io(error: ErrorKind) -> bool {
    error == ErrorKind::Interrupted
}
