//! Hosted-runner proof that a kill between the journal and the registry,
//! and a kill after the change, restores DisableTaskMgr exactly.
//! A normal `cargo test` returns immediately. The settings-journal workflow
//! sets `SOFTLOCK_JOURNAL_LIVE` and builds with `live-fail-closed`.

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use softlock_core::SERVICE_NAME;

const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\System";

#[test]
fn live_settings_journal() {
    let Ok(case) = std::env::var("SOFTLOCK_JOURNAL_LIVE") else {
        return;
    };
    match softlock_windows::runner_may_lock() {
        Ok(()) => {}
        Err(reason) => panic!("refusing the settings journal on this machine: {reason}"),
    }
    if installed_service_running() {
        panic!("refusing while the installed service is running");
    }
    fs::create_dir_all(guard_dir()).expect("temp");
    fs::create_dir_all(artifact_dir()).expect("artifact");
    ensure_service().expect("LocalSystem service");
    let original = query_taskmgr();
    let mut guard = Guard {
        original: original.clone(),
        reported: false,
    };
    let outcome = match case.as_str() {
        "restore" => restore(&mut guard),
        "recover-active" => recover_active(&mut guard),
        other => Err(format!("unknown case {other}")),
    };
    match outcome {
        Ok(()) => {
            capture();
            guard.reported = true;
            write_result(r#"{"ok":true}"#);
        }
        Err(reason) => {
            capture();
            guard.reported = true;
            write_result(&format!(r#"{{"ok":false,"reason":"{reason}"}}"#));
            panic!("{reason}");
        }
    }
}

fn restore(guard: &mut Guard) -> Result<(), String> {
    for previous in ["absent", "0", "1"] {
        kill_before_apply(guard, previous)?;
    }
    kill_after_apply(guard)?;
    reassert_and_repair(guard)?;
    Ok(())
}

fn kill_before_apply(guard: &mut Guard, previous: &str) -> Result<(), String> {
    set_taskmgr(previous)?;
    let pause = guard_dir().join(format!("pause-{previous}"));
    let _ = fs::remove_file(&pause);
    spawn_service(guard, true, &pause)?;
    wait_pipe()?;
    transact(&pipe_name(), lock_now()).map_err(|error| format!("lock: {error}"))?;
    let journal = journal_file();
    let saw = wait_until(Duration::from_secs(20), || {
        journal_ready(&journal) && query_taskmgr() == previous
    });
    if !saw || !journal_ready(&journal) {
        return Err(format!(
            "journal was not durable before the {previous} change"
        ));
    }
    // The service is blocked after the flush. A stop file is not read until
    // that pause ends, which would apply the value. Kill it.
    hard_stop(guard);
    if query_taskmgr() != previous {
        return Err(format!("kill changed DisableTaskMgr from {previous}"));
    }
    spawn_service(guard, false, &pause)?;
    let restored = wait_until(Duration::from_secs(20), || query_taskmgr() == previous);
    hard_stop(guard);
    if !restored {
        return Err(format!("restart did not restore {previous}"));
    }
    if query_taskmgr() != previous {
        return Err(format!("restored value was not {previous}"));
    }
    Ok(())
}

fn kill_after_apply(guard: &mut Guard) -> Result<(), String> {
    set_taskmgr("absent")?;
    let pause = guard_dir().join("pause-after");
    fs::write(&pause, b"go").map_err(|_| "pause file".to_string())?;
    spawn_service(guard, true, &pause)?;
    wait_pipe()?;
    transact(&pipe_name(), lock_now()).map_err(|error| format!("lock: {error}"))?;
    let applied = wait_until(Duration::from_secs(20), || query_taskmgr() == "1");
    if !applied {
        return Err("DisableTaskMgr was not set to 1".to_string());
    }
    hard_stop(guard);
    spawn_service(guard, false, &pause)?;
    let restored = wait_until(Duration::from_secs(20), || query_taskmgr() == "absent");
    hard_stop(guard);
    if !restored {
        return Err("restart did not restore an absent value".to_string());
    }
    Ok(())
}

fn reassert_and_repair(guard: &mut Guard) -> Result<(), String> {
    set_taskmgr("absent")?;
    let pause = guard_dir().join("pause-reassert");
    fs::write(&pause, b"go").map_err(|_| "pause file".to_string())?;
    spawn_service(guard, true, &pause)?;
    wait_pipe()?;
    transact(&pipe_name(), lock_now()).map_err(|error| format!("lock: {error}"))?;
    if !wait_until(Duration::from_secs(20), || query_taskmgr() == "1") {
        return Err("lock did not set DisableTaskMgr".to_string());
    }
    set_taskmgr("absent")?;
    if !wait_until(Duration::from_secs(8), || query_taskmgr() == "1") {
        return Err("cleared DisableTaskMgr was not re-asserted within 5 s".to_string());
    }
    let repair = repair_settings()?;
    if repair == 0 {
        return Err("repair ran while locked".to_string());
    }
    if query_taskmgr() != "1" {
        return Err("repair changed DisableTaskMgr while locked".to_string());
    }
    hard_stop(guard);
    let first = repair_settings()?;
    let second = repair_settings()?;
    if first != 0 || second != 0 {
        return Err(format!(
            "repair after the kill returned {first} then {second}"
        ));
    }
    if query_taskmgr() != "absent" {
        return Err("repair did not restore the absent value".to_string());
    }
    Ok(())
}

fn recover_active(guard: &mut Guard) -> Result<(), String> {
    set_taskmgr("0")?;
    let pause = guard_dir().join("pause-active");
    fs::write(&pause, b"go").map_err(|_| "pause file".to_string())?;
    spawn_service(guard, true, &pause)?;
    wait_pipe()?;
    transact(&pipe_name(), lock_now()).map_err(|error| format!("lock: {error}"))?;
    if !wait_until(Duration::from_secs(20), || {
        query_taskmgr() == "1" && host_running() && journal_ready(&journal_file())
    }) {
        return Err("the lock host was not running with DisableTaskMgr set".to_string());
    }
    kill_service_process();
    if !host_running() {
        return Err("the lock host died with the service".to_string());
    }
    spawn_service(guard, false, &pause)?;
    let mut last = String::new();
    let mut saw_unlocked = false;
    let closed = wait_until(Duration::from_secs(25), || {
        let status = transact(&pipe_name(), status_body()).unwrap_or_default();
        let state = status_state(&status);
        if state.as_deref() == Some("Unlocked") {
            saw_unlocked = true;
        }
        last = status;
        let fail_closed = matches!(
            state.as_deref(),
            Some("FailingClosed" | "Locked" | "OsLocked")
        );
        query_taskmgr() == "1" && fail_closed
    });
    if saw_unlocked {
        return Err("an active lock returned to Unlocked".to_string());
    }
    if !closed || query_taskmgr() != "1" {
        return Err(format!(
            "an active journal did not keep DisableTaskMgr: {last} value {}",
            query_taskmgr()
        ));
    }
    Ok(())
}

fn status_state(json: &str) -> Option<String> {
    let key = "\"state\":\"";
    let start = json.find(key)? + key.len();
    let rest = json.get(start..)?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

struct Guard {
    original: String,
    reported: bool,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "softlock-lockhost.exe"])
            .status();
        kill_service_process();
        let _ = Command::new("sc").args(["delete", SERVICE_NAME]).status();
        let _ = set_taskmgr(&self.original);
        if !self.reported {
            capture();
            write_result(r#"{"ok":false,"reason":"ended"}"#);
        }
    }
}

fn ensure_service() -> Result<(), String> {
    let _ = Command::new("sc").args(["stop", SERVICE_NAME]).status();
    let _ = Command::new("sc").args(["delete", SERVICE_NAME]).status();
    thread::sleep(Duration::from_millis(400));
    let bin = service_exe().display().to_string();
    let status = Command::new("sc")
        .args([
            "create",
            SERVICE_NAME,
            "binPath=",
            &bin,
            "start=",
            "demand",
            "obj=",
            "LocalSystem",
        ])
        .status()
        .map_err(|_| "sc create".to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("sc create failed".to_string())
    }
}

fn spawn_service(_guard: &mut Guard, pause: bool, pause_file: &Path) -> Result<(), String> {
    fs::create_dir_all(guard_dir()).map_err(|_| "temp".to_string())?;
    fs::create_dir_all(guard_dir().join("pin")).map_err(|_| "temp".to_string())?;
    kill_service_process();
    wait_until(Duration::from_secs(15), || !service_running());
    write_service_env(pause, pause_file)?;
    let status = Command::new("sc")
        .args(["start", SERVICE_NAME])
        .status()
        .map_err(|_| "sc start".to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("sc start failed".to_string())
    }
}

fn write_service_env(pause: bool, pause_file: &Path) -> Result<(), String> {
    let runner =
        std::env::var("RUNNER_ENVIRONMENT").unwrap_or_else(|_| "github-hosted".to_string());
    let mut pairs = vec![
        "GITHUB_ACTIONS=true".to_string(),
        format!("RUNNER_ENVIRONMENT={runner}"),
        "SOFTLOCK_DEV_APP=1".to_string(),
        format!("SOFTLOCK_PIPE={}", pipe_name()),
        format!("SOFTLOCK_LOG={}", guard_dir().join("service.log").display()),
        format!("SOFTLOCK_PIN_DIR={}", guard_dir().join("pin").display()),
        format!("SOFTLOCK_STOP_FILE={}", guard_dir().join("stop").display()),
    ];
    if pause {
        pairs.push("SOFTLOCK_JOURNAL_PAUSE=1".to_string());
        pairs.push(format!(
            "SOFTLOCK_JOURNAL_PAUSE_FILE={}",
            pause_file.display()
        ));
    }
    let data = pairs.join("\\0");
    let key = format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{SERVICE_NAME}");
    let status = Command::new("reg")
        .args([
            "add",
            &key,
            "/v",
            "Environment",
            "/t",
            "REG_MULTI_SZ",
            "/d",
            &data,
            "/f",
        ])
        .status()
        .map_err(|_| "env".to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("env".to_string())
    }
}

fn hard_stop(_guard: &mut Guard) {
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "softlock-lockhost.exe"])
        .status();
    kill_service_process();
}

fn kill_service_process() {
    if let Some(pid) = service_pid() {
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .status();
    }
    let _ = Command::new("sc").args(["stop", SERVICE_NAME]).status();
}

fn service_pid() -> Option<u32> {
    let output = Command::new("sc")
        .args(["queryex", SERVICE_NAME])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        let rest = line
            .strip_prefix("PID")
            .or_else(|| line.strip_prefix("PID:"))?;
        let rest = rest.trim().trim_start_matches(':').trim();
        if let Ok(pid) = rest.parse::<u32>()
            && pid != 0
        {
            return Some(pid);
        }
    }
    None
}

fn service_running() -> bool {
    let Ok(output) = Command::new("sc").args(["query", SERVICE_NAME]).output() else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).contains("RUNNING")
}

fn capture() {
    let _ = fs::create_dir_all(artifact_dir());
    let log = guard_dir().join("service.log");
    let body = fs::read_to_string(&log).unwrap_or_else(|_| "missing".to_string());
    let _ = fs::write(artifact_dir().join("service.log"), body);
    let dir = journal_file();
    let parent = dir.parent().unwrap_or(dir.as_path());
    let listing = Command::new("cmd")
        .args(["/c", "dir", "/a", &parent.display().to_string()])
        .output();
    let text = listing
        .map(|output| {
            let mut body = String::from_utf8_lossy(&output.stdout).into_owned();
            body.push_str(&String::from_utf8_lossy(&output.stderr));
            body
        })
        .unwrap_or_else(|_| "missing".to_string());
    let _ = fs::write(artifact_dir().join("journal-dir.txt"), text);
}

fn repair_settings() -> Result<i32, String> {
    let output = Command::new(service_exe())
        .arg("--repair-settings")
        .env("SOFTLOCK_PIPE", pipe_name())
        .output()
        .map_err(|_| "repair".to_string())?;
    Ok(output.status.code().unwrap_or(1))
}

fn wait_pipe() -> Result<(), String> {
    let ok = wait_until(Duration::from_secs(20), || {
        transact(&pipe_name(), hello()).is_ok()
    });
    if ok {
        Ok(())
    } else {
        Err("pipe did not open".to_string())
    }
}

fn journal_ready(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    text.contains("DisableTaskMgr")
        && text.contains("\"says_locked\":true")
        && text.contains("\"sid\":\"S-1-5-21-")
}

fn journal_file() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    PathBuf::from(root)
        .join("System32")
        .join("config")
        .join("systemprofile")
        .join("AppData")
        .join("Local")
        .join(format!("{SERVICE_NAME}Journal"))
        .join("journal.json")
}

fn query_taskmgr() -> String {
    let output = Command::new("reg")
        .args(["query", KEY, "/v", "DisableTaskMgr"])
        .output();
    let Ok(output) = output else {
        return "absent".to_string();
    };
    if !output.status.success() {
        return "absent".to_string();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    if text.contains("0x1") {
        "1".to_string()
    } else if text.contains("0x0") {
        "0".to_string()
    } else {
        "other".to_string()
    }
}

fn set_taskmgr(value: &str) -> Result<(), String> {
    if value == "absent" {
        let _ = Command::new("reg")
            .args(["delete", KEY, "/v", "DisableTaskMgr", "/f"])
            .status();
        if query_taskmgr() != "absent" {
            return Err("could not delete DisableTaskMgr".to_string());
        }
        return Ok(());
    }
    let status = Command::new("reg")
        .args([
            "add",
            KEY,
            "/v",
            "DisableTaskMgr",
            "/t",
            "REG_DWORD",
            "/d",
            value,
            "/f",
        ])
        .status()
        .map_err(|_| "reg add".to_string())?;
    if status.success() && query_taskmgr() == value {
        Ok(())
    } else {
        Err("could not set DisableTaskMgr".to_string())
    }
}

fn installed_service_running() -> bool {
    let Ok(output) = Command::new("sc").args(["query", SERVICE_NAME]).output() else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).contains("RUNNING")
}

fn host_running() -> bool {
    let Ok(output) = Command::new("tasklist")
        .args([
            "/FI",
            "IMAGENAME eq softlock-lockhost.exe",
            "/FO",
            "CSV",
            "/NH",
        ])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout)
        .to_ascii_lowercase()
        .contains("softlock-lockhost.exe")
}

fn service_exe() -> PathBuf {
    std::env::var("SOFTLOCK_SERVICE_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/journal/release/softlock-service.exe"))
}

fn pipe_name() -> String {
    format!(r"\\.\pipe\softlock-journal-{}", std::process::id())
}

fn guard_dir() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    PathBuf::from(root)
        .join("Temp")
        .join(format!("softlock-journal-{}", std::process::id()))
}

fn artifact_dir() -> PathBuf {
    let base = std::env::var("RUNNER_TEMP")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("softlock-journal")
}

fn hello() -> &'static str {
    r#"{"v":1,"id":1,"kind":"Hello","client_version":"0"}"#
}

fn lock_now() -> &'static str {
    r#"{"v":1,"id":2,"kind":"Lock","source":"App"}"#
}

fn status_body() -> &'static str {
    r#"{"v":1,"id":3,"kind":"GetStatus"}"#
}

fn wait_until(limit: Duration, mut probe: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if probe() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    probe()
}

fn write_result(body: &str) {
    let dir = artifact_dir();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("result.json"), body);
}

fn transact(pipe: &str, json: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::os::windows::io::FromRawHandle;
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
        .map_err(|error| format!("open: {error}"))?
    };
    let mut file = unsafe { std::fs::File::from_raw_handle(handle.0) };
    file.write_all(&frame)
        .map_err(|error| format!("write: {error}"))?;
    let mut prefix = [0u8; 4];
    file.read_exact(&mut prefix)
        .map_err(|error| format!("read: {error}"))?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length > 64 * 1024 {
        return Err("frame".to_string());
    }
    let mut body = vec![0u8; length];
    file.read_exact(&mut body)
        .map_err(|error| format!("body: {error}"))?;
    String::from_utf8(body).map_err(|_| "utf8".to_string())
}
