//! Hosted-runner fail-closed cases. A normal `cargo test` returns immediately.
//! The fail-closed workflow sets `SOFTLOCK_FAIL_CLOSED_LIVE` to one case.
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
        .env_remove("SOFTLOCK_STUB_OS_LOCK")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if case == "unhealthy-guard" {
        command.env("SOFTLOCK_FORCE_GUARD_DOWN", "1");
    }
    let mut child = command.spawn().expect("service");
    wait_until(Duration::from_secs(20), || transact(&pipe, hello()).is_ok());
    let response = match case.as_str() {
        "tray-hard-lock" => String::new(),
        _ => transact(&pipe, lock_now()).expect("lock"),
    };
    if case != "tray-hard-lock" {
        assert!(
            response.contains("Accepted"),
            "{case} was not accepted: {response}"
        );
        wait_until(Duration::from_secs(15), || {
            transact(&pipe, status())
                .unwrap_or_default()
                .contains("Locked")
        });
        if matches!(case.as_str(), "kill-host" | "suspend-host") {
            wait_until(Duration::from_secs(10), || host_pid().is_some());
        }
    }
    let started = Instant::now();
    if case == "tray-hard-lock" {
        let response = transact(&pipe, hard_lock()).expect("hard lock");
        assert!(
            response.contains("Accepted"),
            "tray hard lock was not accepted: {response}"
        );
    }
    match case.as_str() {
        "kill-host" => {
            let pid = host_pid().expect("lock host");
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .output();
        }
        "suspend-host" => {
            suspend_process(host_pid().expect("lock host"));
        }
        "kill-service" => {
            child.kill().expect("kill service");
            let _ = child.wait();
        }
        "stop-while-locked" => {
            fs::write(&stop_file, b"stop").expect("stop file");
        }
        "unhealthy-guard" | "tray-hard-lock" => {}
        other => panic!("unknown case {other}"),
    }
    let confirmed = wait_until(Duration::from_secs(2), || {
        let text = fs::read_to_string(&log).unwrap_or_default();
        let marked = fs::read_to_string(&mark).unwrap_or_default();
        (text.contains("result=confirmed") || marked.contains("confirmed"))
            && !text.contains("\"Unlocked\"")
    });
    let elapsed = started.elapsed();
    let text = fs::read_to_string(&log).unwrap_or_default();
    let _ = child.kill();
    let _ = child.wait();
    let _ = Command::new("taskkill")
        .args(["/IM", "softlock-lockhost.exe", "/F"])
        .output();
    assert!(
        confirmed,
        "{case} did not confirm within 2s (budget is 1s from the fault): {text}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "{case} took {} ms",
        elapsed.as_millis()
    );
    assert!(
        !text.contains("state=Unlocked") && !text.contains("\"Unlocked\""),
        "{case} reached Unlocked: {text}"
    );
    let elapsed_us = text
        .split_whitespace()
        .find_map(|field| field.strip_prefix("elapsed_us="))
        .unwrap_or("missing");
    fs::write(
        root.join("result.json"),
        format!(
            "{{\"case\":\"{case}\",\"confirmed\":true,\"elapsed_us\":{elapsed_us},\"unlocked\":false}}\n"
        ),
    )
    .expect("result");
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

fn hard_lock() -> &'static str {
    r#"{"v":1,"id":2,"kind":"HardLock"}"#
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
