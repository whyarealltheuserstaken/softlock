//! Windows calls for the OS-lock chain.
//!
//! `LockWorkStation`'s return is recorded and then ignored. Confirmation is a
//! console-session notification or the session state, inside one second.
//! `DisableLockWorkstation` is not written. No call here reconnects a session.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use softlock_core::{
    Caller, Expect, Method, NoticeKind, NoticeLog, Observation, OsLockOps, Outcome, StepFault,
    console_notice, fail_closed,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
};
use windows::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTS_SESSIONSTATE_LOCK, WTS_SESSIONSTATE_UNLOCK, WTSConnectState,
    WTSDisconnectSession, WTSDisconnected, WTSFreeMemory, WTSGetActiveConsoleSessionId, WTSINFOEXW,
    WTSQuerySessionInformationW, WTSRegisterSessionNotification, WTSSessionInfoEx,
    WTSUnRegisterSessionNotification,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageTime, HWND_MESSAGE,
    PM_NOREMOVE, PeekMessageW, RegisterClassExW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_DESTROY, WNDCLASSEXW,
};
use windows::core::{PCWSTR, PWSTR, w};

const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_CONSOLE_DISCONNECT: u32 = 0x2;
const WTS_SESSION_LOCK: u32 = 0x7;
const NOTIFY_FOR_ALL_SESSIONS: u32 = 1;
const NO_SESSION: u32 = u32::MAX;
const NOTICE_PRESENT: u64 = 1 << 32;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

static GATE: Mutex<()> = Mutex::new(());
static CONSOLE: AtomicU32 = AtomicU32::new(NO_SESSION);
static LOCK_TICK: AtomicU64 = AtomicU64::new(0);
static DISCONNECT_TICK: AtomicU64 = AtomicU64::new(0);
static PUMP_THREAD: AtomicU32 = AtomicU32::new(0);

pub struct CloseResult {
    pub outcome: Outcome,
    pub errors: Vec<String>,
    /// Ticket 17 sets this by clearing `DisableLockWorkstation`. This crate
    /// never does.
    pub policy_cleared: bool,
    pub elapsed: Duration,
}

pub fn run_lock_host() -> CloseResult {
    run(Caller::LockHost, NO_SESSION, None, false)
}

/// The service is already gone. This process cannot update the journal, so
/// `LockWorkStation` would wait out the whole budget while
/// `DisableLockWorkstation` is still set. Disconnect instead.
pub fn run_lock_host_service_dead() -> CloseResult {
    run(Caller::LockHost, NO_SESSION, None, true)
}

pub fn run_lock_host_primary() -> CloseResult {
    run(Caller::LockHostPrimary, NO_SESSION, None, false)
}

pub fn run_service(console: u32, ask_host: impl FnMut() -> bool + 'static) -> CloseResult {
    run(
        Caller::ServiceCoordinated,
        console,
        Some(Box::new(ask_host)),
        false,
    )
}

pub fn run_service_disconnect(console: u32) -> CloseResult {
    run(Caller::Service, console, None, false)
}

pub fn runner_may_lock() -> Result<(), &'static str> {
    crate::proof_allowed(hosted_from_env(), school_from_registry())
}

fn run(
    caller: Caller,
    console: u32,
    ask_host: Option<Box<dyn FnMut() -> bool>>,
    skip_workstation: bool,
) -> CloseResult {
    let _gate = GATE.lock().unwrap_or_else(|poison| poison.into_inner());
    let console = if console == NO_SESSION {
        unsafe { WTSGetActiveConsoleSessionId() }
    } else {
        console
    };
    CONSOLE.store(console, Ordering::Release);
    let mut ops = WindowsOps {
        caller,
        console,
        own: own_session().unwrap_or(console),
        listener: Listener::start(),
        ask_host,
        skip_workstation,
        errors: Vec::new(),
        elapsed: Duration::ZERO,
        noted_lock_query: false,
        noted_disconnect_query: false,
    };
    if let Err(error) = &ops.listener {
        ops.note(error.clone());
    }
    let outcome = fail_closed(caller, &mut ops);
    let elapsed = ops.elapsed;
    CloseResult {
        outcome,
        errors: ops.errors,
        policy_cleared: false,
        elapsed,
    }
}

struct WindowsOps {
    caller: Caller,
    console: u32,
    own: u32,
    listener: Result<Listener, String>,
    ask_host: Option<Box<dyn FnMut() -> bool>>,
    skip_workstation: bool,
    errors: Vec<String>,
    elapsed: Duration,
    noted_lock_query: bool,
    noted_disconnect_query: bool,
}

impl WindowsOps {
    fn note(&mut self, error: String) {
        if !self.errors.iter().any(|existing| existing == &error) {
            self.errors.push(error);
        }
    }

    fn target(&self, method: Method) -> u32 {
        match method {
            Method::DisconnectConsole => self.console,
            Method::LockWorkStation | Method::DisconnectOwnSession => self.own,
        }
    }
}

impl OsLockOps for WindowsOps {
    fn before_lock_workstation(&mut self) -> Result<(), StepFault> {
        if self.skip_workstation {
            return Err(StepFault::Skipped);
        }
        // The service journals the clear. This crate never writes the value.
        Ok(())
    }

    fn call(&mut self, method: Method) -> Result<(), StepFault> {
        if let Ok(listener) = self.listener.as_mut() {
            listener.reset();
        }
        match method {
            Method::LockWorkStation => match self.caller {
                Caller::Service => Err(StepFault::Skipped),
                Caller::ServiceCoordinated => {
                    let asked = match self.ask_host.as_mut() {
                        Some(ask) => ask(),
                        None => false,
                    };
                    if asked {
                        Ok(())
                    } else {
                        Err(StepFault::Skipped)
                    }
                }
                Caller::LockHost | Caller::LockHostPrimary => match unsafe { LockWorkStation() } {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        self.note(format!("lock-workstation-win32={}", error.code().0));
                        Ok(())
                    }
                },
            },
            Method::DisconnectOwnSession | Method::DisconnectConsole => {
                let session = self.target(method);
                if session == NO_SESSION {
                    self.note("no-console-session".to_string());
                    return Err(StepFault::Failed);
                }
                match unsafe { WTSDisconnectSession(None, session, false) } {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        self.note(format!("disconnect-win32={}", error.code().0));
                        Ok(())
                    }
                }
            }
        }
    }

    fn observe(&mut self, expect: Expect, started: Instant) -> Result<Observation, StepFault> {
        let session =
            softlock_core::session_for_expect(self.caller, expect, self.console, self.own);
        let budget = softlock_core::CONFIRM_BUDGET;
        let started_tick = unsafe { GetTickCount() };
        loop {
            if let Ok(listener) = self.listener.as_mut() {
                listener.pump();
            }
            let notice = self
                .listener
                .as_ref()
                .ok()
                .and_then(|listener| listener.fresh(expect, started_tick));
            let session_locked = self.locked_flag(session);
            let session_disconnected = self.disconnected_flag(session);
            let elapsed = started.elapsed();
            self.elapsed = elapsed;
            let observation = Observation {
                notice,
                session_locked,
                session_disconnected,
                elapsed,
            };
            if softlock_core::confirmed(expect, &observation) || elapsed >= budget {
                return Ok(observation);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl WindowsOps {
    fn locked_flag(&mut self, session: u32) -> Option<bool> {
        match session_is_locked(session) {
            Ok(value) => value,
            Err(error) => {
                if !self.noted_lock_query {
                    self.noted_lock_query = true;
                    self.note(error);
                }
                None
            }
        }
    }

    fn disconnected_flag(&mut self, session: u32) -> Option<bool> {
        match session_is_disconnected(session) {
            Ok(value) => Some(value),
            Err(error) => {
                if !self.noted_disconnect_query {
                    self.noted_disconnect_query = true;
                    self.note(error);
                }
                None
            }
        }
    }
}

struct Listener {
    hwnd: HWND,
}

impl Listener {
    fn start() -> Result<Self, String> {
        unsafe {
            let mut discard = windows::Win32::UI::WindowsAndMessaging::MSG::default();
            let _ = PeekMessageW(&mut discard, None, 0, 0, PM_NOREMOVE);
            let class_name = class_name();
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..WNDCLASSEXW::default()
            };
            if RegisterClassExW(&class) == 0 && !class_already_exists() {
                return Err(format!("register-window={}", last_error()));
            }
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(class_name.as_ptr()),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance()),
                None,
            )
            .map_err(|error| format!("create-window={}", error.code().0))?;
            if let Err(error) = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_ALL_SESSIONS) {
                let _ = DestroyWindow(hwnd);
                return Err(format!("register-session={}", error.code().0));
            }
            PUMP_THREAD.store(GetCurrentThreadId(), Ordering::Release);
            Ok(Self { hwnd })
        }
    }

    fn reset(&self) {}

    fn fresh(&self, expect: Expect, started: u32) -> Option<NoticeKind> {
        let mut log = NoticeLog::default();
        if let Some(tick) = load_notice(&LOCK_TICK) {
            log.record(NoticeKind::Lock, tick);
        }
        if let Some(tick) = load_notice(&DISCONNECT_TICK) {
            log.record(NoticeKind::ConsoleDisconnect, tick);
        }
        let kind = match expect {
            Expect::Lock => NoticeKind::Lock,
            Expect::DisconnectConsole | Expect::DisconnectOwn => NoticeKind::ConsoleDisconnect,
        };
        log.fresh(kind, started)
    }

    fn pump(&self) {
        if PUMP_THREAD.load(Ordering::Acquire) != unsafe { GetCurrentThreadId() } {
            return;
        }
        unsafe {
            let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
            while PeekMessageW(
                &mut message,
                None,
                0,
                0,
                windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
            )
            .as_bool()
            {
                let _ = TranslateMessage(&message);
                let _ = DispatchMessageW(&message);
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        unsafe {
            let _ = WTSUnRegisterSessionNotification(self.hwnd);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn store_notice(kind: NoticeKind) {
    let tick = unsafe { GetMessageTime() } as u32;
    let slot = match kind {
        NoticeKind::Lock => &LOCK_TICK,
        NoticeKind::ConsoleDisconnect => &DISCONNECT_TICK,
        NoticeKind::Other => return,
    };
    slot.store(u64::from(tick) | NOTICE_PRESENT, Ordering::Release);
}

fn load_notice(slot: &AtomicU64) -> Option<u32> {
    let value = slot.load(Ordering::Acquire);
    if value & NOTICE_PRESENT == 0 {
        None
    } else {
        Some(value as u32)
    }
}

fn class_name() -> Vec<u16> {
    format!("{}-os-lock", softlock_core::SERVICE_NAME)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

fn class_already_exists() -> bool {
    // 1410 is ERROR_CLASS_ALREADY_EXISTS.
    unsafe { windows::Win32::Foundation::GetLastError().0 == 1410 }
}

fn last_error() -> u32 {
    unsafe { windows::Win32::Foundation::GetLastError().0 }
}

fn instance() -> HINSTANCE {
    let module = unsafe { GetModuleHandleW(None).unwrap_or_default() };
    HINSTANCE(module.0)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_WTSSESSION_CHANGE {
            let kind = match wparam.0 as u32 {
                WTS_SESSION_LOCK => NoticeKind::Lock,
                WTS_CONSOLE_DISCONNECT => NoticeKind::ConsoleDisconnect,
                _ => NoticeKind::Other,
            };
            let session = lparam.0 as u32;
            if let Some(kind) = console_notice(kind, session, CONSOLE.load(Ordering::Acquire)) {
                store_notice(kind);
            }
        }
        if message == WM_DESTROY {
            let _ = WTSUnRegisterSessionNotification(hwnd);
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

fn own_session() -> Option<u32> {
    let mut session = 0u32;
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session).ok()? };
    if session == NO_SESSION {
        None
    } else {
        Some(session)
    }
}

fn session_is_locked(session: u32) -> Result<Option<bool>, String> {
    if session == NO_SESSION {
        return Err("session-lock-unreadable".to_string());
    }
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        if WTSQuerySessionInformationW(None, session, WTSSessionInfoEx, &mut buffer, &mut bytes)
            .is_err()
            || buffer.is_null()
        {
            return Err(format!("session-lock-query={}", last_error()));
        }
        let info = &*buffer.0.cast::<WTSINFOEXW>();
        let level = info.Level;
        let flags = info.Data.WTSInfoExLevel1.SessionFlags;
        WTSFreeMemory(buffer.0.cast());
        if level != 1 {
            return Ok(None);
        }
        let locked = WTS_SESSIONSTATE_LOCK as i32;
        let unlocked = WTS_SESSIONSTATE_UNLOCK as i32;
        if flags != locked && flags != unlocked {
            return Ok(None);
        }
        Ok(Some(flags == locked && flags != unlocked))
    }
}

fn session_is_disconnected(session: u32) -> Result<bool, String> {
    if session == NO_SESSION {
        return Err("session-connect-unreadable".to_string());
    }
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        if WTSQuerySessionInformationW(None, session, WTSConnectState, &mut buffer, &mut bytes)
            .is_err()
            || buffer.is_null()
        {
            return Err(format!("session-connect-query={}", last_error()));
        }
        let state = *buffer.0.cast::<u32>();
        WTSFreeMemory(buffer.0.cast());
        Ok(state == WTSDisconnected.0 as u32)
    }
}

pub fn confirm_main() -> i32 {
    let hosted = hosted_from_env();
    let school = school_from_registry();
    if let Err(reason) = crate::proof_allowed(hosted, school) {
        eprintln!("os-lock confirm refused: {reason}");
        return if reason == "school-workstation" { 3 } else { 2 };
    }
    let result = run_lock_host();
    for error in &result.errors {
        eprintln!("os-lock error={error}");
    }
    match write_report(&result) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("os-lock report={error}");
            return 1;
        }
    }
    match result.outcome {
        Outcome::Confirmed { .. } if result.elapsed < softlock_core::CONFIRM_BUDGET => 0,
        Outcome::Confirmed { .. } | Outcome::FailedClosed { .. } => 1,
    }
}

fn hosted_from_env() -> bool {
    crate::hosted_runner(
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
        std::env::var("RUNNER_ENVIRONMENT").ok().as_deref(),
    )
}

fn school_from_registry() -> crate::SchoolCheck {
    let bios = w!("HARDWARE\\DESCRIPTION\\System\\BIOS");
    crate::classify_school(
        reg_sz(bios, w!("SystemFamily")).as_deref(),
        reg_sz(bios, w!("SystemVersion")).as_deref(),
        reg_sz(bios, w!("SystemProductName")).as_deref(),
    )
}

fn reg_sz(subkey: PCWSTR, value: PCWSTR) -> Option<String> {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, subkey, Some(0), KEY_READ, &mut key).is_err() {
            return None;
        }
        let mut data = [0u16; 128];
        let mut bytes = (data.len() * 2) as u32;
        let status = RegQueryValueExW(
            key,
            value,
            None,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut bytes),
        );
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS {
            return None;
        }
        let end = data
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(data.len());
        let text = String::from_utf16_lossy(&data[..end]);
        if text.is_empty() { None } else { Some(text) }
    }
}

fn write_report(result: &CloseResult) -> Result<(), String> {
    let path = PathBuf::from("suite-output")
        .join("os-lock-confirm")
        .join("result.json");
    refuse_reparse(&path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("report-dir={error}"))?;
        refuse_reparse(parent)?;
    }
    if path.symlink_metadata().is_ok() {
        refuse_reparse(&path)?;
        fs::remove_file(&path).map_err(|error| format!("report-remove={error}"))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("report-create={error}"))?;
    file.write_all(report_json(result).as_bytes())
        .map_err(|error| format!("report-write={error}"))?;
    Ok(())
}

fn refuse_reparse(path: &Path) -> Result<(), String> {
    let mut cursor = PathBuf::new();
    if path.is_absolute() {
        cursor.push(path.components().next().unwrap());
    }
    for component in path.components() {
        match component {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                cursor.push(component);
                continue;
            }
            std::path::Component::CurDir => continue,
            std::path::Component::ParentDir => {
                cursor.pop();
            }
            std::path::Component::Normal(_) => cursor.push(component),
        }
        if is_reparse(&cursor) {
            return Err("report-reparse".to_string());
        }
    }
    Ok(())
}

fn is_reparse(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if meta.file_type().is_symlink() {
        return true;
    }
    let wide = path_wide(path);
    let attributes =
        unsafe { windows::Win32::Storage::FileSystem::GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    attributes != u32::MAX && attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn path_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn report_json(result: &CloseResult) -> String {
    let (confirmed, method, state) = match &result.outcome {
        Outcome::Confirmed { method } => ("true", method.name(), "os-locked"),
        Outcome::FailedClosed { .. } => ("false", "none", "failed-closed"),
    };
    let elapsed_us = u64::try_from(result.elapsed.as_micros()).unwrap_or(u64::MAX);
    let mut errors = String::from("[");
    for (index, error) in result.errors.iter().enumerate() {
        if index > 0 {
            errors.push(',');
        }
        errors.push('"');
        errors.push_str(&json_escape(error));
        errors.push('"');
    }
    errors.push(']');
    format!(
        "{{\"confirmed\":{confirmed},\"method\":\"{method}\",\"state\":\"{state}\",\"elapsed_us\":{elapsed_us},\"hosted\":true,\"school\":false,\"disable_lock_workstation_cleared\":{},\"reconnect\":false,\"errors\":{errors}}}\n",
        if result.policy_cleared {
            "true"
        } else {
            "false"
        }
    )
}

fn json_escape(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '"' => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            _ if ch.is_control() => String::new(),
            _ => ch.to_string(),
        })
        .collect()
}
