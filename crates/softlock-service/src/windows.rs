//! Windows pipe, Service Control Manager, and Lock Host spawn.
//!
//! The public pipe is local. A client whose session is not the active console
//! session is rejected. The Lock Host is a child process with inherited pipe
//! handles. Its nonce goes in the first Welcome message, not on the command line.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
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
    CheckTokenMembership, CreateWellKnownSid, DuplicateTokenEx, GetTokenInformation,
    PSECURITY_DESCRIPTOR, SE_ASSIGNPRIMARYTOKEN_NAME, SE_INCREASE_QUOTA_NAME, SE_TCB_NAME,
    SECURITY_ATTRIBUTES, SetTokenInformation, TOKEN_ALL_ACCESS, TOKEN_QUERY, TOKEN_USER,
    TokenPrimary, TokenSessionId, TokenUser, WinLocalSystemSid,
};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, CreatePipe, DisconnectNamedPipe,
    GetNamedPipeClientProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::RemoteDesktop::{WTSDisconnectSession, WTSGetActiveConsoleSessionId};
use windows::Win32::System::Services::{
    RegisterServiceCtrlHandlerExW, SERVICE_ACCEPT_SESSIONCHANGE, SERVICE_ACCEPT_STOP,
    SERVICE_CONTROL_SESSIONCHANGE, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_STATUS,
    SERVICE_STATUS_HANDLE, SERVICE_STOP_PENDING, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW,
    SERVICE_WIN32_OWN_PROCESS, SetServiceStatus, StartServiceCtrlDispatcherW,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, GetCurrentProcess, GetCurrentProcessId, GetProcessId,
    OpenProcess, OpenProcessToken, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{WTS_SESSION_LOCK, WTS_SESSION_UNLOCK};
use windows::core::{PCWSTR, PWSTR};

use crate::authority::{Authority, HostAction};
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
static SESSION_TX: Mutex<Option<Sender<SessionEvent>>> = Mutex::new(None);

const ACCEPT_CONTROLS: u32 = SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SESSIONCHANGE;

pub fn console_session() -> u32 {
    unsafe { WTSGetActiveConsoleSessionId() }
}

pub fn lock_session(session: u32) -> Result<(), OsLockError> {
    if stub_os_lock() {
        return Err(OsLockError::Failed);
    }
    if current_session() != 0 && unsafe { LockWorkStation() }.is_ok() {
        return Ok(());
    }
    unsafe { WTSDisconnectSession(None, session, false) }.map_err(|_| OsLockError::Failed)
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
    _data: *mut core::ffi::c_void,
    _context: *mut core::ffi::c_void,
) -> u32 {
    if control == SERVICE_CONTROL_STOP {
        // The supervisor confirms an OS lock before the host is killed.
        // Setting STOP here would drop the guard while the session is open.
        set_status(SERVICE_STOP_PENDING, 0, 2_000);
        STOP_REQUESTED.store(true, Ordering::Release);
    }
    if let Some(event) = session_event(control, event_type)
        && let Some(sender) = SESSION_TX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
    {
        let _ = sender.send(event);
    }
    0
}

fn session_event(control: u32, event_type: u32) -> Option<SessionEvent> {
    if control != SERVICE_CONTROL_SESSIONCHANGE {
        return None;
    }
    match event_type {
        WTS_SESSION_LOCK => Some(SessionEvent::OsLocked),
        WTS_SESSION_UNLOCK => Some(SessionEvent::OsUnlocked),
        _ => None,
    }
}

fn serve(probe: Option<PathBuf>) -> i32 {
    let _ = FileLog::create(&log_path()).map(tracing::subscriber::set_global_default);
    let mode = if stub_os_lock() {
        OsLock::Fails
    } else {
        OsLock::System
    };
    let mut authority = Authority::new(ServicePlatform::new(mode), dev_pin_allowed());
    if let Ok(Some(hash)) = PinStore::in_dir(&pin_directory()).load() {
        authority.install_pin_hash(hash);
    }
    remember_pin_status();
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
        let alive = host
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
            .is_some_and(HostChild::alive);
        let action = authority
            .try_lock()
            .map(|mut guard| guard.poll_host(Instant::now(), alive))
            .unwrap_or(HostAction::Hold);
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

    fn stop(self) -> std::io::Result<()> {
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
    let mut block = Vec::new();
    for (key, value) in std::env::vars() {
        if !allow_panic && key == "SOFTLOCK_PANIC" {
            continue;
        }
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
        let mut response = {
            let mut guard = authority
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            guard.handle_public_frame(role, &frame)
        };
        frame.fill(0);
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
    // IU gets read and write (FILE_GENERIC_READ | FILE_GENERIC_WRITE), not
    // generic-all. SYSTEM keeps full control of the pipe.
    let descriptor = security_descriptor("D:P(A;;GA;;;SY)(A;;0x12019f;;;IU)")?;
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
    #[cfg(feature = "simulated-physical")]
    if std::env::var("SOFTLOCK_DEV_APP").ok().as_deref() == Some("1") {
        return softlock_core::Role::App;
    }
    role_for_image(Path::new(image), &install_dir())
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
    write_probe(
        path,
        &format!(
            "session={}\nsystem={system}\nkind={kind}\nhost_user={host_user}\nhost_session={host_session}\nconsole={}\npin_reload={pin}\n",
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
    #[cfg(feature = "simulated-physical")]
    if let Ok(dir) = std::env::var("SOFTLOCK_INSTALL_DIR") {
        return PathBuf::from(dir);
    }
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
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
