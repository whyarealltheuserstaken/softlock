//! Windows pipe, Service Control Manager, and Lock Host spawn.
//!
//! The public pipe is local. A client whose session is not the active console
//! session is rejected. The Lock Host is a child process with inherited pipe
//! handles. Its nonce goes in the first Welcome message, not on the command line.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use softlock_core::{
    Event, LockState, NonceBytes, OsLockError, ServiceToHost, SessionEvent, decode_host_frame,
    encode_frame, encode_service, message_json, read_prefixed_frame, write_prefixed_frame,
};
use windows::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_BROKEN_PIPE, ERROR_PIPE_CONNECTED,
    GetLastError, HANDLE, HLOCAL, LocalFree, WAIT_TIMEOUT,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows::Win32::Security::{
    CheckTokenMembership, CreateWellKnownSid, DuplicateTokenEx, EqualSid, GetTokenInformation,
    PSECURITY_DESCRIPTOR, PSID, SE_ASSIGNPRIMARYTOKEN_NAME, SE_INCREASE_QUOTA_NAME, SE_TCB_NAME,
    SECURITY_ATTRIBUTES, SetTokenInformation, TOKEN_ALL_ACCESS, TOKEN_QUERY, TOKEN_USER,
    TokenPrimary, TokenSessionId, TokenUser, WinLocalSystemSid,
};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, CreatePipe, DisconnectNamedPipe,
    GetNamedPipeClientProcessId, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, PeekNamedPipe,
};
use windows::Win32::System::RemoteDesktop::{
    WTSGetActiveConsoleSessionId, WTSSESSION_NOTIFICATION,
};
use windows::Win32::System::Services::{
    RegisterServiceCtrlHandlerExW, SERVICE_ACCEPT_SESSIONCHANGE, SERVICE_ACCEPT_STOP,
    SERVICE_CONTROL_SESSIONCHANGE, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_STATUS,
    SERVICE_STATUS_HANDLE, SERVICE_STOP_PENDING, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW,
    SERVICE_WIN32_OWN_PROCESS, SetServiceStatus, StartServiceCtrlDispatcherW,
};
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, GetCurrentProcess, GetCurrentProcessId, GetProcessId,
    OpenProcess, OpenProcessToken, OpenThread, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, STARTUPINFOW,
    THREAD_QUERY_LIMITED_INFORMATION, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{WTS_SESSION_LOCK, WTS_SESSION_UNLOCK};
use windows::core::{PCWSTR, PWSTR};

use crate::authority::{Authority, HostAction, HostLoss};
use crate::log::FileLog;
use crate::platform::{ArmWait, OsLock, ServicePlatform};
use crate::secrets::PinStore;
use crate::{
    admit_client, dev_pin_allowed, log_path, pin_directory, pipe_path, role_for_image, stub_os_lock,
};

static STOP: AtomicBool = AtomicBool::new(false);
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static STATUS: AtomicIsize = AtomicIsize::new(0);
static HOST_USER: Mutex<String> = Mutex::new(String::new());
static HOST_SESSION: AtomicU64 = AtomicU64::new(u64::MAX);
static PIN_STATUS: Mutex<String> = Mutex::new(String::new());
static PIN_LOADED: Mutex<String> = Mutex::new(String::new());
static SESSION_TX: Mutex<Option<Sender<SessionEvent>>> = Mutex::new(None);

const ACCEPT_CONTROLS: u32 = SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SESSIONCHANGE;

pub fn console_session() -> u32 {
    unsafe { WTSGetActiveConsoleSessionId() }
}

pub fn lock_session(session: u32) -> Result<(), OsLockError> {
    let caller = planned_caller();
    run_os_lock(session, caller)
}

pub fn schedule_os_lock(session: u32) {
    let caller = planned_caller();
    *DEFERRED.lock().unwrap_or_else(|poison| poison.into_inner()) =
        Some(DeferredLock { session, caller });
}

pub fn chain_running() -> bool {
    CHAIN_RUNNING.load(Ordering::Acquire)
}

pub fn mark_host_unavailable(loss: HostLoss) {
    let code = match loss {
        HostLoss::Exited => LOSS_EXITED,
        HostLoss::Silent => LOSS_SILENT,
        HostLoss::GuardDown => LOSS_NONE,
    };
    if code != LOSS_NONE {
        LOSS.store(code, Ordering::Release);
        forget_host_writer();
    }
}

/// Runs a scheduled OS lock. The caller must not hold the authority mutex.
pub fn drain_os_lock(authority: &Arc<Mutex<Authority>>) {
    let job = DEFERRED
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take();
    let Some(job) = job else {
        return;
    };
    if CHAIN_RUNNING.swap(true, Ordering::AcqRel) {
        restore_deferred(job);
        return;
    }
    let _reset = ChainReset;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_os_lock(job.session, job.caller)
    }));
    if let Ok(Ok(())) = result {
        let mut guard = authority
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        guard.note_os_lock_held();
        guard.notify(Event::OsLockConfirmed);
    }
}

/// Clears the chain flag when the OS lock returns or panics. A panic that
/// left the flag set would make every later host poll wait forever.
struct ChainReset;

impl Drop for ChainReset {
    fn drop(&mut self) {
        CHAIN_RUNNING.store(false, Ordering::Release);
    }
}

/// A job that arrived while the chain was busy stays. Putting the older job
/// back would drop that newer request.
fn restore_deferred(job: DeferredLock) {
    let mut slot = DEFERRED.lock().unwrap_or_else(|poison| poison.into_inner());
    if slot.is_none() {
        *slot = Some(job);
    }
}

fn planned_caller() -> softlock_core::Caller {
    let alive = HOST_TX
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .is_some();
    let loss = match LOSS.load(Ordering::Acquire) {
        LOSS_EXITED => Some(softlock_core::FailReason::HostExited),
        LOSS_SILENT => Some(softlock_core::FailReason::HeartbeatMissed),
        _ => None,
    };
    softlock_core::caller_for_service(alive, loss)
}

fn run_os_lock(session: u32, caller: softlock_core::Caller) -> Result<(), OsLockError> {
    if stub_os_lock() {
        return Err(OsLockError::Failed);
    }
    let result = match caller {
        softlock_core::Caller::Service => softlock_windows::run_service_disconnect(session),
        _ => softlock_windows::run_service(session, request_host_primary),
    };
    let elapsed_us = u64::try_from(result.elapsed.as_micros()).unwrap_or(u64::MAX);
    let elapsed_us = elapsed_us.to_string();
    for error in &result.errors {
        crate::audit("os-lock", &[("error", error)]);
    }
    match result.outcome {
        softlock_core::Outcome::Confirmed { method } => {
            crate::audit(
                "os-lock",
                &[
                    ("result", "confirmed"),
                    ("method", method.name()),
                    ("caller", caller.name()),
                    ("elapsed_us", &elapsed_us),
                    ("policy", softlock_core::policy_seam()),
                ],
            );
            Ok(())
        }
        softlock_core::Outcome::FailedClosed { attempts } => {
            let detail = softlock_core::failed_closed_log(caller, &attempts);
            crate::audit(
                "os-lock",
                &[("result", "failed-closed"), ("detail", &detail)],
            );
            Err(OsLockError::Failed)
        }
    }
}

fn request_host_primary() -> bool {
    let sender = HOST_TX
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    let Some(sender) = sender else {
        crate::audit("os-lock", &[("host", "absent")]);
        return false;
    };
    let Ok(frame) = encode_service(0, &ServiceToHost::OsLock) else {
        crate::audit("os-lock", &[("host", "encode-failed")]);
        return false;
    };
    if sender.send(frame).is_err() {
        crate::audit("os-lock", &[("host", "send-failed")]);
        return false;
    }
    true
}

fn remember_host_writer(sender: Sender<Vec<u8>>) {
    *HOST_TX.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(sender);
}

fn forget_host_writer() {
    *HOST_TX.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
}

static HOST_TX: Mutex<Option<Sender<Vec<u8>>>> = Mutex::new(None);
static DEFERRED: Mutex<Option<DeferredLock>> = Mutex::new(None);
static CHAIN_RUNNING: AtomicBool = AtomicBool::new(false);
static LOSS: AtomicU8 = AtomicU8::new(LOSS_NONE);

const LOSS_NONE: u8 = 0;
const LOSS_EXITED: u8 = 1;
const LOSS_SILENT: u8 = 2;

struct DeferredLock {
    session: u32,
    caller: softlock_core::Caller,
}

pub fn run_foreground() -> i32 {
    serve(None)
}

pub fn run_session_probe(path: &str) -> i32 {
    match probe_client(&pipe_path()) {
        Ok(kind) => {
            let _ = write_probe(
                Path::new(path),
                &format!("kind={kind}\nsession={}\n", current_session()),
            );
            0
        }
        Err(error) => {
            let _ = write_probe(
                Path::new(path),
                &format!("error={error}\nsession={}\n", current_session()),
            );
            1
        }
    }
}

pub fn run_service() -> i32 {
    let mut name = wide(softlock_core::SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR(name.as_mut_ptr()),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW::default(),
    ];
    match unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    unsafe {
        if let Ok(handle) = RegisterServiceCtrlHandlerExW(
            PCWSTR(wide(softlock_core::SERVICE_NAME).as_ptr()),
            Some(control_handler),
            None,
        ) {
            STATUS.store(handle.0 as isize, Ordering::Release);
            set_status(SERVICE_RUNNING, ACCEPT_CONTROLS, 0);
        }
        let probe = probe_arg();
        let _ = std::panic::catch_unwind(|| {
            let _ = serve(probe);
        });
        set_status(SERVICE_STOPPED, 0, 0);
    }
}

unsafe extern "system" fn control_handler(
    control: u32,
    event_type: u32,
    data: *mut core::ffi::c_void,
    _context: *mut core::ffi::c_void,
) -> u32 {
    if control == SERVICE_CONTROL_STOP {
        // The supervisor confirms an OS lock before the host is killed.
        // Setting STOP here would drop the guard while the session is open.
        set_status(SERVICE_STOP_PENDING, 0, 2_000);
        STOP_REQUESTED.store(true, Ordering::Release);
    }
    if let Some(event) = session_event(control, event_type, data, console_session())
        && let Some(sender) = SESSION_TX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
    {
        let _ = sender.send(event);
    }
    0
}

/// A lock or unlock for any session other than the console session is ignored.
/// `data` is the `WTSSESSION_NOTIFICATION` the service control manager passes
/// for `SERVICE_CONTROL_SESSIONCHANGE`.
fn session_event(
    control: u32,
    event_type: u32,
    data: *mut core::ffi::c_void,
    console: u32,
) -> Option<SessionEvent> {
    if control != SERVICE_CONTROL_SESSIONCHANGE || data.is_null() {
        return None;
    }
    let note = unsafe { &*data.cast::<WTSSESSION_NOTIFICATION>() };
    if note.cbSize < std::mem::size_of::<WTSSESSION_NOTIFICATION>() as u32 {
        return None;
    }
    if note.dwSessionId != console {
        return None;
    }
    match event_type {
        WTS_SESSION_LOCK => Some(SessionEvent::OsLocked),
        WTS_SESSION_UNLOCK => Some(SessionEvent::OsUnlocked),
        _ => None,
    }
}

fn install_journal(authority: &mut Authority) {
    match crate::settings::SettingsEngine::open() {
        Ok(mut engine) => match engine.boot() {
            Ok(crate::settings::Boot::FailClosed(journal)) => {
                authority.platform_mut().install_engine(engine);
                authority.notify(Event::ServiceStarted {
                    journal,
                    session_active: true,
                });
            }
            Ok(_) => authority.platform_mut().install_engine(engine),
            Err(reason) => {
                crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
                authority.platform_mut().require_settings();
                let mut journal = softlock_core::Journal::empty();
                journal.mark_locked();
                journal.flush();
                authority.notify(Event::ServiceStarted {
                    journal,
                    session_active: true,
                });
            }
        },
        Err(reason) => {
            crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
            authority.platform_mut().require_settings();
            let mut journal = softlock_core::Journal::empty();
            journal.mark_locked();
            journal.flush();
            authority.notify(Event::ServiceStarted {
                journal,
                session_active: true,
            });
        }
    }
    crate::settings::register_sign_in_recovery();
}

fn poll_journal(authority: &Arc<Mutex<Authority>>) {
    let mut guard = authority
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if guard.platform_mut().poll_settings().is_err() {
        guard.notify(Event::ReassertFailed);
    }
}

pub(crate) fn running_product_state() -> Option<softlock_core::ProductState> {
    let name = pipe_path();
    let mut file = open_client_pipe(&name).ok()?;
    let body = br#"{"v":1,"id":2,"kind":"GetStatus"}"#;
    let mut frame = (body.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(body);
    write_prefixed_frame(&mut file, &frame).ok()?;
    let response = read_prefixed_frame(&mut file).ok()?;
    if response.len() < 4 {
        return Some(softlock_core::ProductState::Locked);
    }
    let text = String::from_utf8_lossy(&response[4..]);
    Some(product_state_from_status(&text))
}

fn product_state_from_status(text: &str) -> softlock_core::ProductState {
    use softlock_core::ProductState;
    for (needle, state) in [
        ("\"state\":\"FailingClosed\"", ProductState::FailingClosed),
        ("\"state\":\"Unlocking\"", ProductState::Unlocking),
        ("\"state\":\"Recovering\"", ProductState::Recovering),
        ("\"state\":\"OsLocked\"", ProductState::OsLocked),
        ("\"state\":\"Unlocked\"", ProductState::Unlocked),
        ("\"state\":\"Arming\"", ProductState::Arming),
        ("\"state\":\"Locked\"", ProductState::Locked),
    ] {
        if text.contains(needle) {
            return state;
        }
    }
    ProductState::Locked
}

fn serve(probe: Option<PathBuf>) -> i32 {
    let data_dir = crate::service_data_directory();
    let store = crate::secrets::verify_service_directory(&data_dir);
    let log = log_path();
    let mut logged = false;
    if store.is_ok() && log.parent() == Some(data_dir.as_path()) {
        if let Ok(file_log) = FileLog::create(&log) {
            logged = tracing::subscriber::set_global_default(file_log).is_ok();
        }
    } else if log.parent() != Some(data_dir.as_path()) {
        if let Some(parent) = log.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(file_log) = FileLog::create(&log) {
            logged = tracing::subscriber::set_global_default(file_log).is_ok();
        }
    }
    if !logged {
        crate::secrets::report_event(
            "service log unverified; PIN unlock stays unavailable when the store does not verify",
        );
    }
    let mode = if stub_os_lock() {
        OsLock::Fails
    } else {
        OsLock::System
    };
    let mut authority = Authority::new(ServicePlatform::new(mode), dev_pin_allowed());
    // A directory that fails verification is fatal for the PIN store. Locking
    // still runs. No hash is installed, so a planted pin.phc cannot unlock.
    let loaded = if store.is_err() && pin_directory() == data_dir {
        "refused".to_string()
    } else {
        match PinStore::in_dir(&pin_directory()).load() {
            Ok(Some(hash)) => {
                authority.install_pin_hash(hash);
                "verified".to_string()
            }
            Ok(None) => "none".to_string(),
            Err(_) => "refused".to_string(),
        }
    };
    *PIN_LOADED
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = loaded;
    remember_pin_status();
    install_journal(&mut authority);
    let arm = ArmWait::new();
    authority.platform_mut().set_arm_wait(arm.clone());
    let authority = Arc::new(Mutex::new(authority));
    let (session_tx, session_rx) = std::sync::mpsc::channel();
    *SESSION_TX
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(session_tx);
    let writer = Arc::new(Mutex::new(None::<std::sync::mpsc::Sender<Vec<u8>>>));
    let host = Arc::new(Mutex::new(None::<HostChild>));
    if let Ok(child) = spawn_host(&authority, &arm, true) {
        *writer.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(child.writer.clone());
        *host.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(child);
    }
    let pipe_name = pipe_path();
    let server = {
        let authority = Arc::clone(&authority);
        let pipe_name = pipe_name.clone();
        thread::spawn(move || pipe_loop(&pipe_name, &authority))
    };
    if let Some(path) = probe {
        let pipe_name = pipe_name.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            let _ = write_self_probe(&pipe_name, &path);
        });
    }
    let supervisor = {
        let authority = Arc::clone(&authority);
        let writer = Arc::clone(&writer);
        let host = Arc::clone(&host);
        let arm = arm.clone();
        thread::spawn(move || supervise(authority, writer, host, arm))
    };
    while !STOP.load(Ordering::Acquire) {
        apply_session_events(&authority, &session_rx);
        note_stop_file();
        poll_journal(&authority);
        poll_stop(&authority);
        thread::sleep(Duration::from_millis(20));
    }
    if let Some(child) = host
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
    {
        let _ = child.stop();
    }
    // The accept loop blocks inside ConnectNamedPipe. Dropping the join
    // handle lets the process exit once the service reports STOPPED.
    drop(server);
    let _ = supervisor.join();
    *SESSION_TX
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = None;
    0
}

fn remember_pin_status() {
    #[cfg(feature = "simulated-physical")]
    {
        if std::env::var("SOFTLOCK_PIN_SELFTEST").ok().as_deref() == Some("1") {
            *PIN_STATUS
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) =
                crate::secrets::exercise_pin_store();
            return;
        }
    }
    *PIN_STATUS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = "skipped".to_string();
}

fn apply_session_events(authority: &Arc<Mutex<Authority>>, events: &Receiver<SessionEvent>) {
    while let Ok(event) = events.try_recv() {
        let mut guard = authority
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        guard.notify(Event::Session(event));
    }
}

/// Stop is accepted only from `Unlocked` or after the OS lock is confirmed.
/// `Locked`, `Arming`, and `FailingClosed` stay up when that confirmation fails.
fn note_stop_file() {
    #[cfg(feature = "live-fail-closed")]
    {
        let Ok(path) = std::env::var("SOFTLOCK_STOP_FILE") else {
            return;
        };
        if live_hooks_allowed() && Path::new(&path).is_file() {
            STOP_REQUESTED.store(true, Ordering::Release);
        }
    }
}

fn poll_stop(authority: &Arc<Mutex<Authority>>) {
    if !STOP_REQUESTED.swap(false, Ordering::AcqRel) {
        return;
    }
    let allow = {
        let mut guard = authority
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        guard.notify(Event::ServiceStopRequested);
        matches!(
            guard.state(),
            LockState::Unlocked | LockState::OsLocked { .. }
        )
    };
    drain_os_lock(authority);
    if allow {
        crate::audit("service-stop", &[("result", "accepted")]);
        STOP.store(true, Ordering::Release);
    } else {
        crate::audit("service-stop", &[("result", "refused")]);
        set_status(SERVICE_RUNNING, ACCEPT_CONTROLS, 0);
    }
}

fn supervise(
    authority: Arc<Mutex<Authority>>,
    writer: Arc<Mutex<Option<std::sync::mpsc::Sender<Vec<u8>>>>>,
    host: Arc<Mutex<Option<HostChild>>>,
    arm: ArmWait,
) {
    let mut arm_sent = false;
    let mut next_id = 10u64;
    while !STOP.load(Ordering::Acquire) {
        if arm.is_waiting() && !arm_sent {
            next_id = next_id.saturating_add(1);
            if let Ok(frame) = encode_service(next_id, &ServiceToHost::Arm)
                && let Some(tx) = writer
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .clone()
            {
                let _ = tx.send(frame);
                arm_sent = true;
            }
        }
        if !arm.is_waiting() {
            arm_sent = false;
        }
        let alive = {
            let mut held = host.lock().unwrap_or_else(|poison| poison.into_inner());
            // A suspended host still looks alive to WaitForSingleObject and can
            // keep a session call blocked. End it before the disconnect.
            if held.as_ref().is_some_and(HostChild::wedged)
                && let Some(child) = held.take()
            {
                let _ = child.stop();
            }
            held.as_ref().is_some_and(HostChild::alive)
        };
        let action = authority
            .try_lock()
            .map(|mut guard| guard.poll_host(Instant::now(), alive))
            .unwrap_or(HostAction::Hold);
        drain_os_lock(&authority);
        if let HostAction::Respawn { arm: arm_again } = action {
            if let Some(child) = host
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .take()
            {
                let _ = child.stop();
            }
            if let Ok(child) = spawn_host(&authority, &arm, false) {
                *writer.lock().unwrap_or_else(|poison| poison.into_inner()) =
                    Some(child.writer.clone());
                if arm_again {
                    next_id = next_id.saturating_add(1);
                    if let Ok(frame) = encode_service(next_id, &ServiceToHost::Arm) {
                        let _ = child.writer.send(frame);
                    }
                }
                *host.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(child);
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn thread_is_suspended(thread: HANDLE) -> bool {
    let mut count = 0u32;
    let status = unsafe {
        NtQueryInformationThread(
            thread.0,
            THREAD_SUSPEND_COUNT,
            &mut count as *mut u32 as *mut core::ffi::c_void,
            4,
            std::ptr::null_mut(),
        )
    };
    status == 0 && count > 0
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationThread(
        thread: *mut core::ffi::c_void,
        class: u32,
        info: *mut core::ffi::c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
}

/// `ThreadSuspendCount` in `THREADINFOCLASS`.
const THREAD_SUSPEND_COUNT: u32 = 35;

struct HostChild {
    process: isize,
    writer: std::sync::mpsc::Sender<Vec<u8>>,
}

impl HostChild {
    fn handle(&self) -> HANDLE {
        HANDLE(self.process as *mut std::ffi::c_void)
    }

    fn alive(&self) -> bool {
        (unsafe { WaitForSingleObject(self.handle(), 0) }) == WAIT_TIMEOUT
    }

    /// True when a thread in this process has been suspended. `NtSuspendProcess`
    /// leaves the process handle unsignaled, so the silence timer would wait
    /// out a host that can no longer run.
    fn wedged(&self) -> bool {
        let pid = unsafe { GetProcessId(self.handle()) };
        if pid == 0 {
            return false;
        }
        unsafe {
            let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else {
                return false;
            };
            let mut entry = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..THREADENTRY32::default()
            };
            let mut wedged = false;
            if Thread32First(snapshot, &mut entry).is_ok() {
                loop {
                    if entry.th32OwnerProcessID == pid
                        && let Ok(thread) =
                            OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, entry.th32ThreadID)
                    {
                        if thread_is_suspended(thread) {
                            wedged = true;
                        }
                        let _ = CloseHandle(thread);
                        if wedged {
                            break;
                        }
                    }
                    if Thread32Next(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
            wedged
        }
    }

    fn stop(self) -> std::io::Result<()> {
        forget_host_writer();
        unsafe {
            let _ = TerminateProcess(self.handle(), 1);
            let _ = CloseHandle(self.handle());
        }
        GENERATION.fetch_add(1, Ordering::Release);
        Ok(())
    }
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

fn spawn_host(
    authority: &Arc<Mutex<Authority>>,
    arm: &ArmWait,
    allow_panic: bool,
) -> std::io::Result<HostChild> {
    let nonce = authority
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .issue_launch_nonce()
        .ok_or_else(|| std::io::Error::other("nonce"))?;
    let (parent_read, child_write) = inheritable_pipe(false)?;
    let (parent_write, child_read) = inheritable_pipe(true)?;
    let generation = GENERATION.fetch_add(1, Ordering::Release) + 1;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let welcome = encode_service(
        1,
        &ServiceToHost::Welcome {
            nonce: NonceBytes::from_nonce(&nonce),
        },
    )
    .map_err(|_| std::io::Error::other("welcome"))?;
    let _ = tx.send(welcome);
    let process = launch_host(child_read, child_write, allow_panic)?;
    unsafe {
        let _ = CloseHandle(child_read);
        let _ = CloseHandle(child_write);
    }
    let write_raw = parent_write.0 as usize;
    let read_raw = parent_read.0 as usize;
    let authority_reader = Arc::clone(authority);
    let replies = tx.clone();
    let arm_reader = arm.clone();
    thread::spawn(move || {
        let mut file = unsafe { std::fs::File::from_raw_handle(read_raw as *mut std::ffi::c_void) };
        host_reader(
            generation,
            &mut file,
            &authority_reader,
            &replies,
            arm_reader,
        );
    });
    thread::spawn(move || {
        let mut file =
            unsafe { std::fs::File::from_raw_handle(write_raw as *mut std::ffi::c_void) };
        host_writer(generation, &mut file, rx);
    });
    remember_host_writer(tx.clone());
    Ok(HostChild {
        process: process.0 as isize,
        writer: tx,
    })
}

fn host_reader(
    generation: u64,
    file: &mut std::fs::File,
    authority: &Arc<Mutex<Authority>>,
    replies: &std::sync::mpsc::Sender<Vec<u8>>,
    arm: ArmWait,
) {
    let mut id = 100u64;
    while GENERATION.load(Ordering::Acquire) == generation && !STOP.load(Ordering::Acquire) {
        let frame = match read_prefixed_frame(file) {
            Ok(frame) => frame,
            Err(_) => break,
        };
        let message = match decode_host_frame(&frame) {
            Ok(message) => message,
            Err(_) => continue,
        };
        // Wake the Lock waiter only after the nonce is accepted. A forged
        // HostReady must not look like the host is armed. `arm` no longer
        // waits while holding the authority mutex, so taking it here cannot
        // deadlock the request thread.
        let announce = matches!(message, softlock_core::HostToService::HostReady { .. });
        let reply = {
            let mut guard = match authority.lock() {
                Ok(guard) => guard,
                Err(poison) => poison.into_inner(),
            };
            guard.accept_host(message)
        };
        drain_os_lock(authority);
        if announce && reply.is_ok() {
            arm.ready();
        }
        if let Ok(Some(body)) = reply {
            id = id.saturating_add(1);
            if let Ok(frame) = encode_service(id, &body) {
                let _ = replies.send(frame);
            }
        }
    }
}

fn host_writer(generation: u64, file: &mut std::fs::File, rx: std::sync::mpsc::Receiver<Vec<u8>>) {
    while GENERATION.load(Ordering::Acquire) == generation {
        let Ok(mut frame) = rx.recv_timeout(Duration::from_millis(200)) else {
            if STOP.load(Ordering::Acquire) {
                break;
            }
            continue;
        };
        if write_prefixed_frame(file, &frame).is_err() {
            frame.fill(0);
            break;
        }
        frame.fill(0);
    }
}

fn inheritable_pipe(parent_writes: bool) -> std::io::Result<(HANDLE, HANDLE)> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    unsafe {
        CreatePipe(&mut read, &mut write, Some(&sa), 0)
            .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
    }
    let (parent, child) = if parent_writes {
        (write, read)
    } else {
        (read, write)
    };
    let parent = duplicate_uninherited(parent)?;
    Ok((parent, child))
}

fn duplicate_uninherited(source: HANDLE) -> std::io::Result<HANDLE> {
    let mut target = HANDLE::default();
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source,
            GetCurrentProcess(),
            &mut target,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
        .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
        let _ = CloseHandle(source);
    }
    Ok(target)
}

fn launch_host(read: HANDLE, write: HANDLE, allow_panic: bool) -> std::io::Result<HANDLE> {
    let host = lock_host_exe()?;
    let command = format!(
        "\"{}\" --service-host --read {} --write {}",
        host.display(),
        read.0 as usize,
        write.0 as usize
    );
    let mut command_wide = wide(&command);
    let mut desktop = wide(r"winsta0\default");
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        lpDesktop: PWSTR(desktop.as_mut_ptr()),
        ..STARTUPINFOW::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    let mut environment = environment_block(allow_panic);
    let created = unsafe {
        if current_session() == 0 {
            launch_in_console(
                &mut command_wide,
                &startup,
                environment.as_mut_ptr(),
                &mut info,
            )
        } else {
            CreateProcessW(
                PCWSTR::null(),
                Some(PWSTR(command_wide.as_mut_ptr())),
                None,
                None,
                true,
                windows::Win32::System::Threading::CREATE_UNICODE_ENVIRONMENT,
                Some(environment.as_mut_ptr().cast()),
                PCWSTR::null(),
                &startup,
                &mut info,
            )
        }
    };
    environment.fill(0);
    created.map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
    unsafe {
        let _ = CloseHandle(info.hThread);
    }
    Ok(info.hProcess)
}

fn launch_in_console(
    command: &mut [u16],
    startup: &STARTUPINFOW,
    environment: *mut u16,
    info: &mut PROCESS_INFORMATION,
) -> windows::core::Result<()> {
    let _ = enable_launch_privileges();
    let console = console_session();
    let mut process_token = HANDLE::default();
    unsafe {
        // The service token, not the interactive user's. Session 0 is
        // LocalSystem. Duplicating it and moving only the session id keeps
        // the Lock Host as S-1-5-18 inside the console session.
        OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut process_token)?;
        let mut primary = HANDLE::default();
        DuplicateTokenEx(
            process_token,
            TOKEN_ALL_ACCESS,
            None,
            windows::Win32::Security::SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        )?;
        let _ = CloseHandle(process_token);
        SetTokenInformation(
            primary,
            TokenSessionId,
            (&console as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )?;
        let created = CreateProcessAsUserW(
            Some(primary),
            PCWSTR::null(),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            true,
            windows::Win32::System::Threading::CREATE_UNICODE_ENVIRONMENT,
            Some(environment.cast()),
            PCWSTR::null(),
            startup,
            info,
        );
        let _ = CloseHandle(primary);
        if created.is_ok() {
            record_host_identity(info.hProcess);
        }
        created
    }
}

fn record_host_identity(process: HANDLE) {
    if let Some(user) = token_user(process) {
        *HOST_USER
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = user;
    }
    let pid = unsafe { GetProcessId(process) };
    let mut session = 0u32;
    unsafe {
        let _ = windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(pid, &mut session);
    }
    HOST_SESSION.store(u64::from(session), Ordering::Release);
}

fn token_user(process: HANDLE) -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return None;
        }
        let words = (needed as usize).div_ceil(std::mem::size_of::<u64>());
        let mut buffer = vec![0u64; words.max(1)];
        if GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .is_err()
        {
            let _ = CloseHandle(token);
            return None;
        }
        let user = &*(buffer.as_ptr().cast::<TOKEN_USER>());
        let mut text = PWSTR::null();
        if ConvertSidToStringSidW(user.User.Sid, &mut text).is_err() || text.is_null() {
            let _ = CloseHandle(token);
            return None;
        }
        let value = text.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        let _ = CloseHandle(token);
        value
    }
}

fn enable_launch_privileges() -> std::io::Result<()> {
    for name in [
        SE_TCB_NAME,
        SE_ASSIGNPRIMARYTOKEN_NAME,
        SE_INCREASE_QUOTA_NAME,
    ] {
        let _ = enable_privilege(name);
    }
    Ok(())
}

fn enable_privilege(name: PCWSTR) -> std::io::Result<()> {
    use windows::Win32::Foundation::ERROR_NOT_ALL_ASSIGNED;
    use windows::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES,
    };
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
        let mut luid = windows::Win32::Foundation::LUID::default();
        let looked = LookupPrivilegeValueW(PCWSTR::null(), name, &mut luid);
        if let Err(error) = looked {
            let _ = CloseHandle(token);
            return Err(std::io::Error::from_raw_os_error(error.code().0));
        }
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let adjusted = AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None);
        let last = GetLastError();
        let _ = CloseHandle(token);
        adjusted.map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
        if last == ERROR_NOT_ALL_ASSIGNED {
            return Err(std::io::Error::other("privilege missing"));
        }
    }
    Ok(())
}

fn environment_block(allow_panic: bool) -> Vec<u16> {
    // Windows reads a custom environment only while the names stay in
    // case-insensitive order. A filtered copy of the parent block is not
    // sorted once a later SetEnvironmentVariable appended a test key, and
    // the child then loses SOFTLOCK_RUN_NONCE and SOFTLOCK_STUB_OS_LOCK.
    let mut pairs: Vec<(String, String)> = std::env::vars()
        .filter(|(key, _)| host_env_allowed(key, allow_panic))
        .collect();
    for key in ["SOFTLOCK_STUB_OS_LOCK", "SOFTLOCK_RUN_NONCE"] {
        if let Ok(value) = std::env::var(key)
            && !pairs
                .iter()
                .any(|(existing, _)| existing.eq_ignore_ascii_case(key))
        {
            pairs.push((key.to_string(), value));
        }
    }
    #[cfg(any(feature = "simulated-physical", feature = "live-fail-closed"))]
    if let Ok(value) = std::env::var("SOFTLOCK_OS_LOCK_MARK")
        && !pairs
            .iter()
            .any(|(existing, _)| existing.eq_ignore_ascii_case("SOFTLOCK_OS_LOCK_MARK"))
    {
        pairs.push(("SOFTLOCK_OS_LOCK_MARK".to_string(), value));
    }
    if allow_panic
        && let Ok(value) = std::env::var("SOFTLOCK_PANIC")
        && !pairs
            .iter()
            .any(|(existing, _)| existing.eq_ignore_ascii_case("SOFTLOCK_PANIC"))
    {
        pairs.push(("SOFTLOCK_PANIC".to_string(), value));
    }
    if let Some((_, root)) = pairs
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("SystemRoot"))
        && !pairs
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("PATH"))
    {
        // System32 only. The parent PATH is not copied.
        let root = root.clone();
        pairs.push(("PATH".to_string(), format!("{root}\\System32;{root}")));
    }
    pairs.sort_by(|left, right| {
        left.0
            .to_ascii_lowercase()
            .cmp(&right.0.to_ascii_lowercase())
    });
    pairs.dedup_by(|left, right| left.0.eq_ignore_ascii_case(&right.0));
    let mut block = Vec::new();
    for (key, value) in pairs {
        for unit in key.encode_utf16() {
            block.push(unit);
        }
        block.push(u16::from(b'='));
        for unit in value.encode_utf16() {
            block.push(unit);
        }
        block.push(0);
    }
    block.push(0);
    block
}

fn host_env_allowed(key: &str, allow_panic: bool) -> bool {
    const SYSTEM: &[&str] = &[
        "SystemRoot",
        "SystemDrive",
        "windir",
        "ComSpec",
        "PATHEXT",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_IDENTIFIER",
        "PROCESSOR_LEVEL",
        "PROCESSOR_REVISION",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "ALLUSERSPROFILE",
        "ProgramData",
        "ProgramFiles",
        "CommonProgramFiles",
        "APPDATA",
        "LOCALAPPDATA",
    ];
    if SYSTEM.iter().any(|name| key.eq_ignore_ascii_case(name)) {
        return true;
    }
    #[cfg(feature = "simulated-physical")]
    if key.eq_ignore_ascii_case("SOFTLOCK_IMPOSTOR_PID")
        || key.eq_ignore_ascii_case("SOFTLOCK_IMPOSTOR_WITNESS")
        || key.eq_ignore_ascii_case("SOFTLOCK_IMPOSTOR_CASE")
    {
        return true;
    }
    match key {
        "SOFTLOCK_STUB_OS_LOCK" | "SOFTLOCK_RUN_NONCE" => true,
        "SOFTLOCK_PANIC" => allow_panic,
        _ => host_test_env(key),
    }
}

fn host_test_env(key: &str) -> bool {
    #[cfg(any(feature = "simulated-physical", feature = "live-fail-closed"))]
    if key.eq_ignore_ascii_case("SOFTLOCK_OS_LOCK_MARK") {
        return true;
    }
    #[cfg(feature = "live-fail-closed")]
    if key.eq_ignore_ascii_case("SOFTLOCK_FORCE_GUARD_DOWN")
        || key.eq_ignore_ascii_case("GITHUB_ACTIONS")
        || key.eq_ignore_ascii_case("RUNNER_ENVIRONMENT")
    {
        return true;
    }
    let _ = key;
    false
}

fn lock_host_exe() -> std::io::Result<PathBuf> {
    let current = std::env::current_exe()?;
    let dir = current.parent().unwrap_or(Path::new("."));
    let candidate = dir.join("softlock-lockhost.exe");
    if candidate.is_file() {
        Ok(candidate)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "lock host executable is missing",
        ))
    }
}

fn pipe_loop(name: &str, authority: &Arc<Mutex<Authority>>) {
    // One instance at a time would let an idle client hold the only pipe and
    // block Lock. Each connection gets its own instance. The first create
    // claims the name; later ones join it.
    let mut first = true;
    while !STOP.load(Ordering::Acquire) {
        let pipe = match create_public_pipe(name, first) {
            Ok(pipe) => pipe,
            Err(_) if first => {
                first = false;
                continue;
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(20));
                continue;
            }
        };
        first = false;
        let connected = unsafe { ConnectNamedPipe(pipe, None) };
        let already = connected.is_err() && unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if connected.is_err() && !already {
            unsafe {
                let _ = CloseHandle(pipe);
            }
            if unsafe { GetLastError() } == ERROR_BROKEN_PIPE {
                continue;
            }
            thread::sleep(Duration::from_millis(20));
            continue;
        }
        let authority = Arc::clone(authority);
        let raw = pipe.0 as usize;
        thread::spawn(move || {
            let pipe = HANDLE(raw as *mut core::ffi::c_void);
            serve_client(pipe, &authority);
            unsafe {
                let _ = DisconnectNamedPipe(pipe);
                let _ = CloseHandle(pipe);
            }
        });
    }
}

fn serve_client(pipe: HANDLE, authority: &Arc<Mutex<Authority>>) {
    let (session, image) = match client_identity(pipe) {
        Ok(identity) => identity,
        Err(_) => {
            reject_client(pipe);
            return;
        }
    };
    if !admit_client(session, console_session()) {
        crate::audit("rejected", &[("reason", "session")]);
        reject_client(pipe);
        return;
    }
    let role = client_role(&image);
    loop {
        if STOP.load(Ordering::Acquire) {
            break;
        }
        let mut prefix = [0u8; 4];
        if !read_exact(pipe, &mut prefix) {
            break;
        }
        let Ok(length) = softlock_core::checked_body_len(&prefix) else {
            let _ = write_response(pipe, 0, &softlock_core::ResponseKind::Unauthorized);
            break;
        };
        let mut frame = vec![0u8; 4 + length];
        frame[..4].copy_from_slice(&prefix);
        if !read_exact(pipe, &mut frame[4..]) {
            break;
        }
        let decoded_subscribe = subscribe_request(&frame);
        let mut response = {
            let mut guard = authority
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            guard.handle_public_frame(role, &frame)
        };
        drain_os_lock(authority);
        frame.fill(0);
        if stream_subscription(pipe, authority, &decoded_subscribe, &response) {
            break;
        }
        // Lock has marked the arm slot. Wait only after the authority mutex
        // is released, and only once HostReady has been accepted.
        if matches!(response, softlock_core::ResponseKind::Accepted) {
            let arm = authority
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .arm_wait();
            if let Some(arm) = arm.filter(|arm| arm.is_waiting())
                && !arm.wait_ready()
            {
                response = softlock_core::ResponseKind::NeedsConfirmation {
                    fields: vec!["guard".to_string()],
                };
            }
        }
        if write_response(pipe, 0, &response).is_err() {
            break;
        }
    }
}

/// Read the request the client has already written, then send the refusal.
/// Disconnecting before that read drops the reply, and the client only sees
/// a broken pipe.
fn reject_client(pipe: HANDLE) {
    let mut prefix = [0u8; 4];
    if read_exact(pipe, &mut prefix)
        && let Ok(length) = softlock_core::checked_body_len(&prefix)
    {
        let mut body = vec![0u8; length];
        let _ = read_exact(pipe, &mut body);
    }
    let _ = write_response(pipe, 0, &softlock_core::ResponseKind::Unauthorized);
    let mut extra = [0u8; 1];
    let _ = read_exact(pipe, &mut extra);
}

fn client_identity(pipe: HANDLE) -> std::io::Result<(u32, String)> {
    let mut pid = 0u32;
    unsafe {
        GetNamedPipeClientProcessId(pipe, &mut pid)
            .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
    }
    let mut session = 0u32;
    unsafe {
        windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(pid, &mut session)
            .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
    }
    let process = unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?
    };
    let mut buffer = vec![0u16; 1024];
    let mut len = buffer.len() as u32;
    let image = unsafe {
        let queried = QueryFullProcessImageNameW(
            process,
            windows::Win32::System::Threading::PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        queried.map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
        String::from_utf16_lossy(&buffer[..len as usize])
    };
    Ok((session, image))
}

fn create_public_pipe(name: &str, first: bool) -> std::io::Result<HANDLE> {
    // IU gets read and write, without FILE_CREATE_PIPE_INSTANCE (0x4).
    // 0x12019b is FILE_GENERIC_READ|FILE_GENERIC_WRITE with that bit cleared.
    // SYSTEM keeps full control. OW is the owner, so the service that created
    // the first instance can add more. Another user in IU cannot.
    let descriptor = security_descriptor("D:P(A;;GA;;;SY)(A;;0x4;;;OW)(A;;0x12019b;;;IU)")?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0.0,
        bInheritHandle: false.into(),
    };
    let wide_name = wide(name);
    let access = if first {
        FILE_FLAG_FIRST_PIPE_INSTANCE | PIPE_ACCESS_DUPLEX
    } else {
        PIPE_ACCESS_DUPLEX
    };
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(wide_name.as_ptr()),
            access,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            Some(&sa),
        )
    };
    if handle.is_invalid() {
        return Err(std::io::Error::last_os_error());
    }
    Ok(handle)
}

struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

fn security_descriptor(sddl: &str) -> std::io::Result<SecurityDescriptor> {
    let wide = wide(sddl);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide.as_ptr()),
            1,
            &mut descriptor,
            None,
        )
        .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
    }
    Ok(SecurityDescriptor(descriptor))
}

fn subscribe_request(frame: &[u8]) -> Option<u64> {
    match softlock_core::decode_request_frame(frame) {
        Ok(decoded) if matches!(decoded.kind, softlock_core::RequestKind::Subscribe) => {
            Some(decoded.id)
        }
        _ => None,
    }
}

/// Live Subscribe streams. A client that has already gone away must not keep a
/// slot until the next state change.
const SUBSCRIBE_CAP: usize = 8;
static SUBSCRIBERS: AtomicUsize = AtomicUsize::new(0);

/// After `Subscribe` is accepted, the connection stays open and the service
/// writes `StateChanged` whenever the public state name changes. A write
/// error or a disconnected client ends the stream. The function returns
/// whether it took the connection.
fn stream_subscription(
    pipe: HANDLE,
    authority: &Arc<Mutex<Authority>>,
    subscribe_id: &Option<u64>,
    response: &softlock_core::ResponseKind,
) -> bool {
    let Some(id) = subscribe_id else {
        return false;
    };
    if !matches!(response, softlock_core::ResponseKind::Accepted) {
        return false;
    }
    if write_response(pipe, *id, response).is_err() {
        return true;
    }
    let held = SUBSCRIBERS.fetch_add(1, Ordering::AcqRel);
    if held >= SUBSCRIBE_CAP {
        SUBSCRIBERS.fetch_sub(1, Ordering::AcqRel);
        // DisconnectNamedPipe drops unread bytes. Wait until the client has
        // the rejection frame so it can tell a full stream from a broken pipe.
        unsafe {
            let _ = windows::Win32::Storage::FileSystem::FlushFileBuffers(pipe);
        }
        return true;
    }
    let mut previous = None;
    let mut previous_fallback = {
        let guard = authority
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        guard.fallback_stream_start()
    };
    while !STOP.load(Ordering::Acquire) {
        if !subscriber_connected(pipe) {
            break;
        }
        let (name, fallback) = {
            let guard = authority
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            (guard.state_name(), guard.last_fallback())
        };
        if crate::fallback_is_new(previous_fallback, fallback) {
            previous_fallback = fallback;
            if let Some(fallback) = fallback
                && write_response(
                    pipe,
                    *id,
                    &softlock_core::ResponseKind::FellBackToOsLock { fallback },
                )
                .is_err()
            {
                break;
            }
        }
        if previous == Some(name) {
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        previous = Some(name);
        if write_response(
            pipe,
            *id,
            &softlock_core::ResponseKind::StateChanged { state: name },
        )
        .is_err()
        {
            break;
        }
    }
    SUBSCRIBERS.fetch_sub(1, Ordering::AcqRel);
    true
}

fn subscriber_connected(pipe: HANDLE) -> bool {
    let mut available = 0u32;
    unsafe { PeekNamedPipe(pipe, None, 0, None, Some(&mut available), None) }.is_ok()
}

fn write_response(
    pipe: HANDLE,
    id: u64,
    body: &softlock_core::ResponseKind,
) -> std::io::Result<()> {
    let json = message_json(id, body).map_err(|_| std::io::Error::other("encode"))?;
    let frame = encode_frame(&json).map_err(|_| std::io::Error::other("frame"))?;
    write_exact(pipe, &frame)
}

fn read_exact(handle: HANDLE, buf: &mut [u8]) -> bool {
    let mut offset = 0;
    while offset < buf.len() {
        let mut read = 0u32;
        let result = unsafe {
            windows::Win32::Storage::FileSystem::ReadFile(
                handle,
                Some(&mut buf[offset..]),
                Some(&mut read),
                None,
            )
        };
        if result.is_err() || read == 0 {
            return false;
        }
        offset += read as usize;
    }
    true
}

fn write_exact(handle: HANDLE, buf: &[u8]) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < buf.len() {
        let mut wrote = 0u32;
        unsafe {
            windows::Win32::Storage::FileSystem::WriteFile(
                handle,
                Some(&buf[offset..]),
                Some(&mut wrote),
                None,
            )
            .map_err(|error| std::io::Error::from_raw_os_error(error.code().0))?;
        }
        if wrote == 0 {
            return Err(std::io::Error::other("short write"));
        }
        offset += wrote as usize;
    }
    Ok(())
}

fn probe_client(name: &str) -> Result<String, String> {
    let mut file = open_client_pipe(name)?;
    let hello = br#"{"v":1,"id":1,"kind":"Hello","client_version":"0"}"#;
    let mut frame = (hello.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(hello);
    write_prefixed_frame(&mut file, &frame).map_err(|error| format!("{error:?}"))?;
    let response = read_prefixed_frame(&mut file).map_err(|error| format!("{error:?}"))?;
    let text = String::from_utf8_lossy(&response[4..]).to_string();
    if text.contains("Unauthorized") {
        Ok("Unauthorized".to_string())
    } else if text.contains("Hello") {
        Ok("Hello".to_string())
    } else {
        Ok("other".to_string())
    }
}

fn open_client_pipe(name: &str) -> Result<std::fs::File, String> {
    let wide_name = wide(name);
    let handle = unsafe {
        windows::Win32::Storage::FileSystem::CreateFileW(
            PCWSTR(wide_name.as_ptr()),
            windows::Win32::Storage::FileSystem::FILE_GENERIC_READ.0
                | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE.0,
            windows::Win32::Storage::FileSystem::FILE_SHARE_READ
                | windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE,
            None,
            windows::Win32::Storage::FileSystem::OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(|error| format!("open pipe: {error}"))?
    };
    Ok(unsafe { std::fs::File::from_raw_handle(handle.0) })
}

fn client_role(image: &str) -> softlock_core::Role {
    if dev_app_role() {
        return softlock_core::Role::App;
    }
    role_for_image(Path::new(image), &install_dir())
}

fn dev_app_role() -> bool {
    #[cfg(any(feature = "simulated-physical", feature = "live-fail-closed"))]
    {
        std::env::var("SOFTLOCK_DEV_APP").ok().as_deref() == Some("1") && dev_app_role_allowed()
    }
    #[cfg(not(any(feature = "simulated-physical", feature = "live-fail-closed")))]
    {
        false
    }
}

#[cfg(feature = "simulated-physical")]
fn dev_app_role_allowed() -> bool {
    true
}

#[cfg(all(feature = "live-fail-closed", not(feature = "simulated-physical")))]
fn dev_app_role_allowed() -> bool {
    live_hooks_allowed()
}

#[cfg(feature = "live-fail-closed")]
pub fn live_hooks_allowed() -> bool {
    softlock_windows::runner_may_lock().is_ok()
}

fn write_self_probe(pipe_name: &str, path: &Path) -> std::io::Result<()> {
    let started = Instant::now();
    let kind = loop {
        if let Ok(kind) = probe_client(pipe_name) {
            break kind;
        }
        if started.elapsed() > Duration::from_secs(10) {
            break "error:timeout".to_string();
        }
        thread::sleep(Duration::from_millis(100));
    };
    let system = if is_local_system() { "yes" } else { "no" };
    let host_user = HOST_USER
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    let host_session = HOST_SESSION.load(Ordering::Acquire);
    let host_session = if host_session == u64::MAX {
        "missing".to_string()
    } else {
        host_session.to_string()
    };
    let pin = PIN_STATUS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    let loaded = PIN_LOADED
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    write_probe(
        path,
        &format!(
            "session={}\nsystem={system}\nkind={kind}\nhost_user={host_user}\nhost_session={host_session}\nconsole={}\npin_loaded={loaded}\npin_reload={pin}\n",
            current_session(),
            console_session()
        ),
    )
}

fn write_probe(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    Ok(())
}

fn probe_arg() -> Option<PathBuf> {
    let mut args = std::env::args();
    while let Some(arg) = args.next() {
        if arg == "--probe" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

/// The pipe server's image is the service executable.
pub fn server_image_is_agent(pipe: HANDLE) -> bool {
    server_pid(pipe).is_some_and(image_is_service)
}

/// The pipe server is LocalSystem and its image is the service executable.
pub fn server_is_system_agent(pipe: HANDLE) -> bool {
    match server_pid(pipe) {
        Some(pid) => image_is_service(pid) && (pid_is_system(pid) || scm_pid_is_local_system(pid)),
        None => false,
    }
}

fn server_pid(pipe: HANDLE) -> Option<u32> {
    let mut pid = 0u32;
    unsafe { GetNamedPipeServerProcessId(pipe, &mut pid) }.ok()?;
    (pid != 0).then_some(pid)
}

fn image_is_service(pid: u32) -> bool {
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return false;
    };
    let mut buffer = vec![0u16; 1024];
    let mut len = buffer.len() as u32;
    let queried = unsafe {
        QueryFullProcessImageNameW(
            process,
            windows::Win32::System::Threading::PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    };
    unsafe {
        let _ = CloseHandle(process);
    }
    if queried.is_err() {
        return false;
    }
    let image = String::from_utf16_lossy(&buffer[..len as usize]);
    image
        .rsplit(['\\', '/'])
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("softlock-service.exe"))
}

fn pid_is_system(pid: u32) -> bool {
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut token = HANDLE::default();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token);
        let _ = CloseHandle(process);
        if opened.is_err() {
            return false;
        }
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return false;
        }
        let mut buffer = vec![0u8; needed as usize];
        let read = GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        );
        if read.is_err() {
            let _ = CloseHandle(token);
            return false;
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid_buf = vec![0u8; 64];
        let mut len = sid_buf.len() as u32;
        let created = CreateWellKnownSid(
            WinLocalSystemSid,
            None,
            Some(PSID(sid_buf.as_mut_ptr().cast())),
            &mut len,
        );
        let equal =
            created.is_ok() && EqualSid(user.User.Sid, PSID(sid_buf.as_mut_ptr().cast())).is_ok();
        let _ = CloseHandle(token);
        equal
    }
}

/// A client may not be allowed to open the service process token. The
/// Service Control Manager still reports the process id and the account.
fn scm_pid_is_local_system(pid: u32) -> bool {
    use windows::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceConfigW,
        QueryServiceStatusEx, SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_QUERY_CONFIG,
        SERVICE_QUERY_STATUS, SERVICE_STATUS_PROCESS,
    };
    unsafe {
        let Ok(manager) = OpenSCManagerW(None, None, SC_MANAGER_CONNECT) else {
            return false;
        };
        let name = wide(softlock_core::SERVICE_NAME);
        let Ok(service) = OpenServiceW(
            manager,
            PCWSTR(name.as_ptr()),
            SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG,
        ) else {
            let _ = CloseServiceHandle(manager);
            return false;
        };
        let mut status_buf = vec![0u8; std::mem::size_of::<SERVICE_STATUS_PROCESS>()];
        let mut needed = 0u32;
        let status_ok = QueryServiceStatusEx(
            service,
            SC_STATUS_PROCESS_INFO,
            Some(status_buf.as_mut_slice()),
            &mut needed,
        );
        let service_pid = if status_ok.is_ok() {
            (*status_buf.as_ptr().cast::<SERVICE_STATUS_PROCESS>()).dwProcessId
        } else {
            0
        };
        let mut needed = 0u32;
        let _ = QueryServiceConfigW(service, None, 0, &mut needed);
        let account_is_system = if needed == 0 {
            false
        } else {
            let mut config_buf = vec![0u8; needed as usize];
            let read = QueryServiceConfigW(
                service,
                Some(config_buf.as_mut_ptr().cast()),
                needed,
                &mut needed,
            );
            if read.is_err() {
                false
            } else {
                let config = &*config_buf
                    .as_ptr()
                    .cast::<windows::Win32::System::Services::QUERY_SERVICE_CONFIGW>();
                account_text(config.lpServiceStartName).eq_ignore_ascii_case("LocalSystem")
            }
        };
        let _ = CloseServiceHandle(service);
        let _ = CloseServiceHandle(manager);
        service_pid == pid && pid != 0 && account_is_system
    }
}

fn account_text(wide: PWSTR) -> String {
    if wide.is_null() {
        return String::new();
    }
    let mut units = Vec::new();
    let mut cursor = wide.0;
    for _ in 0..256 {
        let unit = unsafe { cursor.read() };
        if unit == 0 {
            break;
        }
        units.push(unit);
        cursor = unsafe { cursor.add(1) };
    }
    String::from_utf16_lossy(&units)
}

fn is_local_system() -> bool {
    unsafe {
        let mut sid_buf = vec![0u8; 64];
        let mut len = sid_buf.len() as u32;
        if CreateWellKnownSid(
            WinLocalSystemSid,
            None,
            Some(windows::Win32::Security::PSID(sid_buf.as_mut_ptr().cast())),
            &mut len,
        )
        .is_err()
        {
            return false;
        }
        if len as usize > sid_buf.len() {
            return false;
        }
        let mut member = windows::core::BOOL::default();
        CheckTokenMembership(
            None,
            windows::Win32::Security::PSID(sid_buf.as_mut_ptr().cast()),
            &mut member,
        )
        .is_ok()
            && member.as_bool()
    }
}

fn current_session() -> u32 {
    let mut session = 0u32;
    unsafe {
        let _ = windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(
            GetCurrentProcessId(),
            &mut session,
        );
    }
    session
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn install_dir() -> PathBuf {
    if let Some(dir) = install_dir_override() {
        return dir;
    }
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn install_dir_override() -> Option<PathBuf> {
    #[cfg(feature = "simulated-physical")]
    if let Ok(dir) = std::env::var("SOFTLOCK_INSTALL_DIR") {
        return Some(PathBuf::from(dir));
    }
    #[cfg(all(feature = "live-fail-closed", not(feature = "simulated-physical")))]
    if live_hooks_allowed()
        && let Ok(dir) = std::env::var("SOFTLOCK_INSTALL_DIR")
    {
        return Some(PathBuf::from(dir));
    }
    None
}

fn set_status(
    state: windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE,
    controls: u32,
    exit_code: u32,
) {
    let raw = STATUS.load(Ordering::Acquire);
    if raw == 0 {
        return;
    }
    let handle = SERVICE_STATUS_HANDLE(raw as *mut std::ffi::c_void);
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: controls,
        dwWin32ExitCode: exit_code,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 0,
    };
    unsafe {
        let _ = SetServiceStatus(handle, &status);
    }
}

#[cfg(test)]
mod session_filter {
    use super::session_event;
    use softlock_core::SessionEvent;
    use windows::Win32::System::RemoteDesktop::WTSSESSION_NOTIFICATION;
    use windows::Win32::System::Services::SERVICE_CONTROL_SESSIONCHANGE;
    use windows::Win32::UI::WindowsAndMessaging::{WTS_SESSION_LOCK, WTS_SESSION_UNLOCK};

    #[test]
    fn another_sessions_lock_does_not_move_the_console() {
        let console = 2u32;
        let mut other = WTSSESSION_NOTIFICATION {
            cbSize: std::mem::size_of::<WTSSESSION_NOTIFICATION>() as u32,
            dwSessionId: 5,
        };
        let other_ptr = (&mut other as *mut WTSSESSION_NOTIFICATION).cast::<core::ffi::c_void>();
        assert!(
            session_event(
                SERVICE_CONTROL_SESSIONCHANGE,
                WTS_SESSION_LOCK,
                other_ptr,
                console
            )
            .is_none()
        );
        assert!(
            session_event(
                SERVICE_CONTROL_SESSIONCHANGE,
                WTS_SESSION_UNLOCK,
                other_ptr,
                console
            )
            .is_none()
        );
        let mut same = WTSSESSION_NOTIFICATION {
            cbSize: std::mem::size_of::<WTSSESSION_NOTIFICATION>() as u32,
            dwSessionId: console,
        };
        let same_ptr = (&mut same as *mut WTSSESSION_NOTIFICATION).cast::<core::ffi::c_void>();
        assert_eq!(
            session_event(
                SERVICE_CONTROL_SESSIONCHANGE,
                WTS_SESSION_LOCK,
                same_ptr,
                console
            ),
            Some(SessionEvent::OsLocked)
        );
        assert_eq!(
            session_event(
                SERVICE_CONTROL_SESSIONCHANGE,
                WTS_SESSION_UNLOCK,
                same_ptr,
                console
            ),
            Some(SessionEvent::OsUnlocked)
        );
        assert!(session_event(SERVICE_CONTROL_SESSIONCHANGE, 1, same_ptr, console).is_none());
        assert!(
            session_event(
                SERVICE_CONTROL_SESSIONCHANGE,
                WTS_SESSION_LOCK,
                std::ptr::null_mut(),
                console
            )
            .is_none()
        );
    }
}

#[cfg(test)]
mod chain_tests {
    use std::sync::atomic::Ordering;
    use std::sync::{Mutex, MutexGuard};

    use super::{
        CHAIN_RUNNING, ChainReset, DEFERRED, DeferredLock, chain_running, restore_deferred,
    };

    fn hold() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    #[test]
    fn a_panic_in_the_os_lock_chain_clears_the_running_flag() {
        let _hold = hold();
        CHAIN_RUNNING.store(true, Ordering::Release);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _reset = ChainReset;
            panic!("os lock");
        }));
        assert!(panicked.is_err());
        assert!(
            !chain_running(),
            "a panic left the chain flag set, so host polls would stall"
        );
    }

    #[test]
    fn a_busy_chain_does_not_replace_a_newer_job() {
        let _hold = hold();
        CHAIN_RUNNING.store(false, Ordering::Release);
        *DEFERRED.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(DeferredLock {
            session: 9,
            caller: softlock_core::Caller::Service,
        });
        restore_deferred(DeferredLock {
            session: 4,
            caller: softlock_core::Caller::LockHost,
        });
        let kept = DEFERRED
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        assert_eq!(kept.expect("job").session, 9);
    }

    #[test]
    fn an_empty_slot_keeps_the_deferred_job() {
        let _hold = hold();
        *DEFERRED.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
        restore_deferred(DeferredLock {
            session: 4,
            caller: softlock_core::Caller::Service,
        });
        let kept = DEFERRED
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        assert_eq!(kept.expect("job").session, 4);
    }
}
