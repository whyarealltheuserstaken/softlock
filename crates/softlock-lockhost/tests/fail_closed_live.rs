//! Hosted-runner fail-closed cases. A normal `cargo test` returns immediately.
//! The fail-closed workflow sets `SOFTLOCK_FAIL_CLOSED_LIVE` to one case and
//! builds the service, Lock Host, and owner with `live-fail-closed`.
//! The build must not enable `simulated-physical`: that feature stubs the OS lock.

#![cfg(windows)]

use std::fs;
use std::os::windows::io::FromRawHandle;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_ACCESS_RIGHTS};

#[test]
fn live_fail_closed() {
    let Ok(case) = std::env::var("SOFTLOCK_FAIL_CLOSED_LIVE") else {
        return;
    };
    match softlock_windows::runner_may_lock() {
        Ok(()) => {}
        Err(reason) => panic!("refusing live fail-closed on this machine: {reason}"),
    }
    let root = std::env::temp_dir().join(format!("fail-closed-{case}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temp");
    let log = root.join("service.log");
    let pin = root.join("pin");
    fs::create_dir_all(&pin).expect("pin");
    let pipe = format!(r"\\.\pipe\softlock-live-{}-{}", std::process::id(), case);
    let stop_file = root.join("stop");
    let mark = root.join("os-lock.txt");
    let guard_file = root.join("guard-down");
    let hwnd_file = root.join("owner-hwnd.txt");
    let service = service_exe();
    let mut command = Command::new(&service);
    command
        .arg("--foreground")
        .env("SOFTLOCK_PIN_DIR", &pin)
        .env("SOFTLOCK_LOG", &log)
        .env("SOFTLOCK_PIPE", &pipe)
        .env("SOFTLOCK_DEV_APP", "1")
        .env("SOFTLOCK_STOP_FILE", &stop_file)
        .env("SOFTLOCK_OS_LOCK_MARK", &mark)
        .env("SOFTLOCK_FORCE_GUARD_DOWN", &guard_file)
        .env_remove("SOFTLOCK_STUB_OS_LOCK")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("service");
    let mut owner = None;
    if case == "tray-hard-lock" {
        owner = Some(spawn_owner(&pipe, &hwnd_file));
    }
    wait_until(Duration::from_secs(20), || transact(&pipe, hello()).is_ok());
    let mut saw_unlocked = false;
    let mut saw_locked = false;
    if case != "tray-hard-lock" {
        let response = transact(&pipe, lock_now()).expect("lock");
        assert!(
            response.contains("Accepted"),
            "{case} was not accepted: {response}"
        );
        let locked = wait_until(Duration::from_secs(15), || {
            let state = status_state(&transact(&pipe, status()).unwrap_or_default());
            if state.as_deref() == Some("Unlocked") {
                saw_unlocked = true;
            }
            if state.as_deref() == Some("Locked") {
                saw_locked = true;
            }
            saw_locked
        });
        assert!(locked, "{case} did not reach Locked");
        assert!(!saw_unlocked, "{case} read Unlocked before the fault");
        if matches!(case.as_str(), "kill-host" | "suspend-host" | "kill-service") {
            wait_until(Duration::from_secs(10), || host_pid().is_some());
        }
    }
    let host_before = host_pid();
    if case == "tray-hard-lock" {
        let ready = wait_until(Duration::from_secs(15), || hwnd_file.is_file());
        assert!(ready, "the owner window did not publish its handle");
    }
    let log_at = fs::metadata(&log)
        .map(|meta| meta.len() as usize)
        .unwrap_or(0);
    let _ = fs::remove_file(&mark);
    let started = Instant::now();
    match case.as_str() {
        "kill-host" => {
            let pid = host_before.expect("lock host");
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .output();
        }
        "suspend-host" => {
            suspend_process(host_before.expect("lock host"));
        }
        "kill-service" => {
            child.kill().expect("kill service");
            let _ = child.wait();
        }
        "stop-while-locked" => {
            fs::write(&stop_file, b"stop").expect("stop file");
        }
        "unhealthy-guard" => {
            fs::write(&guard_file, b"down").expect("guard file");
        }
        "tray-hard-lock" => post_tray_hard_lock(&hwnd_file),
        other => panic!("unknown case {other}"),
    }
    let mut confirmed_at = None;
    while started.elapsed() < Duration::from_secs(1) {
        if case != "kill-service"
            && let Ok(body) = transact(&pipe, status())
        {
            let state = status_state(&body);
            if state.as_deref() == Some("Unlocked") && case != "tray-hard-lock" {
                saw_unlocked = true;
            }
            if state.as_deref() == Some("Locked") {
                saw_locked = true;
            }
        }
        let fresh = fresh_log(&log, log_at);
        let marked = fs::read_to_string(&mark).unwrap_or_default();
        let os_locked = fresh.contains("result=confirmed") || marked.contains("confirmed");
        let host_done = case == "kill-service"
            && os_locked
            && host_before.is_some()
            && host_pid() != host_before;
        if (case != "kill-service" && os_locked) || host_done {
            confirmed_at = Some(started.elapsed());
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let elapsed = confirmed_at.unwrap_or_else(|| started.elapsed());
    let fresh = fresh_log(&log, log_at);
    println!("fail-closed case={case} elapsed_ms={}", elapsed.as_millis());
    let elapsed_us = fresh
        .split_whitespace()
        .find_map(|field| field.strip_prefix("elapsed_us="))
        .unwrap_or("0");
    let elapsed_us = if elapsed_us.chars().all(|ch| ch.is_ascii_digit()) {
        elapsed_us
    } else {
        "0"
    };
    fs::write(
        result_file(),
        format!(
            "{{\"case\":\"{case}\",\"confirmed\":{},\"elapsed_ms\":{},\"elapsed_us\":{elapsed_us},\"unlocked\":{saw_unlocked}}}\n",
            confirmed_at.is_some(),
            elapsed.as_millis()
        ),
    )
    .expect("result");
    if let Some(mut owner) = owner {
        let _ = owner.kill();
        let _ = owner.wait();
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = Command::new("taskkill")
        .args(["/IM", "softlock-lockhost.exe", "/F"])
        .output();
    let _ = Command::new("taskkill")
        .args(["/IM", "softlock-app.exe", "/F"])
        .output();
    assert!(
        confirmed_at.is_some(),
        "{case} did not confirm within 1s of the fault: {fresh}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "{case} took {} ms",
        elapsed.as_millis()
    );
    if case == "tray-hard-lock" {
        assert!(
            fresh.contains("result=confirmed")
                || fs::read_to_string(&mark)
                    .unwrap_or_default()
                    .contains("confirmed"),
            "tray hard lock was not confirmed: {fresh}"
        );
    } else {
        assert!(
            saw_locked || case == "kill-service",
            "{case} never read Locked"
        );
        assert!(!saw_unlocked, "{case} read Unlocked: {fresh}");
    }
}

fn hello() -> &'static str {
    r#"{"v":1,"id":1,"kind":"Hello","client_version":"0"}"#
}

fn lock_now() -> &'static str {
    r#"{"v":1,"id":2,"kind":"Lock","source":"App"}"#
}

fn status() -> &'static str {
    r#"{"v":1,"id":3,"kind":"GetStatus"}"#
}

fn status_state(json: &str) -> Option<String> {
    let key = "\"state\":\"";
    let start = json.find(key)? + key.len();
    let rest = json.get(start..)?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn fresh_log(path: &std::path::Path, from: usize) -> String {
    let bytes = fs::read(path).unwrap_or_default();
    let from = from.min(bytes.len());
    String::from_utf8_lossy(&bytes[from..]).into_owned()
}

fn result_file() -> PathBuf {
    let base = std::env::var("RUNNER_TEMP")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("GITHUB_WORKSPACE").map(PathBuf::from))
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let dir = base.join("softlock-fail-closed");
    fs::create_dir_all(&dir).expect("result dir");
    dir.join("result.json")
}

fn spawn_owner(pipe: &str, hwnd_file: &std::path::Path) -> std::process::Child {
    Command::new(app_exe())
        .arg("--owner-session")
        .env("SOFTLOCK_PIPE", pipe)
        .env("SOFTLOCK_NO_AUTOSTART", "1")
        .env("SOFTLOCK_OWNER_HWND", hwnd_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("owner session")
}

fn post_tray_hard_lock(hwnd_file: &std::path::Path) {
    let text = fs::read_to_string(hwnd_file).expect("hwnd");
    let raw: isize = text.trim().parse().expect("hwnd number");
    let posted = unsafe {
        windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(windows::Win32::Foundation::HWND(
                raw as *mut core::ffi::c_void,
            )),
            windows::Win32::UI::WindowsAndMessaging::WM_COMMAND,
            windows::Win32::Foundation::WPARAM(12),
            windows::Win32::Foundation::LPARAM(0),
        )
    };
    assert!(posted.is_ok(), "WM_COMMAND CMD_HARD_LOCK was not posted");
}

fn app_exe() -> PathBuf {
    if let Ok(path) = std::env::var("SOFTLOCK_APP_EXE") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/release/softlock-app.exe")
}

fn service_exe() -> PathBuf {
    if let Ok(path) = std::env::var("SOFTLOCK_SERVICE_EXE") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/release/softlock-service.exe")
}

fn host_pid() -> Option<u32> {
    let output = Command::new("tasklist")
        .args([
            "/FI",
            "IMAGENAME eq softlock-lockhost.exe",
            "/FO",
            "CSV",
            "/NH",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?;
    let mut parts = line.split(',');
    let _name = parts.next()?;
    let pid = parts.next()?.trim_matches('"');
    pid.parse().ok()
}

fn suspend_process(pid: u32) {
    unsafe {
        // PROCESS_SUSPEND_RESUME. Suspending every thread from the snapshot
        // missed the UI thread, so heartbeats kept the host healthy.
        let process = OpenProcess(PROCESS_ACCESS_RIGHTS(0x0800), false, pid).expect("open host");
        let status = NtSuspendProcess(process.0);
        let _ = CloseHandle(process);
        assert_eq!(status, 0, "suspend host returned {status}");
    }
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSuspendProcess(process: *mut core::ffi::c_void) -> i32;
}

fn transact(pipe: &str, json: &str) -> Result<String, String> {
    let mut frame = (json.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(json.as_bytes());
    let wide: Vec<u16> = pipe.encode_utf16().chain(std::iter::once(0)).collect();
    let handle = unsafe {
        windows::Win32::Storage::FileSystem::CreateFileW(
            windows::core::PCWSTR(wide.as_ptr()),
            0xC000_0000,
            windows::Win32::Storage::FileSystem::FILE_SHARE_READ
                | windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE,
            None,
            windows::Win32::Storage::FileSystem::OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(|error| error.to_string())?
    };
    let mut file = unsafe { std::fs::File::from_raw_handle(handle.0) };
    use std::io::{Read, Write};
    file.write_all(&frame).map_err(|error| error.to_string())?;
    let mut prefix = [0u8; 4];
    file.read_exact(&mut prefix)
        .map_err(|error| error.to_string())?;
    let length = u32::from_le_bytes(prefix) as usize;
    let mut body = vec![0u8; length];
    file.read_exact(&mut body)
        .map_err(|error| error.to_string())?;
    String::from_utf8(body).map_err(|error| error.to_string())
}

fn wait_until(limit: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if ready() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    ready()
}
