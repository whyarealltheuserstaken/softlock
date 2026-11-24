//! Seam 3. The service owns the lock. The Lock Host is a child process.

#![cfg(all(windows, feature = "simulated-physical"))]

use std::fs;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, Once};
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
    static WATCHDOG: Once = Once::new();
    WATCHDOG.call_once(|| {
        thread::spawn(|| {
            // The suite finishes in about a minute on main. Anything still
            // running here is a wait the per-call deadlines did not cover.
            thread::sleep(Duration::from_secs(8 * 60));
            eprintln!(
                "service_session did not finish within 8 minutes, decline at {}",
                softlock_app_lib::decline_at()
            );
            std::process::exit(3);
        });
    });
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

struct Session {
    child: std::process::Child,
    pipe: String,
    log: PathBuf,
    pin_dir: PathBuf,
    config_dir: PathBuf,
    extra: u64,
}

impl Session {
    fn start(panic: Option<&str>, dev_app: bool) -> Self {
        Self::launch(panic, dev_app, "1", None)
    }

    fn launch(panic: Option<&str>, dev_app: bool, stub: &str, mark: Option<&Path>) -> Self {
        Self::launch_extra(panic, dev_app, stub, mark, &[])
    }

    fn launch_extra(
        panic: Option<&str>,
        dev_app: bool,
        stub: &str,
        mark: Option<&Path>,
        extra_env: &[(&str, &str)],
    ) -> Self {
        let _ = gate;
        // A host whose OS lock failed stays guarded after its service dies.
        // Kill any survivor before the next service starts.
        stop_lock_hosts();
        // A failed unlock leaves a journal that says locked. The next service
        // must not boot from that file: recovery fail-closes, and every later
        // test then looks already locked.
        static SESSIONS: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "svc-{}-{}",
            std::process::id(),
            SESSIONS.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp");
        let pin_dir = root.join("pin");
        let log = root.join("service.log");
        let journal_dir = root.join("journal");
        let config_dir = root.join("config");
        fs::create_dir_all(&pin_dir).expect("pin dir");
        fs::create_dir_all(&journal_dir).expect("journal dir");
        fs::create_dir_all(&config_dir).expect("config dir");
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
            .env("SOFTLOCK_STUB_OS_LOCK", stub)
            .env("SOFTLOCK_PIN_DIR", &pin_dir)
            .env("SOFTLOCK_LOG", &log)
            .env("SOFTLOCK_JOURNAL_DIR", &journal_dir)
            .env("SOFTLOCK_CONFIG_DIR", &config_dir)
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
        if let Some(mark) = mark {
            command.env("SOFTLOCK_OS_LOCK_MARK", mark);
        }
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let child = command.spawn().expect("service process");
        let session = Self {
            child,
            pipe,
            log,
            pin_dir,
            config_dir,
            extra,
        };
        // `hello` used to block inside `ReadFile`, so this deadline was never
        // checked and a silent service held the job until the runner killed it.
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut last_error = String::new();
        while Instant::now() < deadline {
            match session.hello() {
                Ok(_) => {
                    last_error.clear();
                    break;
                }
                Err(error) => last_error = error,
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(
            last_error.is_empty(),
            "timed out waiting for the service: {last_error}"
        );
        session
    }

    fn pipe(&self) -> &str {
        &self.pipe
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

    fn config(&self) -> Result<String, String> {
        self.transact(r#"{"v":1,"id":5,"kind":"GetConfig"}"#)
    }

    fn set_pin(&self) -> Result<String, String> {
        let pin = std::str::from_utf8(PIN).unwrap_or("");
        self.transact(&format!(
            r#"{{"v":1,"id":4,"kind":"SetPin","pin":"{pin}"}}"#
        ))
    }

    fn transact(&self, json: &str) -> Result<String, String> {
        let handle = open_pipe(&self.pipe)?;
        if let Err(error) = reject_untrusted_lock(&self.pipe, handle, json) {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
            }
            return Err(error);
        }
        let mut frame = (json.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(json.as_bytes());
        write_all(handle, &frame)?;
        let response = read_frame(handle)?;
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
        String::from_utf8(response).map_err(|error| error.to_string())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        wait_child(&mut self.child, Duration::from_secs(5));
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &self.child.id().to_string()])
            .output_bounded();
        // Quit while the OS lock is unavailable does not exit the host. The
        // test process must not leave it holding the runner's stdout pipe.
        stop_lock_hosts();
    }
}

struct KillChild(Option<std::process::Child>);

impl Drop for KillChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            wait_child(child, Duration::from_secs(5));
        }
    }
}

fn stop_lock_hosts() {
    let mut command = Command::new("taskkill");
    command
        .args(["/F", "/IM", "softlock-lockhost.exe"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(mut child) = command.spawn() else {
        return;
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.try_wait();
                return;
            }
        }
    }
}

fn wait_child(child: &mut std::process::Child, limit: Duration) {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if started.elapsed() < limit => thread::sleep(Duration::from_millis(50)),
            _ => return,
        }
    }
}

trait BoundedOutput {
    fn output_bounded(&mut self) -> std::io::Result<std::process::Output>;
}

impl BoundedOutput for Command {
    fn output_bounded(&mut self) -> std::io::Result<std::process::Output> {
        const LIMIT: Duration = Duration::from_secs(30);
        self.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = self.spawn()?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(take_output(status, &mut child)),
                Ok(None) if started.elapsed() < LIMIT => thread::sleep(Duration::from_millis(50)),
                Ok(None) => {
                    let _ = child.kill();
                    let killed = Instant::now();
                    loop {
                        match child.try_wait() {
                            Ok(Some(status)) => return Ok(take_output(status, &mut child)),
                            Ok(None) if killed.elapsed() < Duration::from_secs(2) => {
                                thread::sleep(Duration::from_millis(50));
                            }
                            _ => {
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::TimedOut,
                                    "command timed out after 30s",
                                ));
                            }
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }
}

fn take_output(
    status: std::process::ExitStatus,
    child: &mut std::process::Child,
) -> std::process::Output {
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if let Some(mut pipe) = stdout_pipe.take() {
            let _ = pipe.read_to_end(&mut stdout);
        }
        if let Some(mut pipe) = stderr_pipe.take() {
            let _ = pipe.read_to_end(&mut stderr);
        }
        let _ = sender.send((stdout, stderr));
    });
    // A grandchild that inherited the pipe keeps `read_to_end` blocked after
    // the process itself has already exited.
    let (stdout, stderr) = receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_default();
    std::process::Output {
        status,
        stdout,
        stderr,
    }
}

fn cargo_finished(command: &mut Command, what: &str) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().unwrap_or_else(|error| {
        panic!("{what} did not start: {error}");
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                assert!(status.success(), "{what} failed");
                return;
            }
            Ok(None) if started.elapsed() < Duration::from_secs(180) => {
                thread::sleep(Duration::from_millis(200));
            }
            Ok(None) => {
                let _ = child.kill();
                wait_child(&mut child, Duration::from_secs(5));
                panic!("{what} timed out after 180s");
            }
            Err(error) => panic!("{what} wait failed: {error}"),
        }
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
    cargo_finished(&mut command, "cargo build of softlock-service");
    assert!(service.is_file(), "missing {}", service.display());
    service
}

fn open_pipe(name: &str) -> Result<HANDLE, String> {
    let started = Instant::now();
    loop {
        match open_pipe_once(name) {
            Err(error)
                if (error.contains("0x800700E7") || error.contains("0x80070002"))
                    && started.elapsed() < Duration::from_secs(2) =>
            {
                // 231: every instance is busy. 2: the name has no instance
                // while the server is between clients.
                thread::sleep(Duration::from_millis(20));
            }
            other => return other,
        }
    }
}

fn open_pipe_once(name: &str) -> Result<HANDLE, String> {
    let name = name.to_string();
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(open_pipe_blocking(&name).map(|handle| handle.0 as isize));
    });
    // `PIPE_WAIT` makes `CreateFileW` itself block when every instance is taken.
    // The retry loop above never gets to check its deadline in that case.
    match receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(raw)) => Ok(HANDLE(raw as *mut core::ffi::c_void)),
        Ok(Err(error)) => Err(error),
        Err(_) => Err("timed out opening the service pipe".to_string()),
    }
}

fn open_pipe_blocking(name: &str) -> Result<HANDLE, String> {
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
    let bytes = bytes.to_vec();
    let raw = handle.0 as usize;
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let handle = HANDLE(raw as *mut core::ffi::c_void);
        let _ = sender.send(write_blocking(handle, &bytes));
    });
    // A server that never reads fills the pipe and `WriteFile` does not return.
    match receiver.recv_timeout(Duration::from_secs(15)) {
        Ok(result) => result,
        Err(_) => Err("timed out writing to the service".to_string()),
    }
}

fn write_blocking(handle: HANDLE, bytes: &[u8]) -> Result<(), String> {
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
    read_frame_within(handle, Duration::from_secs(15))
}

fn read_frame_within(handle: HANDLE, timeout: Duration) -> Result<Vec<u8>, String> {
    let raw = handle.0 as usize;
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let handle = HANDLE(raw as *mut core::ffi::c_void);
        let _ = sender.send(read_frame_blocking(handle));
    });
    // `PeekNamedPipe` and `ReadFile` run on the worker. A stuck call never
    // gets back to the deadline on this thread.
    match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => Err("timed out waiting for the service to answer".to_string()),
    }
}

fn read_frame_blocking(handle: HANDLE) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut prefix = [0u8; 4];
    read_until(handle, &mut prefix, deadline)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length > 64 * 1024 {
        return Err("frame too large".to_string());
    }
    let mut body = vec![0u8; length];
    read_until(handle, &mut body, deadline)?;
    Ok(body)
}

fn read_until(handle: HANDLE, buf: &mut [u8], deadline: Instant) -> Result<(), String> {
    let mut offset = 0;
    while offset < buf.len() {
        let mut available = 0u32;
        unsafe {
            windows::Win32::System::Pipes::PeekNamedPipe(
                handle,
                None,
                0,
                None,
                Some(&mut available),
                None,
            )
        }
        .map_err(|error| format!("read: {error}"))?;
        if available == 0 {
            if Instant::now() >= deadline {
                return Err("timed out waiting for the service to answer".to_string());
            }
            thread::sleep(Duration::from_millis(20));
            continue;
        }
        let want = (buf.len() - offset).min(available as usize);
        let mut read = 0u32;
        unsafe {
            ReadFile(
                handle,
                Some(&mut buf[offset..offset + want]),
                Some(&mut read),
                None,
            )
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
    let lost = wait_for(Duration::from_secs(30), || {
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
    let log_text = fs::read_to_string(&session.log).unwrap_or_default();
    assert!(
        !saw_unlocked,
        "the service reported Unlocked during recovery"
    );
    assert!(
        lost,
        "the service log did not record host-lost after the window panic: {log_text}"
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
fn service_kill_while_locked_stays_guarded_when_os_lock_fails() {
    kill_while_locked("fail", true);
}

#[test]
fn service_kill_while_locked_releases_only_after_os_lock() {
    kill_while_locked("succeed", false);
}

fn kill_while_locked(stub: &str, stay_blocked: bool) {
    let _guard = gate();
    let mark =
        std::env::temp_dir().join(format!("svc-os-lock-{}-{}.txt", stub, std::process::id()));
    let _ = fs::remove_file(&mark);
    let mut session = Session::launch(None, false, stub, Some(&mark));
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    session.lock().expect("lock");
    let before = session.status().expect("status");
    assert!(
        !before.contains("Unlocked"),
        "locked session reported Unlocked: {before}"
    );
    assert!(
        physical_is_blocked(&probe, session.extra),
        "lock did not block physical input"
    );
    session.child.kill().expect("kill service");
    wait_child(&mut session.child, Duration::from_secs(5));
    let mut passed_before_mark = false;
    let marked = wait_for(Duration::from_secs(8), || {
        if !mark.is_file() {
            let passed = physical_reached(&probe, session.extra);
            if passed && !mark.is_file() {
                passed_before_mark = true;
            }
        }
        mark.is_file()
    });
    assert!(
        !passed_before_mark,
        "physical input passed before the OS lock mark existed"
    );
    assert!(marked, "Quit did not call the OS lock");
    if stay_blocked {
        thread::sleep(Duration::from_millis(200));
        assert!(
            physical_is_blocked(&probe, session.extra),
            "hooks dropped after the service died without an OS lock"
        );
        assert!(
            lock_host_running(),
            "the lock host exited without a confirmed OS lock"
        );
    } else {
        assert!(
            wait_for(Duration::from_secs(8), || {
                physical_reached(&probe, session.extra)
            }),
            "physical input stayed blocked after the OS lock mark"
        );
        assert!(
            wait_for(Duration::from_secs(8), || !lock_host_running()),
            "hooks stayed installed after the OS lock"
        );
    }
    if let Ok(status) = session.status() {
        assert!(
            !status.contains("Unlocked"),
            "state reached Unlocked after the service died: {status}"
        );
    }
    let _ = Command::new("taskkill")
        .args(["/IM", "softlock-lockhost.exe", "/F"])
        .output_bounded();
    let _ = fs::remove_file(&mark);
}

#[test]
fn impostor_process_holds() {
    let Ok(done) = std::env::var("SOFTLOCK_IMPOSTOR_HOLD") else {
        return;
    };
    let done = PathBuf::from(done);
    if let Some(parent) = done.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(done.with_extension("ready"), b"up").expect("ready");
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(60) && !done.exists() {
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn impostor_pid_does_not_release_the_guard() {
    // The renamed copy is not SYSTEM. The host is the installed LocalSystem
    // service, so its armed event opens and only the token check can reject.
    impostor_case_system("token", "reason=token", true);
}

#[test]
fn impostor_path_does_not_release_the_guard() {
    // Foreground hosts are the interactive user, so a child they spawn fails
    // the token check first. The installed service is LocalSystem and the
    // host inherits that token, so only the other path can reject.
    impostor_case_system("path", "reason=path", false);
}

#[test]
fn reused_pid_is_not_trusted_again() {
    // Same constraint: only a SYSTEM host can hold a same-path process, kill
    // it, and prove that PID is not opened again.
    impostor_case_system("reuse", "dropped=1 reopened=0 rejected=1", false);
}

fn impostor_case_system(case: &str, expect: &str, spawn_user_copy: bool) {
    let _guard = gate();
    stop_lock_hosts();
    let service = service_exe();
    let (name, display) = service_names(&service);
    let nonce = RunNonce::random().expect("run nonce");
    set_run_nonce(nonce);
    let extra = nonce.as_extra_info();
    let root = std::env::temp_dir().join(format!("impostor-{case}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("impostor dir");
    let witness = root.join("witness");
    let mark = root.join("os-lock.txt");
    let probe = root.join("probe.txt");
    let log = root.join("service.log");
    let pid_file = root.join("pid");
    let mut env = vec![
        "SOFTLOCK_STUB_OS_LOCK=fail".to_string(),
        format!("SOFTLOCK_RUN_NONCE={extra}"),
        r"SOFTLOCK_INSTALL_DIR=C:\no\such\install".to_string(),
        format!("SOFTLOCK_LOG={}", log.display()),
        format!("SOFTLOCK_IMPOSTOR_CASE={case}"),
        format!("SOFTLOCK_IMPOSTOR_WITNESS={}", witness.display()),
        format!("SOFTLOCK_OS_LOCK_MARK={}", mark.display()),
    ];
    if spawn_user_copy {
        env.push(format!("SOFTLOCK_IMPOSTOR_PID={}", pid_file.display()));
    }
    let body = install_service(&service, &name, &display, &probe, &env);
    assert!(body.contains("system=yes"), "not LocalSystem: {body}");
    let probe_window = Probe::open().expect("probe");
    probe_window.claim_focus();
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
        physical_is_blocked(&probe_window, extra),
        "lock did not block physical input"
    );
    let user_copy = if spawn_user_copy {
        Some(spawn_user_impostor(&root))
    } else {
        None
    };
    let pid = service_pid(&name).expect("service pid");
    let _ = Command::new("taskkill.exe")
        .args(["/F", "/PID", &pid])
        .output_bounded();
    assert!(
        wait_for(Duration::from_secs(8), || mark.is_file()),
        "Quit did not call the OS lock"
    );
    if let Some((_, impostor_pid)) = &user_copy {
        fs::write(&pid_file, impostor_pid.to_string()).expect("pid file");
    }
    assert!(
        wait_for(Duration::from_secs(8), || witness.is_file()),
        "the host never judged the impostor"
    );
    if case == "reuse" {
        // A few host polls for a late reopen to rewrite the witness. The
        // host rewrites on every poll, and std::fs::write truncates first,
        // so a single read can observe an empty file. Empty is not a verdict.
        thread::sleep(Duration::from_millis(200));
    }
    let decision = non_empty_witness(&witness);
    assert_eq!(decision, expect, "impostor case {case} released the guard");
    thread::sleep(Duration::from_millis(200));
    assert!(
        physical_is_blocked(&probe_window, extra),
        "hooks dropped for an impostor ({case})"
    );
    if let Some((child, _)) = user_copy {
        drop(child);
    }
    remove_service(&name);
    stop_lock_hosts();
    let _ = fs::remove_dir_all(&root);
}

fn spawn_user_impostor(root: &std::path::Path) -> (KillChild, u32) {
    let done = root.join("done");
    let ready = done.with_extension("ready");
    let image = root.join("softlock-lockhost.exe");
    fs::copy(std::env::current_exe().expect("test exe"), &image).expect("copy impostor");
    assert!(
        image
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("softlock-lockhost.exe")),
        "the impostor image name is the one the old check trusted"
    );
    let impostor = Command::new(&image)
        .args([
            "--exact",
            "impostor_process_holds",
            "--test-threads=1",
            "--nocapture",
        ])
        .env("SOFTLOCK_IMPOSTOR_HOLD", &done)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("impostor process");
    let impostor_pid = impostor.id();
    assert!(
        wait_for(Duration::from_secs(8), || ready.is_file()),
        "impostor did not stay running"
    );
    (KillChild(Some(impostor)), impostor_pid)
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
            .output_bounded();
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
    if let Some(job) = plant_job_from_args() {
        plant_child(&job);
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
    let public = std::env::temp_dir().join(format!("sl-plant-{}", std::process::id()));
    let _ = fs::remove_dir_all(&public);
    fs::create_dir_all(&public).expect("report dir");
    grant_users_modify(&public);
    let done = public.join("done.txt");
    let _ = fs::remove_file(&done);
    let modes = ["hold-file", "junction", "replant", "hold-dir"];
    let mut children = Vec::new();
    for mode in modes {
        let report = public.join(format!("{mode}.txt"));
        let _ = fs::remove_file(&report);
        children.push(spawn_standard_user(
            &account,
            "standard_user_cannot_plant_the_secret_store",
            mode,
            &dir,
            &bait,
            &report,
            &done,
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
    let mut exits = Vec::new();
    for child in &children {
        wait_process(child.process, Duration::from_secs(20));
        exits.push(process_exit_code(child.process));
    }
    remove_service(&name);
    let reports: Vec<String> = modes
        .iter()
        .map(|mode| {
            fs::read_to_string(public.join(format!("{mode}.txt")))
                .unwrap_or_else(|error| format!("missing report: {error}"))
        })
        .collect();
    for (index, (mode, report)) in modes.iter().zip(reports.iter()).enumerate() {
        assert!(
            report.contains("il=8192"),
            "{mode} did not run at medium integrity (exit {}): {report}",
            exits.get(index).copied().unwrap_or(u32::MAX)
        );
        assert!(
            report.contains("result=denied"),
            "{mode} planted the secret store: {report}"
        );
        assert!(
            report.contains("code=5"),
            "{mode} was not access denied: {report}"
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

#[test]
fn standard_user_cannot_create_the_service_pipe() {
    if let Some(job) = plant_job_from_args() {
        plant_child(&job);
        return;
    }
    let _guard = gate();
    let account = StandardAccount::create();
    let service = service_exe();
    let (name, display) = service_names(&service);
    let probe = std::env::temp_dir().join(format!("svc-pipe-{}.txt", std::process::id()));
    let body = install_service(&service, &name, &display, &probe, &[]);
    assert!(body.contains("system=yes"), "not LocalSystem: {body}");
    let public = std::env::temp_dir().join(format!("sl-pipe-{}", std::process::id()));
    let _ = fs::remove_dir_all(&public);
    fs::create_dir_all(&public).expect("report dir");
    grant_users_modify(&public);
    let report = public.join("pipe.txt");
    let done = public.join("done.txt");
    let _ = fs::remove_file(&report);
    let _ = fs::remove_file(&done);
    let child = spawn_standard_user(
        &account,
        "standard_user_cannot_create_the_service_pipe",
        "pipe",
        Path::new(&pipe_name()),
        &public,
        &report,
        &done,
    );
    thread::sleep(Duration::from_millis(500));
    fs::write(&done, b"done").expect("done");
    wait_process(child.process, Duration::from_secs(20));
    let exit = process_exit_code(child.process);
    let text =
        fs::read_to_string(&report).unwrap_or_else(|error| format!("missing report: {error}"));
    remove_service(&name);
    assert!(
        text.contains("il=8192"),
        "pipe squat did not run at medium integrity (exit {exit}): {text}"
    );
    assert!(
        text.contains("result=denied"),
        "standard user created the service pipe: {text}"
    );
    assert!(
        text.contains("code=5"),
        "CreateNamedPipeW was not access denied: {text}"
    );
    drop(child);
    let _ = fs::remove_dir_all(&public);
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
        .output_bounded()
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
    thread::sleep(Duration::from_millis(200));
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
        .output_bounded()
        .expect("sc create");
    assert!(
        created.status.success(),
        "sc create failed: {} {}",
        String::from_utf8_lossy(&created.stdout),
        String::from_utf8_lossy(&created.stderr)
    );
    let journal = std::env::temp_dir().join(format!("{name}-journal"));
    let _ = fs::remove_dir_all(&journal);
    fs::create_dir_all(&journal).expect("journal dir");
    let mut service_env = env.to_vec();
    service_env.push(format!("SOFTLOCK_JOURNAL_DIR={}", journal.display()));
    set_service_environment(name, &service_env);
    let started = Command::new("sc.exe")
        .args(["start", name])
        .output_bounded()
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
        .output_bounded()
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
            .output_bounded();
    }
    let _ = Command::new("sc.exe").args(["stop", name]).output_bounded();
    thread::sleep(Duration::from_millis(300));
    let _ = Command::new("sc.exe")
        .args(["delete", name])
        .output_bounded();
}

fn service_pid(name: &str) -> Option<String> {
    let output = Command::new("sc.exe")
        .args(["queryex", name])
        .output_bounded()
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

fn reject_untrusted_lock(pipe: &str, handle: HANDLE, json: &str) -> Result<(), String> {
    if !json.contains("\"kind\":\"Lock\"") {
        return Ok(());
    }
    let trusted = if pipe == pipe_name() {
        softlock_service::server_is_system_agent(handle)
    } else {
        softlock_service::server_image_is_agent(handle)
    };
    if trusted {
        Ok(())
    } else {
        Err("lock server is not the system agent".to_string())
    }
}

fn transact(pipe: &str, json: &str) -> Result<String, String> {
    let handle = open_pipe(pipe)?;
    if let Err(error) = reject_untrusted_lock(pipe, handle, json) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
        return Err(error);
    }
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

fn lock_host_running() -> bool {
    let output = Command::new("tasklist")
        .args([
            "/FI",
            "IMAGENAME eq softlock-lockhost.exe",
            "/FO",
            "CSV",
            "/NH",
        ])
        .output_bounded();
    output.is_ok_and(|output| {
        String::from_utf8_lossy(&output.stdout)
            .to_ascii_lowercase()
            .contains("softlock-lockhost.exe")
    })
}

fn physical_reached(probe: &Probe, extra: u64) -> bool {
    let before = probe
        .inputs()
        .iter()
        .filter(|input| input.virtual_key == VK_F14)
        .count();
    if send_virtual_key(VK_F14, extra).is_err() {
        return false;
    }
    thread::sleep(Duration::from_millis(40));
    probe
        .inputs()
        .iter()
        .filter(|input| input.virtual_key == VK_F14)
        .count()
        > before
}

#[test]
fn changing_the_hotkey_makes_the_new_hotkey_lock() {
    let _guard = gate();
    softlock_windows::runner_may_lock().expect("refusing the hotkey change off a hosted runner");
    let session = Session::start(None, true);
    let toml = "[lock]\nhotkey = \"Ctrl+Alt+Shift+K\"\n";
    let put = format!(
        r#"{{"v":1,"id":8,"kind":"PutConfig","toml":"{}","base_revision":1}}"#,
        json_escape(toml)
    );
    let applied = session.transact(&put).expect("put");
    assert!(
        applied.contains("Applied"),
        "the new hotkey was not saved: {applied}"
    );
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let status_path = session.pin_dir.join("owner-status.txt");
    let _owner = KillChild(Some(spawn_owner(&session, &status_path)));
    assert!(
        wait_for(Duration::from_secs(20), || {
            let text = status_file(&status_path);
            text.contains("hotkey=ok") && text.contains("chord=Ctrl+Alt+Shift+K")
        }),
        "the app did not register the saved hotkey: {}",
        status_file(&status_path)
    );
    let before = status_seq(&status_file(&status_path));
    send_chord(u16::from(b'K'));
    assert!(
        wait_for(Duration::from_secs(20), || {
            let text = status_file(&status_path);
            status_seq(&text) > before && text.contains("state=Locked")
        }),
        "the saved hotkey did not lock: {}",
        status_file(&status_path)
    );
}

fn json_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[test]
fn global_hotkey_locks_and_the_probe_confirms() {
    let _guard = gate();
    let session = Session::start(None, true);
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let status_path = session.pin_dir.join("owner-status.txt");
    let _owner = KillChild(Some(spawn_owner(&session, &status_path)));
    assert!(
        wait_for(Duration::from_secs(20), || {
            status_file(&status_path).contains("hotkey=ok")
                && status_file(&status_path).contains("service=running")
        }),
        "the app did not register the hotkey: {}",
        status_file(&status_path)
    );
    let before = status_seq(&status_file(&status_path));
    send_default_hotkey();
    assert!(
        wait_for(Duration::from_secs(20), || {
            let text = status_file(&status_path);
            status_seq(&text) > before && text.contains("tray=on") && text.contains("state=Locked")
        }),
        "the hotkey did not lock: {}",
        status_file(&status_path)
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
        status.contains("Hotkey"),
        "GetStatus did not record the hotkey: {status}"
    );
    assert!(
        !status.contains("Unlocked"),
        "status reported Unlocked: {status}"
    );
}

#[test]
fn app_subscribe_shows_locked_then_unlocked_after_the_pin() {
    let _guard = gate();
    let session = Session::start(None, true);
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let stored = session.set_pin().expect("set pin");
    assert!(stored.contains("Ok"), "pin was not stored: {stored}");
    let status_path = session.pin_dir.join("owner-status.txt");
    let _owner = KillChild(Some(spawn_owner(&session, &status_path)));
    assert!(
        wait_for(Duration::from_secs(20), || {
            status_file(&status_path).contains("service=running")
        }),
        "the app did not subscribe: {}",
        status_file(&status_path)
    );
    let before = status_seq(&status_file(&status_path));
    let locked = session.lock().expect("lock");
    assert!(
        locked.contains("Accepted"),
        "lock was not accepted: {locked}"
    );
    assert!(
        wait_for(Duration::from_secs(15), || {
            let text = status_file(&status_path);
            status_seq(&text) > before && text.contains("tray=on") && text.contains("state=Locked")
        }),
        "subscribe did not show the lock: {}",
        status_file(&status_path)
    );
    let before_unlock = status_seq(&status_file(&status_path));
    type_pin(session.extra);
    assert!(
        wait_for(Duration::from_secs(30), || {
            let text = status_file(&status_path);
            status_seq(&text) > before_unlock
                && text.contains("state=Unlocked")
                && text.contains("tray=off")
        }),
        "the pin did not return the app to unlocked: {}",
        status_file(&status_path)
    );
    let status = session.status().expect("status");
    assert!(
        status.contains("Unlocked"),
        "GetStatus stayed locked: {status}"
    );
}

#[test]
fn app_quitting_leaves_the_service_running_for_a_lock_client() {
    let _guard = gate();
    let install = std::env::temp_dir().join(format!("softlock-app-inst-{}", std::process::id()));
    let _ = fs::remove_dir_all(&install);
    fs::create_dir_all(&install).expect("install dir");
    let dest = install.join("softlock-app.exe");
    fs::copy(app_exe(), &dest).expect("copy app");
    let install_text = install.display().to_string();
    let session = Session::launch_extra(
        None,
        false,
        "1",
        None,
        &[("SOFTLOCK_INSTALL_DIR", install_text.as_str())],
    );
    let status_path = session.pin_dir.join("owner-status.txt");
    let mut child = Command::new(&dest)
        .arg("--owner-session")
        .env("SOFTLOCK_PIPE", session.pipe())
        .env("SOFTLOCK_STATUS_FILE", &status_path)
        .env("SOFTLOCK_NO_AUTOSTART", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("app process");
    assert!(
        wait_for(Duration::from_secs(20), || {
            let text = status_file(&status_path);
            text.contains("service=running") && owner_hwnd(&text).is_some()
        }),
        "the owner session did not publish a window: {}",
        status_file(&status_path)
    );
    let hwnd = owner_hwnd(&status_file(&status_path)).expect("hwnd");
    post_owner(hwnd, WM_COMMAND, CMD_QUIT as isize);
    let deadline = Instant::now() + Duration::from_secs(5);
    let finished = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("Quit did not exit within 5s: {}", status_file(&status_path));
        }
        thread::sleep(Duration::from_millis(50));
    };
    assert!(finished.success(), "the app quit with {finished}");
    let hello = session.hello().expect("service still answers");
    assert!(
        hello.contains("Hello"),
        "quitting the app stopped the service: {hello}"
    );
    let locked = session.lock().expect("lock client");
    assert!(
        locked.contains("Accepted"),
        "a lock client could not lock after the app quit: {locked}"
    );
    let _ = fs::remove_dir_all(&install);
}

#[test]
fn tray_command_and_app_lock_show_sources_in_get_status() {
    let _guard = gate();
    assert_lock_source(WM_COMMAND, CMD_LOCK as isize, "Tray");
    assert_lock_source(WM_LOCK_APP, 0, "App");
}

#[test]
fn hotkey_unavailable_is_shown_when_registration_fails() {
    let _guard = gate();
    let session = Session::start(None, true);
    let first_path = session.pin_dir.join("hotkey-first.txt");
    let second_path = session.pin_dir.join("hotkey-second.txt");
    let _first = KillChild(Some(spawn_owner(&session, &first_path)));
    assert!(
        wait_for(Duration::from_secs(20), || {
            status_file(&first_path).contains("hotkey=ok")
        }),
        "the first owner did not register the hotkey: {}",
        status_file(&first_path)
    );
    let _second = KillChild(Some(spawn_owner(&session, &second_path)));
    assert!(
        wait_for(Duration::from_secs(20), || {
            let text = status_file(&second_path);
            text.contains("hotkey=failed") && text.contains("notice=hotkey unavailable")
        }),
        "a failed hotkey was not shown: {}",
        status_file(&second_path)
    );
}

fn assert_lock_source(message: u32, wparam: isize, source: &str) {
    let session = Session::start(None, true);
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let status_path = session.pin_dir.join(format!("source-{source}.txt"));
    let _owner = KillChild(Some(spawn_owner(&session, &status_path)));
    assert!(
        wait_for(Duration::from_secs(20), || owner_hwnd(&status_file(
            &status_path
        ))
        .is_some()),
        "no owner window: {}",
        status_file(&status_path)
    );
    let hwnd = owner_hwnd(&status_file(&status_path)).expect("hwnd");
    let before = status_seq(&status_file(&status_path));
    post_owner(hwnd, message, wparam);
    assert!(
        wait_for(Duration::from_secs(20), || {
            session
                .status()
                .ok()
                .is_some_and(|status| status.contains(&format!("\"{source}\"")))
                && status_seq(&status_file(&status_path)) > before
        }),
        "GetStatus did not record {source}: {:?} file {}",
        session.status().ok(),
        status_file(&status_path)
    );
}

fn spawn_owner(session: &Session, status_path: &Path) -> std::process::Child {
    Command::new(app_exe())
        .arg("--owner-session")
        .env("SOFTLOCK_PIPE", session.pipe())
        .env("SOFTLOCK_STATUS_FILE", status_path)
        .env("SOFTLOCK_NO_AUTOSTART", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("owner session")
}

fn app_exe() -> PathBuf {
    static BUILD: Mutex<()> = Mutex::new(());
    let _build = BUILD.lock().unwrap_or_else(|poison| poison.into_inner());
    let host = PathBuf::from(env!("CARGO_BIN_EXE_softlock-lockhost"));
    let app = host.with_file_name("softlock-app.exe");
    let mut command = Command::new("cargo");
    command.args([
        "build",
        "-p",
        "softlock-app",
        "--bin",
        "softlock-app",
        "--features",
        "simulated-physical",
    ]);
    if host.to_string_lossy().contains("x86_64-pc-windows-msvc") {
        command.args(["--target", "x86_64-pc-windows-msvc"]);
    }
    cargo_finished(&mut command, "cargo build of softlock-app");
    assert!(app.is_file(), "missing {}", app.display());
    app
}

fn status_file(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn status_seq(text: &str) -> u64 {
    text.lines()
        .find_map(|line| line.strip_prefix("seq=")?.parse().ok())
        .unwrap_or(0)
}

fn owner_hwnd(text: &str) -> Option<isize> {
    let raw = text.lines().find_map(|line| line.strip_prefix("hwnd="))?;
    let value: isize = raw.parse().ok()?;
    if value == 0 { None } else { Some(value) }
}

const WM_COMMAND: u32 = 0x0111;
const WM_LOCK_APP: u32 = 0x8002;
const CMD_LOCK: usize = 10;
const CMD_QUIT: usize = 11;

fn post_owner(hwnd: isize, message: u32, wparam: isize) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    let posted = unsafe {
        PostMessageW(
            Some(HWND(hwnd as *mut core::ffi::c_void)),
            message,
            WPARAM(wparam as usize),
            LPARAM(0),
        )
    };
    assert!(posted.is_ok(), "PostMessage failed");
}

#[test]
fn a_closed_subscriber_frees_its_stream_slot() {
    let _guard = gate();
    let session = Session::start(None, true);
    let mut held = Vec::new();
    for index in 0..8 {
        held.push(subscribe_live(&session, index));
    }
    let extra = open_pipe(&session.pipe).expect("ninth pipe");
    write_all(extra, &subscribe_frame(90)).expect("ninth subscribe");
    let accepted = read_frame(extra).expect("ninth accepted");
    let accepted = String::from_utf8_lossy(&accepted);
    assert!(
        accepted.contains("Accepted"),
        "the extra subscriber was not answered: {accepted}"
    );
    let closed = read_frame_deadline(extra, Duration::from_secs(2));
    assert!(
        closed.is_err(),
        "the ninth subscriber stayed open past the cap: {closed:?}"
    );
    for handle in held {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
    }
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(extra);
    }
    assert!(
        wait_for(Duration::from_secs(3), || {
            subscribe_receives_state(&session)
        }),
        "a freed subscriber slot did not accept a new stream"
    );
}

fn subscribe_frame(id: u64) -> Vec<u8> {
    let json = format!(r#"{{"v":1,"id":{id},"kind":"Subscribe"}}"#);
    let mut frame = (json.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(json.as_bytes());
    frame
}

fn subscribe_live(session: &Session, index: u64) -> HANDLE {
    let handle = open_pipe(session.pipe()).expect("subscribe pipe");
    write_all(handle, &subscribe_frame(10 + index)).expect("subscribe");
    let first = read_frame(handle).expect("accepted");
    assert!(
        String::from_utf8_lossy(&first).contains("Accepted"),
        "subscribe was not accepted"
    );
    let state = read_frame(handle).expect("state");
    assert!(
        String::from_utf8_lossy(&state).contains("StateChanged"),
        "the stream did not send the state"
    );
    handle
}

fn subscribe_receives_state(session: &Session) -> bool {
    let Ok(handle) = open_pipe(session.pipe()) else {
        return false;
    };
    if write_all(handle, &subscribe_frame(200)).is_err() {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
        return false;
    }
    let ok = read_frame_deadline(handle, Duration::from_secs(2))
        .ok()
        .is_some_and(|frame| String::from_utf8_lossy(&frame).contains("Accepted"))
        && read_frame_deadline(handle, Duration::from_secs(2))
            .ok()
            .is_some_and(|frame| String::from_utf8_lossy(&frame).contains("StateChanged"));
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(handle);
    }
    ok
}

fn read_frame_deadline(handle: HANDLE, timeout: Duration) -> Result<Vec<u8>, String> {
    read_frame_within(handle, timeout)
}

fn send_input_within(inputs: &[windows::Win32::UI::Input::KeyboardAndMouse::INPUT]) -> u32 {
    use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, SendInput};
    let inputs = inputs.to_vec();
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        let _ = sender.send(sent);
    });
    // A hook that does not pump makes `SendInput` wait forever. Fail the test.
    receiver.recv_timeout(Duration::from_secs(3)).unwrap_or(0)
}

fn send_chord(key: u16) {
    let keys = [0xA2u16, 0xA4, 0xA0, key];
    let mut inputs = Vec::new();
    for key in keys {
        inputs.push(chord_key(key, false));
    }
    for key in keys.into_iter().rev() {
        inputs.push(chord_key(key, true));
    }
    let sent = send_input_within(&inputs);
    assert_eq!(
        sent,
        inputs.len() as u32,
        "SendInput did not deliver the hotkey"
    );
}

fn send_default_hotkey() {
    send_chord(u16::from(b'L'));
}

fn chord_key(virtual_key: u16, up: bool) -> windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(virtual_key),
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
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

/// First non-empty snapshot. A rewrite can truncate the file for an instant.
/// An empty result after the deadline is still a failure for the caller.
fn non_empty_witness(path: &std::path::Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let text = fs::read_to_string(path).unwrap_or_default();
        if !text.is_empty() || Instant::now() >= deadline {
            return text;
        }
        thread::sleep(Duration::from_millis(20));
    }
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
        .output_bounded();
    let _ = Command::new("icacls.exe")
        .args([&text, "/grant", "*S-1-1-0:(OI)(CI)F", "/T"])
        .output_bounded();
    if is_reparse_point(dir).ok() == Some(true) {
        let _ = Command::new("cmd.exe")
            .args(["/C", "rmdir", &text])
            .output_bounded();
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
            .output_bounded();
        let added = Command::new("net.exe")
            .args(["user", &name, &password, "/add"])
            .output_bounded()
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
            .output_bounded();
    }
}

struct PlantChild {
    process: HANDLE,
}

fn spawn_standard_user(
    account: &StandardAccount,
    test_name: &str,
    mode: &str,
    dir: &Path,
    bait: &Path,
    report: &Path,
    done: &Path,
) -> PlantChild {
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_PROCESS_LOGON_FLAGS, CREATE_UNICODE_ENVIRONMENT,
        CreateProcessWithLogonW, PROCESS_INFORMATION, STARTUPINFOW,
    };
    use windows::core::{PCWSTR, PWSTR};
    // CreateProcessAsUserW needs SeAssignPrimaryToken, which the runner does
    // not hold. CreateProcessWithLogonW goes through the secondary logon
    // service and still yields the account's medium token.
    let user_w = wide_chars(&account.name);
    let password_w = wide_chars(&account.password);
    let domain = wide_chars(".");
    let exe = std::env::current_exe().expect("test exe");
    let command = format!(
        "\"{}\" --exact {} --test-threads=1 --nocapture -- {} \"{}\" \"{}\" \"{}\" \"{}\"",
        exe.display(),
        test_name,
        mode,
        dir.display(),
        bait.display(),
        report.display(),
        done.display()
    );
    let mut command_w = wide_chars(&command);
    let env = plant_environment(mode, dir, bait, report, done);
    // A desktop name hangs the new logon session: it never reaches main,
    // so the report file stays missing and the process is still active.
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..STARTUPINFOW::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    let directory = wide_chars(r"C:\Users\Public");
    unsafe {
        CreateProcessWithLogonW(
            PCWSTR(user_w.as_ptr()),
            PCWSTR(domain.as_ptr()),
            PCWSTR(password_w.as_ptr()),
            CREATE_PROCESS_LOGON_FLAGS(0),
            PCWSTR::null(),
            Some(PWSTR(command_w.as_mut_ptr())),
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            Some(env.as_ptr().cast()),
            PCWSTR(directory.as_ptr()),
            &startup,
            &mut info,
        )
        .unwrap_or_else(|error| panic!("CreateProcessWithLogonW {mode}: {error}"));
        let _ = windows::Win32::Foundation::CloseHandle(info.hThread);
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

struct PlantJob {
    mode: String,
    dir: PathBuf,
    bait: PathBuf,
    report: PathBuf,
    done: PathBuf,
}

fn grant_users_modify(path: &Path) {
    let text = path.display().to_string();
    let _ = Command::new("icacls.exe")
        .args([&text, "/grant", "*S-1-1-0:(OI)(CI)M"])
        .output_bounded();
}

fn plant_job_from_args() -> Option<PlantJob> {
    let mut args = std::env::args().skip_while(|arg| arg != "--");
    args.next()?;
    Some(PlantJob {
        mode: args.next()?,
        dir: PathBuf::from(args.next()?),
        bait: PathBuf::from(args.next()?),
        report: PathBuf::from(args.next()?),
        done: PathBuf::from(args.next()?),
    })
}

fn process_exit_code(process: HANDLE) -> u32 {
    use windows::Win32::System::Threading::GetExitCodeProcess;
    let mut code = 0u32;
    unsafe {
        let _ = GetExitCodeProcess(process, &mut code);
    }
    code
}

fn plant_child(job: &PlantJob) {
    let dir = &job.dir;
    let bait = &job.bait;
    let report = &job.report;
    let done = &job.done;
    let mode = job.mode.as_str();
    let il = integrity_rid();
    let _ = fs::write(
        report,
        format!("il={il}\nresult=running\ncode=0\nmode={mode}\n"),
    );
    let mut opened = false;
    let mut code = 0i32;
    let mut saw_denied = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    let held = loop {
        if done.exists() || Instant::now() > deadline {
            break None;
        }
        match mode {
            "hold-file" => {
                // CREATE_NEW on the existing hash returns file-exists (80).
                // A new name in the same directory is access-denied (5) once
                // LocalSystem has created it, and path-not-found before that.
                let attempts = [dir.join("pin.phc"), dir.join("user-plant.phc")];
                let mut planted = None;
                for path in &attempts {
                    match create_held_file(path) {
                        Ok(handle) => {
                            opened = true;
                            planted = Some(handle);
                            break;
                        }
                        Err(error) => record_plant_error(&mut code, &mut saw_denied, error),
                    }
                }
                if planted.is_some() {
                    break planted;
                }
            }
            "junction" => match create_junction(dir, bait) {
                Ok(()) => {
                    opened = true;
                    break None;
                }
                Err(error) => record_plant_error(&mut code, &mut saw_denied, error),
            },
            "replant" => match plant_pin(dir) {
                Ok(()) => {
                    opened = true;
                    break None;
                }
                Err(error) => record_plant_error(&mut code, &mut saw_denied, error),
            },
            "hold-dir" => match open_dir_for_write(dir) {
                Ok(handle) => {
                    opened = true;
                    break Some(handle);
                }
                Err(error) => record_plant_error(&mut code, &mut saw_denied, error),
            },
            "pipe" => match create_pipe_instance(dir) {
                Ok(handle) => {
                    opened = true;
                    unsafe {
                        let _ = windows::Win32::Foundation::CloseHandle(handle);
                    }
                    break None;
                }
                Err(error) => {
                    record_plant_error(&mut code, &mut saw_denied, error);
                    break None;
                }
            },
            _ => break None,
        }
        thread::sleep(Duration::from_millis(20));
    };
    while !done.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    let result = if opened { "opened" } else { "denied" };
    let reported = if saw_denied { 5 } else { code };
    let _ = fs::write(
        report,
        format!("il={il}\nresult={result}\ncode={reported}\nmode={mode}\n"),
    );
    if let Some(handle) = held {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
    }
}

fn record_plant_error(code: &mut i32, saw_denied: &mut bool, error: std::io::Error) {
    *code = win32_code(error.raw_os_error().unwrap_or(*code));
    if *code == 5 {
        *saw_denied = true;
    }
}

/// `windows::core::Error::code()` is an HRESULT (`0x8007xxxx`). The plant
/// report has to name the Win32 code, and a later path-not-found must not
/// erase an access-denied that already happened.
fn win32_code(raw: i32) -> i32 {
    let unsigned = raw as u32;
    if unsigned & 0xFFFF_0000 == 0x8007_0000 {
        (unsigned & 0xFFFF) as i32
    } else {
        raw
    }
}

fn create_pipe_instance(name: &Path) -> std::io::Result<HANDLE> {
    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    let text = name.to_string_lossy();
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(wide.as_ptr()),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            4096,
            0,
            None,
        )
    };
    if handle.is_invalid() {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(handle)
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

#[test]
fn a_locked_file_edit_is_offered_and_applied_on_accept() {
    softlock_windows::runner_may_lock().expect("refusing the config edit off a hosted runner");
    let _guard = gate();
    let session = within("locked_launch", Duration::from_secs(60), || {
        Session::start(None, true)
    });
    arm_owner(&session);
    let stored = session.set_pin().expect("set pin");
    assert!(stored.contains("Ok"), "pin was not stored: {stored}");
    let locked = session.lock().expect("lock");
    assert!(
        locked.contains("Accepted"),
        "lock was not accepted: {locked}"
    );
    assert!(
        softlock_app_lib::refresh_from_service(),
        "timed out waiting for the service to answer GetStatus"
    );
    let path = session.config_dir.join("config.toml");
    let original = wait_for_file(&path, Duration::from_secs(10)).expect("config file");
    let edited = original.replace("Ctrl+Alt+Shift+L", "Ctrl+Alt+Shift+K");
    assert_ne!(edited, original, "the default hotkey was not in the file");
    write_file_bounded(&path, edited.as_bytes()).expect("write");
    let offered = within("poll_offer", Duration::from_secs(60), || {
        wait_for(Duration::from_secs(10), || {
            softlock_app_lib::poll_config_once();
            softlock_app_lib::config_offer()
        })
    });
    assert!(
        offered,
        "the locked edit did not raise the offer\nstatus {}\nconfig {}\ndisk {}\nlog {}",
        session.status().unwrap_or_default(),
        session.config().unwrap_or_default(),
        read_file_bounded(&path).unwrap_or_default(),
        log_tail(&session.log)
    );
    let during = session.config().expect("config");
    assert!(
        during.contains("Ctrl+Alt+Shift+L") && !during.contains("Ctrl+Alt+Shift+K"),
        "the new value was in effect while locked: {during}"
    );
    type_pin(session.extra);
    let unlocked = wait_for(Duration::from_secs(30), || {
        session
            .status()
            .ok()
            .is_some_and(|status| status.contains("\"state\":\"Unlocked\""))
    });
    let status = session.status().unwrap_or_default();
    assert!(
        unlocked && status.contains("\"state\":\"Unlocked\""),
        "the pin did not unlock after the accepted locked edit: {status}"
    );
    assert!(
        softlock_app_lib::refresh_from_service(),
        "timed out waiting for the service to answer GetStatus after unlock"
    );
    within("accept", Duration::from_secs(60), || {
        assert!(
            softlock_app_lib::accept_offered_config(),
            "PutConfig did not return Applied"
        );
    });
    let after = session.config().expect("config");
    assert!(
        after.contains("Ctrl+Alt+Shift+K"),
        "the accepted value is not effective: {after}"
    );
}

#[test]
fn a_declined_file_edit_restores_the_file_and_keeps_the_old_value() {
    softlock_windows::runner_may_lock().expect("refusing the config edit off a hosted runner");
    let _guard = gate();
    // Ask the service to lock this file's DACL the same way it locks the
    // real profile. The other seam sessions leave the private directory
    // unlocked because a failed protected DACL would fail-close every lock.
    // Each step runs on its own thread. A stuck `CreateFile` or `ReadFile`
    // inside the step must fail here, not hold the suite until the watchdog.
    let session = within("launch", Duration::from_secs(60), || {
        Session::launch_extra(None, true, "1", None, &[("SOFTLOCK_LOCK_CONFIG_ACL", "1")])
    });
    arm_owner(&session);
    let session = within("set_pin", Duration::from_secs(20), move || {
        session.set_pin().expect("set pin");
        session
    });
    let session = within("lock", Duration::from_secs(20), move || {
        let locked = session.lock().expect("lock");
        assert!(
            locked.contains("Accepted"),
            "lock was not accepted: {locked}"
        );
        session
    });
    within("refresh", Duration::from_secs(25), || {
        assert!(
            softlock_app_lib::refresh_from_service(),
            "timed out waiting for the service to answer GetStatus"
        );
    });
    let path = session.config_dir.join("config.toml");
    let original = within("wait_for_file", Duration::from_secs(20), {
        let path = path.clone();
        move || wait_for_file(&path, Duration::from_secs(10)).expect("config file")
    });
    let edited = original.replace("Ctrl+Alt+Shift+L", "Ctrl+Alt+Shift+K");
    let denied = within("write_locked", Duration::from_secs(10), {
        let path = path.clone();
        let edited = edited.clone();
        move || write_file_bounded(&path, edited.as_bytes())
    });
    assert!(
        denied.is_err(),
        "the locked config file still accepted a write"
    );
    within("decline_locked", Duration::from_secs(30), || {
        assert!(
            !softlock_app_lib::decline_offered_config(),
            "decline wrote the config file while locked"
        );
    });
    let disk = within("read_locked", Duration::from_secs(10), {
        let path = path.clone();
        move || read_file_bounded(&path).expect("locked file")
    });
    assert_eq!(disk, original, "decline changed the file while locked");
    let extra = session.extra;
    within("type_pin", Duration::from_secs(30), move || type_pin(extra));
    let unlocked = within("wait_unlocked", Duration::from_secs(45), {
        let pipe = session.pipe.clone();
        move || {
            wait_for(Duration::from_secs(30), || {
                transact(&pipe, r#"{"v":1,"id":3,"kind":"GetStatus"}"#)
                    .ok()
                    .is_some_and(|status| status.contains("\"state\":\"Unlocked\""))
            })
        }
    });
    let status = session.status().unwrap_or_default();
    assert!(
        unlocked && status.contains("\"state\":\"Unlocked\""),
        "the pin did not unlock after the declined locked edit: {status}\nlog {}",
        log_tail(&session.log)
    );
    within("refresh_unlocked", Duration::from_secs(25), || {
        assert!(
            softlock_app_lib::refresh_from_service(),
            "timed out waiting for the service to answer GetStatus after unlock"
        );
    });
    within("write_unlocked", Duration::from_secs(10), {
        let path = path.clone();
        let edited = edited.clone();
        move || write_file_bounded(&path, edited.as_bytes()).expect("write after unlock")
    });
    // An unlocked edit is saved, not offered. Decline writes the effective
    // document back over the disk. Polling first would PutConfig the new
    // bytes and there would be nothing left to restore.
    // The open walks one path component at a time, gives up, then tries one
    // full-path open. Those deadlines sit inside this step.
    within("decline_unlocked", Duration::from_secs(90), || {
        assert!(
            softlock_app_lib::decline_offered_config(),
            "decline did not restore the file after unlock"
        );
    });
    assert!(
        !softlock_app_lib::config_offer(),
        "an unlocked edit raised the locked-edit offer"
    );
    let disk = read_file_bounded(&path).expect("restored file");
    assert!(
        disk.contains("Ctrl+Alt+Shift+L") && !disk.contains("Ctrl+Alt+Shift+K"),
        "decline did not restore the file: {disk}"
    );
    let after = session.config().expect("config");
    assert!(
        after.contains("Ctrl+Alt+Shift+L") && !after.contains("Ctrl+Alt+Shift+K"),
        "decline changed the effective config: {after}"
    );
}

fn log_tail(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let start = text.len().saturating_sub(800);
    let start = (start..=text.len())
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(text.len());
    text[start..].to_string()
}

fn arm_owner(session: &Session) {
    unsafe {
        std::env::set_var("SOFTLOCK_PIPE", session.pipe());
        std::env::set_var("SOFTLOCK_CONFIG_DIR", &session.config_dir);
        std::env::set_var("SOFTLOCK_NO_AUTOSTART", "1");
    }
}

fn wait_for_file(path: &Path, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(text) = read_file_bounded(path)
            && !text.is_empty()
        {
            return Some(text);
        }
        thread::sleep(Duration::from_millis(200));
    }
    read_file_bounded(path).ok()
}

/// Runs `work` off the test thread. `thread::scope` would join that worker
/// and wait out a stuck call, so the worker is detached and this thread is
/// the one that fails.
fn within<T: Send + 'static>(
    step: &'static str,
    limit: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
        let _ = sender.send(result);
    });
    match receiver.recv_timeout(limit) {
        Ok(Ok(value)) => value,
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => panic!(
            "timed out during {step}, decline at {}",
            softlock_app_lib::decline_at()
        ),
    }
}

fn read_file_bounded(path: &Path) -> Result<String, String> {
    let path = path.to_path_buf();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(fs::read_to_string(path));
    });
    match receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("timed out reading the config file".to_string()),
    }
}

fn write_file_bounded(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let path = path.to_path_buf();
    let bytes = bytes.to_vec();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(fs::write(path, bytes));
    });
    match receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("timed out writing the config file".to_string()),
    }
}
