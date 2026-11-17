//! Seam 3. The service owns the lock. The Lock Host is a child process.

#![cfg(all(windows, feature = "simulated-physical"))]

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use softlock_core::pipe_name;
use softlock_lockhost::{RunNonce, set_run_nonce};
use softlock_seam3::{Probe, send_virtual_key};
use softlock_service::{
    SYSTEM_ONLY_SDDL, dacl_sddl, is_reparse_point, service_data_directory, user_read_denied,
};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile, WriteFile,
};
use windows::core::PCWSTR;

const VK_F13: u16 = 0x7C;
const VK_F14: u16 = 0x7D;
const VK_RETURN: u16 = 0x0D;
const PIN: &[u8] = b"135742";
const NOTE: &str = "Quarterly Report - Notepad";

fn gate() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

struct Session {
    child: std::process::Child,
    pipe: String,
    log: PathBuf,
    pin_dir: PathBuf,
    extra: u64,
}

impl Session {
    fn start(panic: Option<&str>, dev_app: bool) -> Self {
        let _ = gate;
        let root = std::env::temp_dir().join(format!("svc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp");
        let pin_dir = root.join("pin");
        let log = root.join("service.log");
        fs::create_dir_all(&pin_dir).expect("pin dir");
        static PIPES: AtomicU64 = AtomicU64::new(0);
        let pipe = format!(
            r"\\.\pipe\softlock-e689-{}-{}",
            std::process::id(),
            PIPES.fetch_add(1, Ordering::Relaxed)
        );
        let nonce = RunNonce::random().expect("run nonce");
        set_run_nonce(nonce);
        let extra = nonce.as_extra_info();
        let service = service_exe();
        let mut command = Command::new(&service);
        command
            .arg("--foreground")
            .env("SOFTLOCK_STUB_OS_LOCK", "1")
            .env("SOFTLOCK_PIN_DIR", &pin_dir)
            .env("SOFTLOCK_LOG", &log)
            .env("SOFTLOCK_PIPE", &pipe)
            .env("SOFTLOCK_RUN_NONCE", extra.to_string())
            .env("SOFTLOCK_INSTALL_DIR", r"C:\no\such\install")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(mode) = panic {
            command.env("SOFTLOCK_PANIC", mode);
        } else {
            command.env_remove("SOFTLOCK_PANIC");
        }
        if dev_app {
            command.env("SOFTLOCK_DEV_APP", "1");
        }
        let child = command.spawn().expect("service process");
        let session = Self {
            child,
            pipe,
            log,
            pin_dir,
            extra,
        };
        session.wait_until(|session| session.hello().is_ok(), Duration::from_secs(15));
        session
    }

    fn hello(&self) -> Result<String, String> {
        self.transact(r#"{"v":1,"id":1,"kind":"Hello","client_version":"0"}"#)
    }

    fn lock(&self) -> Result<String, String> {
        self.transact(&format!(
            r#"{{"v":1,"id":2,"kind":"Lock","source":"LocalApi","note":"{NOTE}"}}"#
        ))
    }

    fn status(&self) -> Result<String, String> {
        self.transact(r#"{"v":1,"id":3,"kind":"GetStatus"}"#)
    }

    fn set_pin(&self) -> Result<String, String> {
        let pin = std::str::from_utf8(PIN).unwrap_or("");
        self.transact(&format!(
            r#"{{"v":1,"id":4,"kind":"SetPin","pin":"{pin}"}}"#
        ))
    }

    fn transact(&self, json: &str) -> Result<String, String> {
        let handle = open_pipe(&self.pipe)?;
        let mut frame = (json.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(json.as_bytes());
        write_all(handle, &frame)?;
        let response = read_frame(handle)?;
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
        String::from_utf8(response).map_err(|error| error.to_string())
    }

    fn wait_until(&self, mut ready: impl FnMut(&Self) -> bool, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if ready(self) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(ready(self), "timed out waiting for the service");
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &self.child.id().to_string()])
            .output();
    }
}

fn service_exe() -> PathBuf {
    let host = PathBuf::from(env!("CARGO_BIN_EXE_softlock-lockhost"));
    let service = host.with_file_name("softlock-service.exe");
    if service.is_file() {
        return service;
    }
    let mut command = Command::new("cargo");
    command.args([
        "build",
        "-p",
        "softlock-service",
        "--features",
        "simulated-physical",
    ]);
    if host.to_string_lossy().contains("x86_64-pc-windows-msvc") {
        command.args(["--target", "x86_64-pc-windows-msvc"]);
    }
    let status = command.status().expect("cargo build");
    assert!(status.success(), "service build failed");
    assert!(service.is_file(), "missing {}", service.display());
    service
}

fn open_pipe(name: &str) -> Result<HANDLE, String> {
    let started = Instant::now();
    loop {
        match open_pipe_once(name) {
            Err(error)
                if error.contains("0x800700E7") && started.elapsed() < Duration::from_secs(2) =>
            {
                thread::sleep(Duration::from_millis(20));
            }
            other => return other,
        }
    }
}

fn open_pipe_once(name: &str) -> Result<HANDLE, String> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(|error| format!("open pipe: {error}"))
    }
}

fn write_all(handle: HANDLE, bytes: &[u8]) -> Result<(), String> {
    let mut offset = 0;
    while offset < bytes.len() {
        let mut wrote = 0u32;
        unsafe {
            WriteFile(handle, Some(&bytes[offset..]), Some(&mut wrote), None)
                .map_err(|error| format!("write: {error}"))?;
        }
        if wrote == 0 {
            return Err("short write".to_string());
        }
        offset += wrote as usize;
    }
    Ok(())
}

fn read_frame(handle: HANDLE) -> Result<Vec<u8>, String> {
    let mut prefix = [0u8; 4];
    read_exact(handle, &mut prefix)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length > 64 * 1024 {
        return Err("frame too large".to_string());
    }
    let mut body = vec![0u8; length];
    read_exact(handle, &mut body)?;
    Ok(body)
}

fn read_exact(handle: HANDLE, buf: &mut [u8]) -> Result<(), String> {
    let mut offset = 0;
    while offset < buf.len() {
        let mut read = 0u32;
        unsafe {
            ReadFile(handle, Some(&mut buf[offset..]), Some(&mut read), None)
                .map_err(|error| format!("read: {error}"))?;
        }
        if read == 0 {
            return Err("eof".to_string());
        }
        offset += read as usize;
    }
    Ok(())
}

fn type_pin(extra: u64) {
    send_virtual_key(VK_F13, extra).expect("wake");
    thread::sleep(Duration::from_millis(100));
    for byte in PIN {
        send_virtual_key(u16::from(*byte), extra).expect("digit");
    }
    send_virtual_key(VK_RETURN, extra).expect("submit");
}

#[test]
fn lock_host_idles_before_lock_and_arms_within_500ms() {
    let _guard = gate();
    let session = Session::start(None, false);
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    send_virtual_key(VK_F13, 0).expect("idle key");
    probe
        .wait_for_key(VK_F13, Duration::from_secs(3))
        .expect("the lock host swallowed input before the first lock");
    let started = Instant::now();
    let locked = session.lock().expect("lock");
    let measured = started.elapsed();
    print!("service arming took {} ms ", measured.as_millis());
    assert!(
        locked.contains("Accepted"),
        "lock was not accepted: {locked}"
    );
    let blocked = physical_is_blocked(&probe, session.extra);
    assert!(blocked, "physical input was not blocked");
    assert!(
        measured < Duration::from_millis(500),
        "service arming took {measured:?}"
    );
}

#[test]
fn lock_client_lock_blocks_physical_input_and_passes_injected() {
    let _guard = gate();
    let session = Session::start(None, false);
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let locked = session.lock().expect("lock");
    assert!(
        locked.contains("Accepted"),
        "lock was not accepted: {locked}"
    );
    assert!(
        physical_is_blocked(&probe, session.extra),
        "physical input reached the probe"
    );
    send_virtual_key(VK_F13, 0).expect("injected");
    probe
        .wait_for_key(VK_F13, Duration::from_secs(3))
        .expect("injected input did not reach the probe");
    let status = session.status().expect("status");
    assert!(
        status.contains("Locked") || status.contains("FailingClosed"),
        "status was not a locked state: {status}"
    );
    assert!(
        !status.contains("Unlocked"),
        "status reported Unlocked: {status}"
    );
}

#[test]
fn private_pin_unlocks_and_the_hash_is_not_user_readable() {
    let _guard = gate();
    let session = Session::start(None, true);
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let stored = session.set_pin().expect("set pin");
    assert!(stored.contains("Ok"), "pin was not stored: {stored}");
    let hash_path = session.pin_dir.join("pin.phc");
    match fs::File::open(&hash_path) {
        Ok(_) => panic!(
            "the user could read the pin hash at {}",
            hash_path.display()
        ),
        Err(error) => assert_eq!(
            error.raw_os_error(),
            Some(5),
            "reading the hash failed with {error}"
        ),
    }
    let locked = session.lock().expect("lock");
    // Lock returns Accepted only after HostReady, so the hooks are already
    // up. That response is the event; there is no timed poll.
    assert!(
        locked.contains("Accepted"),
        "lock was not accepted: {locked}"
    );
    assert!(
        physical_is_blocked(&probe, session.extra),
        "lock did not block physical input before the pin"
    );
    type_pin(session.extra);
    assert!(
        wait_for(Duration::from_secs(30), || {
            session
                .status()
                .ok()
                .is_some_and(|status| status.contains("Unlocked"))
        }),
        "the correct pin did not unlock"
    );
    // Status flips when the service accepts the PIN. The host drops its hooks
    // when the following Disarm arrives, so a physical key is retried until
    // one reaches the probe.
    let before = probe
        .inputs()
        .iter()
        .filter(|input| input.virtual_key == VK_F14)
        .count();
    assert!(
        wait_for(Duration::from_secs(8), || {
            if send_virtual_key(VK_F14, session.extra).is_err() {
                return false;
            }
            thread::sleep(Duration::from_millis(40));
            probe
                .inputs()
                .iter()
                .filter(|input| input.virtual_key == VK_F14)
                .count()
                > before
        }),
        "input stayed blocked after unlock"
    );
}

#[test]
fn service_log_omits_the_pin_and_the_note() {
    let _guard = gate();
    let session = Session::start(None, true);
    session.set_pin().expect("set pin");
    session.lock().expect("lock");
    thread::sleep(Duration::from_millis(300));
    let log = fs::read_to_string(&session.log).unwrap_or_default();
    let pin = std::str::from_utf8(PIN).unwrap_or("");
    assert!(!log.contains(pin), "pin reached the log: {log}");
    assert!(!log.contains("$argon2"), "hash reached the log: {log}");
    assert!(!log.contains(NOTE), "window title reached the log: {log}");
    assert!(
        !log.contains("Notepad"),
        "window title reached the log: {log}"
    );
}

#[test]
fn panic_while_os_lock_is_down_is_closed_by_the_service() {
    let _guard = gate();
    let session = Session::start(Some("after-arm"), false);
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    session.lock().expect("lock");
    let mut saw_unlocked = false;
    let lost = wait_for(Duration::from_secs(8), || {
        if session
            .status()
            .ok()
            .is_some_and(|status| status.contains("\"Unlocked\""))
        {
            saw_unlocked = true;
        }
        fs::read_to_string(&session.log)
            .unwrap_or_default()
            .contains("host-lost")
    });
    assert!(
        !saw_unlocked,
        "the service reported Unlocked during recovery"
    );
    assert!(
        lost,
        "the service log did not record host-lost after the window panic"
    );
    let recovered = wait_for(Duration::from_secs(20), || {
        if session
            .status()
            .ok()
            .is_some_and(|status| status.contains("\"Unlocked\""))
        {
            saw_unlocked = true;
        }
        physical_is_blocked(&probe, session.extra)
    });
    assert!(recovered, "the service did not restore the guard");
    let status = session.status().expect("status");
    assert!(
        !status.contains("\"Unlocked\""),
        "recovery ended Unlocked: {status}"
    );
}

#[test]
fn emergency_after_both_os_locks_fail_is_closed_by_the_service() {
    let _guard = gate();
    let session = Session::start(Some("callback"), false);
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    session.lock().expect("lock");
    // F13 panics the hook callback. Both OS locks are stubbed, so the window
    // thread runs emergency() and the process exits. The service log records
    // that loss even when the replacement host is armed before the next poll.
    let _ = send_virtual_key(VK_F13, session.extra);
    let mut saw_unlocked = false;
    let lost = wait_for(Duration::from_secs(8), || {
        if session
            .status()
            .ok()
            .is_some_and(|status| status.contains("\"Unlocked\""))
        {
            saw_unlocked = true;
        }
        fs::read_to_string(&session.log)
            .unwrap_or_default()
            .contains("host-lost")
    });
    assert!(
        !saw_unlocked,
        "the service reported Unlocked after emergency()"
    );
    assert!(
        lost,
        "emergency() did not look like a lost host to the service"
    );
    let recovered = wait_for(Duration::from_secs(20), || {
        if session
            .status()
            .ok()
            .is_some_and(|status| status.contains("\"Unlocked\""))
        {
            saw_unlocked = true;
        }
        physical_is_blocked(&probe, session.extra)
    });
    assert!(
        !saw_unlocked,
        "the service reported Unlocked during recovery"
    );
    assert!(
        recovered,
        "the service did not restore the guard after emergency()"
    );
    let status = session.status().expect("status");
    assert!(
        !status.contains("\"Unlocked\""),
        "recovery ended Unlocked: {status}"
    );
}

#[test]
fn installed_service_runs_as_local_system_and_rejects_session_zero() {
    let _guard = gate();
    let service = service_exe();
    let (name, display) = service_names(&service);
    let probe = std::env::temp_dir().join(format!("svc-probe-{}.txt", std::process::id()));
    let body = install_service(&service, &name, &display, &probe, &[]);
    remove_service(&name);
    assert!(body.contains("session=0"), "not session 0: {body}");
    assert!(body.contains("system=yes"), "not LocalSystem: {body}");
    assert!(
        body.contains("kind=Unauthorized"),
        "session 0 client was not rejected: {body}"
    );
}

#[test]
fn lock_host_token_is_system_in_the_console_session() {
    let _guard = gate();
    let service = service_exe();
    let (name, display) = service_names(&service);
    let probe = std::env::temp_dir().join(format!("svc-host-{}.txt", std::process::id()));
    let body = install_service(&service, &name, &display, &probe, &[]);
    remove_service(&name);
    assert!(body.contains("system=yes"), "not LocalSystem: {body}");
    assert!(
        body.contains("host_user=S-1-5-18"),
        "lock host is not LocalSystem: {body}"
    );
    let host_session = probe_value(&body, "host_session");
    let console = probe_value(&body, "console");
    assert_eq!(
        host_session, console,
        "host session {host_session} is not the console session {console}: {body}"
    );
    assert_ne!(
        host_session, "missing",
        "host session was not recorded: {body}"
    );
    assert_ne!(host_session, "0", "host stayed in session 0: {body}");
}

#[test]
fn service_stop_while_locked_keeps_input_blocked() {
    let _guard = gate();
    let service = service_exe();
    let (name, display) = service_names(&service);
    let nonce = RunNonce::random().expect("run nonce");
    set_run_nonce(nonce);
    let extra = nonce.as_extra_info();
    let probe = std::env::temp_dir().join(format!("svc-stop-{}.txt", std::process::id()));
    let log = std::env::temp_dir().join(format!("svc-stop-{}.log", std::process::id()));
    let _ = fs::remove_file(&log);
    let env = [
        "SOFTLOCK_STUB_OS_LOCK=1".to_string(),
        format!("SOFTLOCK_RUN_NONCE={extra}"),
        r"SOFTLOCK_INSTALL_DIR=C:\no\such\install".to_string(),
        format!("SOFTLOCK_LOG={}", log.display()),
    ];
    let body = install_service(&service, &name, &display, &probe, &env);
    assert!(body.contains("system=yes"), "not LocalSystem: {body}");
    let input = Probe::open().expect("probe");
    input.claim_focus();
    let locked = transact(
        &pipe_name(),
        &format!(r#"{{"v":1,"id":2,"kind":"Lock","source":"LocalApi","note":"{NOTE}"}}"#),
    )
    .expect("lock");
    assert!(
        locked.contains("Accepted"),
        "lock was not accepted: {locked}"
    );
    assert!(
        physical_is_blocked(&input, extra),
        "physical input was not blocked before stop"
    );
    let name_for_stop = name.clone();
    thread::spawn(move || {
        let _ = Command::new("sc.exe")
            .args(["stop", &name_for_stop])
            .output();
    });
    let saw_stop = wait_for(Duration::from_secs(15), || {
        fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("service-stop")
    });
    let log_text = fs::read_to_string(&log).unwrap_or_default();
    assert!(saw_stop, "sc stop was not handled: {log_text}");
    assert!(
        log_text.contains("result=refused"),
        "stop was accepted without an OS lock: {log_text}"
    );
    assert!(
        physical_is_blocked(&input, extra),
        "sc stop dropped the guard while the OS lock was not confirmed"
    );
    let status = transact(&pipe_name(), r#"{"v":1,"id":3,"kind":"GetStatus"}"#).unwrap_or_default();
    assert!(
        !status.contains("\"Unlocked\""),
        "stop ended Unlocked: {status}"
    );
    remove_service(&name);
}

#[test]
fn local_system_pin_file_is_rewritten_and_unreadable() {
    let _guard = gate();
    let service = service_exe();
    let (name, display) = service_names(&service);
    let dir = service_data_directory();
    reclaim_data_dir(&dir);
    let probe = std::env::temp_dir().join(format!("svc-pin-{}.txt", std::process::id()));
    let body = install_service(
        &service,
        &name,
        &display,
        &probe,
        &["SOFTLOCK_PIN_SELFTEST=1".to_string()],
    );
    remove_service(&name);
    assert!(
        body.contains("pin_loaded=none"),
        "a hash was loaded before the service wrote one: {body}"
    );
    assert!(
        body.contains("pin_reload=ok"),
        "LocalSystem did not save and reload the pin file: {body}"
    );
    let path = dir.join("pin.phc");
    assert!(
        user_read_denied(&path),
        "the interactive user could read {}",
        path.display()
    );
    assert_eq!(dacl_sddl(&dir).expect("directory dacl"), SYSTEM_ONLY_SDDL);
    assert_eq!(dacl_sddl(&path).expect("pin dacl"), SYSTEM_ONLY_SDDL);
    reclaim_data_dir(&dir);
}

#[test]
fn several_pipe_instances_accept_at_once() {
    let _guard = gate();
    let session = Session::start(None, false);
    let hello = br#"{"v":1,"id":1,"kind":"Hello","client_version":"0"}"#;
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(open_pipe(&session.pipe).expect("held pipe"));
    }
    for handle in &held {
        let mut frame = (hello.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(hello);
        write_all(*handle, &frame).expect("write hello");
    }
    let fifth = session.hello().expect("fifth pipe");
    assert!(
        fifth.contains("Hello"),
        "an extra client was blocked while other instances were open: {fifth}"
    );
    for handle in &held {
        let body = read_frame(*handle).expect("held response");
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("Hello"),
            "a held pipe instance got no reply: {text}"
        );
    }
    for handle in held {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
    }
}

#[test]
fn standard_user_cannot_plant_the_secret_store() {
    if let Ok(mode) = std::env::var("SOFTLOCK_PLANT_CHILD") {
        plant_child(&mode);
        return;
    }
    let _guard = gate();
    let account = StandardAccount::create();
    let dir = service_data_directory();
    reclaim_data_dir(&dir);
    let bait = std::env::temp_dir().join(format!("svc-bait-{}", std::process::id()));
    let _ = fs::remove_dir_all(&bait);
    fs::create_dir_all(&bait).expect("bait");
    fs::write(bait.join("marker.txt"), b"bait-marker").expect("marker");
    fs::write(bait.join("pin.phc"), b"bait-hash").expect("bait pin");
    fs::write(bait.join("outside.txt"), b"outside").expect("outside");
    let public = PathBuf::from(r"C:\Users\Public");
    let done = public.join(format!("sl-plant-done-{}.txt", std::process::id()));
    let _ = fs::remove_file(&done);
    let modes = ["hold-file", "junction", "replant", "hold-dir"];
    let mut children = Vec::new();
    for mode in modes {
        let report = public.join(format!("sl-plant-{mode}-{}.txt", std::process::id()));
        let _ = fs::remove_file(&report);
        children.push(spawn_standard_user(
            &account, mode, &dir, &bait, &report, &done,
        ));
    }
    thread::sleep(Duration::from_millis(500));
    let service = service_exe();
    let (name, display) = service_names(&service);
    let probe = std::env::temp_dir().join(format!("svc-plant-{}.txt", std::process::id()));
    let body = install_service(
        &service,
        &name,
        &display,
        &probe,
        &["SOFTLOCK_PIN_SELFTEST=1".to_string()],
    );
    fs::write(&done, b"done").expect("done");
    for child in &children {
        wait_process(child.process, Duration::from_secs(20));
    }
    remove_service(&name);
    let reports: Vec<String> = modes
        .iter()
        .map(|mode| {
            fs::read_to_string(public.join(format!("sl-plant-{mode}-{}.txt", std::process::id())))
                .unwrap_or_else(|error| format!("missing report: {error}"))
        })
        .collect();
    for (mode, report) in modes.iter().zip(reports.iter()) {
        assert!(
            report.contains("il=8192"),
            "{mode} did not run at medium integrity: {report}"
        );
        assert!(
            report.contains("result=denied"),
            "{mode} planted the secret store: {report}"
        );
    }
    assert!(
        body.contains("pin_loaded=none"),
        "a planted hash was loaded: {body}"
    );
    assert!(
        body.contains("pin_reload=ok"),
        "the service did not create its own store: {body}"
    );
    assert_eq!(
        fs::read(bait.join("marker.txt")).expect("marker"),
        b"bait-marker"
    );
    assert_eq!(
        fs::read(bait.join("pin.phc")).expect("bait pin"),
        b"bait-hash"
    );
    assert_eq!(
        fs::read(bait.join("outside.txt")).expect("outside"),
        b"outside"
    );
    assert!(
        !bait.join("service.log").exists(),
        "the service wrote the log outside its directory"
    );
    assert!(
        is_reparse_point(&dir).ok() != Some(true),
        "the service directory is a junction"
    );
    assert_eq!(dacl_sddl(&dir).expect("directory dacl"), SYSTEM_ONLY_SDDL);
    let pin = dir.join("pin.phc");
    assert!(user_read_denied(&pin), "the hash is readable");
    assert_eq!(dacl_sddl(&pin).expect("pin dacl"), SYSTEM_ONLY_SDDL);
    let bytes = read_with_backup(&pin).expect("read hash as backup");
    assert_eq!(
        bytes,
        b"phc-round-2",
        "planted bytes were stored: {}",
        String::from_utf8_lossy(&bytes)
    );
    assert!(
        !bytes
            .windows(b"planted".len())
            .any(|window| window == b"planted"),
        "planted hash bytes were stored"
    );
    drop(children);
    let _ = fs::remove_file(&done);
    reclaim_data_dir(&dir);
}

impl Drop for PlantChild {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Threading::TerminateProcess(self.process, 1);
            let _ = windows::Win32::Foundation::CloseHandle(self.process);
        }
    }
}

fn service_names(service: &Path) -> (String, String) {
    let names = Command::new(service)
        .arg("--print-names")
        .output()
        .expect("names");
    let text = String::from_utf8_lossy(&names.stdout);
    let name = text
        .lines()
        .find_map(|line| line.strip_prefix("name="))
        .expect("service name")
        .trim()
        .to_string();
    let display = text
        .lines()
        .find_map(|line| line.strip_prefix("display="))
        .unwrap_or(name.as_str())
        .trim()
        .to_string();
    (name, display)
}

fn probe_value(body: &str, key: &str) -> String {
    body.lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .unwrap_or("")
        .trim()
        .to_string()
}

fn install_service(
    service: &Path,
    name: &str,
    display: &str,
    probe: &Path,
    env: &[String],
) -> String {
    let _ = fs::remove_file(probe);
    remove_service(name);
    thread::sleep(Duration::from_millis(500));
    // sc.exe takes everything after the binPath value as its own options.
    // The exe and `--scm` have to be one quoted ImagePath, or `--scm` is
    // parsed as an sc option and create prints its usage.
    let bin_path = format!(
        "binPath= \"{} --scm --probe {}\"",
        sc_image(service),
        sc_image(probe)
    );
    let created = Command::new("sc.exe")
        .raw_arg("create")
        .raw_arg(name)
        .raw_arg(bin_path)
        .raw_arg("start= demand")
        .raw_arg("obj= LocalSystem")
        .raw_arg(format!("DisplayName= \"{display}\""))
        .output()
        .expect("sc create");
    assert!(
        created.status.success(),
        "sc create failed: {} {}",
        String::from_utf8_lossy(&created.stdout),
        String::from_utf8_lossy(&created.stderr)
    );
    if !env.is_empty() {
        set_service_environment(name, env);
    }
    let started = Command::new("sc.exe")
        .args(["start", name])
        .output()
        .expect("sc start");
    let body = wait_for_file(probe, Duration::from_secs(25));
    assert!(
        started.status.success() || body.is_some(),
        "sc start failed: {} {}",
        String::from_utf8_lossy(&started.stdout),
        String::from_utf8_lossy(&started.stderr)
    );
    body.expect("the service did not write the session probe")
}

fn set_service_environment(name: &str, env: &[String]) {
    let values = env
        .iter()
        .map(|value| format!("'{value}'"))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "$p = 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\{name}'; New-ItemProperty -Path $p -Name Environment -PropertyType MultiString -Value @({values}) -Force | Out-Null"
    );
    let wrote = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .expect("service environment");
    assert!(
        wrote.status.success(),
        "service environment failed: {} {}",
        String::from_utf8_lossy(&wrote.stdout),
        String::from_utf8_lossy(&wrote.stderr)
    );
}

fn remove_service(name: &str) {
    if let Some(pid) = service_pid(name) {
        let _ = Command::new("taskkill.exe")
            .args(["/F", "/PID", &pid])
            .output();
    }
    let _ = Command::new("sc.exe").args(["stop", name]).output();
    thread::sleep(Duration::from_millis(300));
    let _ = Command::new("sc.exe").args(["delete", name]).output();
}

fn service_pid(name: &str) -> Option<String> {
    let output = Command::new("sc.exe")
        .args(["queryex", name])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("PID") {
            let pid = rest.trim().trim_start_matches(':').trim();
            if pid.chars().all(|ch| ch.is_ascii_digit()) && pid != "0" {
                return Some(pid.to_string());
            }
        }
    }
    None
}

fn transact(pipe: &str, json: &str) -> Result<String, String> {
    let handle = open_pipe(pipe)?;
    let mut frame = (json.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(json.as_bytes());
    write_all(handle, &frame)?;
    let response = read_frame(handle)?;
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(handle);
    }
    String::from_utf8(response).map_err(|error| error.to_string())
}

/// A path with a space is quoted inside the binPath value. sc.exe keeps
/// `\"` as a quote in that value, which is what the service control manager
/// needs in order to find the executable.
fn sc_image(path: &Path) -> String {
    let text = path.display().to_string();
    if text.contains(' ') {
        format!("\\\"{text}\\\"")
    } else {
        text
    }
}

fn physical_is_blocked(probe: &Probe, extra: u64) -> bool {
    let before = probe
        .inputs()
        .iter()
        .filter(|input| input.virtual_key == VK_F14)
        .count();
    if send_virtual_key(VK_F14, extra).is_err() {
        return false;
    }
    thread::sleep(Duration::from_millis(40));
    let after = probe
        .inputs()
        .iter()
        .filter(|input| input.virtual_key == VK_F14)
        .count();
    after == before
}

fn wait_for(timeout: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if ready() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    ready()
}

fn reclaim_data_dir(dir: &Path) {
    let text = dir.display().to_string();
    let _ = Command::new("takeown.exe")
        .args(["/F", &text, "/R", "/D", "Y"])
        .output();
    let _ = Command::new("icacls.exe")
        .args([&text, "/grant", "*S-1-1-0:(OI)(CI)F", "/T"])
        .output();
    if is_reparse_point(dir).ok() == Some(true) {
        let _ = Command::new("cmd.exe")
            .args(["/C", "rmdir", &text])
            .output();
    } else {
        let _ = fs::remove_dir_all(dir);
    }
    assert!(
        is_reparse_point(dir).ok() != Some(true) && !dir.exists(),
        "could not remove {}",
        dir.display()
    );
}

struct StandardAccount {
    name: String,
    password: String,
}

impl StandardAccount {
    fn create() -> Self {
        let name = format!("slp{}", std::process::id());
        // The account name must not appear in the password. The runner
        // rejects that as a complexity failure.
        let password = format!("Aa1!QkzzMm9{}", std::process::id() % 97);
        let _ = Command::new("net.exe")
            .args(["user", &name, "/delete"])
            .output();
        let added = Command::new("net.exe")
            .args(["user", &name, &password, "/add"])
            .output()
            .expect("net user");
        assert!(
            added.status.success(),
            "net user /add failed: {} {}",
            String::from_utf8_lossy(&added.stdout),
            String::from_utf8_lossy(&added.stderr)
        );
        Self { name, password }
    }
}

impl Drop for StandardAccount {
    fn drop(&mut self) {
        let _ = Command::new("net.exe")
            .args(["user", &self.name, "/delete"])
            .output();
    }
}

struct PlantChild {
    process: HANDLE,
}

fn spawn_standard_user(
    account: &StandardAccount,
    mode: &str,
    dir: &Path,
    bait: &Path,
    report: &Path,
    done: &Path,
) -> PlantChild {
    use windows::Win32::Security::{
        CreateWellKnownSid, DuplicateTokenEx, LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT,
        LogonUserW, PSID, SE_ASSIGNPRIMARYTOKEN_NAME, SE_INCREASE_QUOTA_NAME,
        SecurityImpersonation, TOKEN_ALL_ACCESS, TOKEN_MANDATORY_LABEL, TokenIntegrityLevel,
        TokenPrimary, WinMediumLabelSid,
    };
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, PROCESS_INFORMATION,
        STARTUPINFOW,
    };
    use windows::core::{PCWSTR, PWSTR};
    enable_privilege(SE_ASSIGNPRIMARYTOKEN_NAME);
    enable_privilege(SE_INCREASE_QUOTA_NAME);
    let user = account.name.clone();
    let password = account.password.clone();
    let user_w = wide_chars(&user);
    let password_w = wide_chars(&password);
    let domain = wide_chars(".");
    let mut token = HANDLE::default();
    unsafe {
        LogonUserW(
            PCWSTR(user_w.as_ptr()),
            PCWSTR(domain.as_ptr()),
            PCWSTR(password_w.as_ptr()),
            LOGON32_LOGON_INTERACTIVE,
            LOGON32_PROVIDER_DEFAULT,
            &mut token,
        )
        .expect("LogonUserW");
    }
    let mut primary = HANDLE::default();
    unsafe {
        DuplicateTokenEx(
            token,
            TOKEN_ALL_ACCESS,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        )
        .expect("DuplicateTokenEx");
        let _ = windows::Win32::Foundation::CloseHandle(token);
    }
    let mut sid = vec![0u8; 68];
    let mut sid_len = sid.len() as u32;
    unsafe {
        CreateWellKnownSid(
            WinMediumLabelSid,
            None,
            Some(PSID(sid.as_mut_ptr().cast())),
            &mut sid_len,
        )
        .expect("medium sid");
        let label = TOKEN_MANDATORY_LABEL {
            Label: windows::Win32::Security::SID_AND_ATTRIBUTES {
                Sid: PSID(sid.as_mut_ptr().cast()),
                Attributes: (windows::Win32::System::SystemServices::SE_GROUP_INTEGRITY
                    | windows::Win32::System::SystemServices::SE_GROUP_INTEGRITY_ENABLED)
                    as u32,
            },
        };
        let _ = windows::Win32::Security::SetTokenInformation(
            primary,
            TokenIntegrityLevel,
            (&label as *const TOKEN_MANDATORY_LABEL).cast(),
            std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + sid_len,
        );
    }
    let exe = std::env::current_exe().expect("test exe");
    let command = format!(
        "\"{}\" --exact standard_user_cannot_plant_the_secret_store --test-threads=1 --nocapture",
        exe.display()
    );
    let mut command_w = wide_chars(&command);
    let mut env = plant_environment(mode, dir, bait, report, done);
    let mut desktop = wide_chars(r"winsta0\default");
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        lpDesktop: PWSTR(desktop.as_mut_ptr()),
        ..STARTUPINFOW::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    let directory = wide_chars(r"C:\Users\Public");
    unsafe {
        CreateProcessAsUserW(
            Some(primary),
            PCWSTR::null(),
            Some(PWSTR(command_w.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            Some(env.as_mut_ptr().cast()),
            PCWSTR(directory.as_ptr()),
            &startup,
            &mut info,
        )
        .unwrap_or_else(|error| panic!("CreateProcessAsUserW {mode}: {error}"));
        let _ = windows::Win32::Foundation::CloseHandle(info.hThread);
        let _ = windows::Win32::Foundation::CloseHandle(primary);
    }
    PlantChild {
        process: info.hProcess,
    }
}

fn plant_environment(mode: &str, dir: &Path, bait: &Path, report: &Path, done: &Path) -> Vec<u16> {
    let mut block = String::new();
    for (key, value) in std::env::vars() {
        if key.starts_with("SOFTLOCK_PLANT_") {
            continue;
        }
        block.push_str(&key);
        block.push('=');
        block.push_str(&value);
        block.push('\0');
    }
    for (key, value) in [
        ("SOFTLOCK_PLANT_CHILD", mode),
        ("SOFTLOCK_PLANT_DIR", &dir.display().to_string()),
        ("SOFTLOCK_PLANT_BAIT", &bait.display().to_string()),
        ("SOFTLOCK_PLANT_REPORT", &report.display().to_string()),
        ("SOFTLOCK_PLANT_DONE", &done.display().to_string()),
    ] {
        block.push_str(key);
        block.push('=');
        block.push_str(value);
        block.push('\0');
    }
    block.push('\0');
    block.encode_utf16().collect()
}

fn plant_child(mode: &str) {
    let dir = PathBuf::from(std::env::var("SOFTLOCK_PLANT_DIR").unwrap_or_default());
    let bait = PathBuf::from(std::env::var("SOFTLOCK_PLANT_BAIT").unwrap_or_default());
    let report = PathBuf::from(std::env::var("SOFTLOCK_PLANT_REPORT").unwrap_or_default());
    let done = PathBuf::from(std::env::var("SOFTLOCK_PLANT_DONE").unwrap_or_default());
    let il = integrity_rid();
    let mut opened = false;
    let mut code = 0i32;
    let deadline = Instant::now() + Duration::from_secs(60);
    let held = loop {
        if done.exists() || Instant::now() > deadline {
            break None;
        }
        match mode {
            "hold-file" => match create_held_file(&dir.join("pin.phc")) {
                Ok(handle) => {
                    opened = true;
                    break Some(handle);
                }
                Err(error) => code = error.raw_os_error().unwrap_or(code),
            },
            "junction" => match create_junction(&dir, &bait) {
                Ok(()) => {
                    opened = true;
                    break None;
                }
                Err(error) => code = error.raw_os_error().unwrap_or(code),
            },
            "replant" => match plant_pin(&dir) {
                Ok(()) => {
                    opened = true;
                    break None;
                }
                Err(error) => code = error.raw_os_error().unwrap_or(code),
            },
            "hold-dir" => match open_dir_for_write(&dir) {
                Ok(handle) => {
                    opened = true;
                    break Some(handle);
                }
                Err(error) => code = error.raw_os_error().unwrap_or(code),
            },
            _ => break None,
        }
        thread::sleep(Duration::from_millis(20));
    };
    while !done.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    let result = if opened { "opened" } else { "denied" };
    let _ = fs::write(
        &report,
        format!("il={il}\nresult={result}\ncode={code}\nmode={mode}\n"),
    );
    if let Some(handle) = held {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
    }
}

fn integrity_rid() -> u32 {
    use windows::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TokenIntegrityLevel,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return 0;
        }
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut needed);
        let mut buffer = vec![0u8; needed as usize];
        if GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .is_err()
        {
            let _ = windows::Win32::Foundation::CloseHandle(token);
            return 0;
        }
        let label = &*(buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>());
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        let rid = *GetSidSubAuthority(label.Label.Sid, u32::from(count.saturating_sub(1)));
        let _ = windows::Win32::Foundation::CloseHandle(token);
        rid
    }
}

fn create_held_file(path: &Path) -> std::io::Result<HANDLE> {
    use windows::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_SHARE_MODE,
    };
    use windows::core::PCWSTR;
    let wide_path = wide_chars(&path.display().to_string());
    unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))
    }
}

fn plant_pin(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join("pin.phc"), b"planted-by-user")?;
    Ok(())
}

fn open_dir_for_write(path: &Path) -> std::io::Result<HANDLE> {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ADD_FILE, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_MODE, OPEN_EXISTING,
        WRITE_DAC,
    };
    use windows::core::PCWSTR;
    let wide_path = wide_chars(&path.display().to_string());
    unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_ADD_FILE.0 | WRITE_DAC.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))
    }
}

fn enable_privilege(name: windows::core::PCWSTR) {
    use windows::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .is_err()
        {
            return;
        }
        let mut luid = windows::Win32::Foundation::LUID::default();
        if LookupPrivilegeValueW(windows::core::PCWSTR::null(), name, &mut luid).is_err() {
            let _ = windows::Win32::Foundation::CloseHandle(token);
            return;
        }
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let _ = AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None);
        let _ = windows::Win32::Foundation::CloseHandle(token);
    }
}

fn wait_process(process: HANDLE, timeout: Duration) {
    use windows::Win32::System::Threading::WaitForSingleObject;
    unsafe {
        let _ = WaitForSingleObject(process, timeout.as_millis() as u32);
    }
}

fn read_with_backup(path: &Path) -> std::io::Result<Vec<u8>> {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING,
        ReadFile,
    };
    use windows::core::PCWSTR;
    enable_privilege(windows::Win32::Security::SE_BACKUP_NAME);
    let wide_path = wide_chars(&path.display().to_string());
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?
    };
    let mut buffer = vec![0u8; 64];
    let mut read = 0u32;
    unsafe {
        ReadFile(handle, Some(&mut buffer), Some(&mut read), None)
            .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
        let _ = windows::Win32::Foundation::CloseHandle(handle);
    }
    buffer.truncate(read as usize);
    Ok(buffer)
}

fn wide_chars(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn create_junction(link: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_GENERIC_WRITE, FILE_SHARE_READ, OPEN_EXISTING,
    };
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::core::PCWSTR;
    let wide_link: Vec<u16> = link
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe { CreateDirectoryW(PCWSTR(wide_link.as_ptr()), None) }
        .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
    let substitute: Vec<u16> = format!(r"\??\{}", target.display())
        .encode_utf16()
        .collect();
    let print: Vec<u16> = target.as_os_str().encode_wide().collect();
    let sub_bytes = substitute.len() * 2;
    let print_bytes = print.len() * 2;
    let mut buffer = Vec::<u8>::new();
    let reparse_data_length = (8 + sub_bytes + 2 + print_bytes + 2) as u16;
    buffer.extend_from_slice(&0xA0000003u32.to_le_bytes());
    buffer.extend_from_slice(&reparse_data_length.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&(sub_bytes as u16).to_le_bytes());
    buffer.extend_from_slice(&((sub_bytes + 2) as u16).to_le_bytes());
    buffer.extend_from_slice(&(print_bytes as u16).to_le_bytes());
    for unit in &substitute {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }
    buffer.extend_from_slice(&0u16.to_le_bytes());
    for unit in &print {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }
    buffer.extend_from_slice(&0u16.to_le_bytes());
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_link.as_ptr()),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
        .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?
    };
    let mut returned = 0u32;
    let set = unsafe {
        DeviceIoControl(
            handle,
            0x0009_00A4,
            Some(buffer.as_ptr().cast()),
            buffer.len() as u32,
            None,
            0,
            Some(&mut returned),
            None,
        )
    };
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(handle);
    }
    set.map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
    Ok(())
}

fn wait_for_file(path: &Path, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(text) = fs::read_to_string(path)
            && !text.is_empty()
        {
            return Some(text);
        }
        thread::sleep(Duration::from_millis(200));
    }
    fs::read_to_string(path).ok()
}
