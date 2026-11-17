//! Real Windows lock when the guard can no longer be guaranteed.
//!
//! Finding 04: `LockWorkStation` does not lock while `DisableLockWorkstation`
//! is set, and it does lock once that value is clear. This dev launch never
//! sets that value and never clears it. Ticket 15 clears it before calling
//! `LockWorkStation`. If the session is not actually locked, the in-session
//! fall-back is `WTSDisconnectSession`, which does not depend on the policy.
//! If that fails too, the guard and the overlay stay up. The PIN is the only
//! release, and the OS lock is retried. A hook that has already died is not
//! blocking input; the overlay still shows the error and the retry still runs.
//! Ctrl+Alt+Del is the secure attention sequence and is not swallowed.

use std::sync::atomic::{AtomicPtr, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HLOCAL, HWND, INVALID_HANDLE_VALUE,
    LocalFree,
};
use windows::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES,
};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MapViewOfFile, OpenFileMappingW,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTS_SESSIONSTATE_LOCK, WTS_SESSIONSTATE_UNLOCK, WTSConnectState,
    WTSDisconnectSession, WTSDisconnected, WTSFreeMemory, WTSGetActiveConsoleSessionId, WTSINFOEXW,
    WTSQuerySessionInformationW, WTSSessionInfoEx,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};
use windows::core::{PCWSTR, PWSTR};
static ARMED_MAPPING: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

use super::session::Shared;
use crate::real_lock::{RealLock, decide};

pub(crate) fn lock_or_disconnect(session: u32) -> RealLock {
    note_os_lock_attempt();
    if let Some(shared) = super::session::installed_session() {
        shared
            .os_lock_attempts
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        match shared
            .os_lock_stub
            .load(std::sync::atomic::Ordering::Acquire)
        {
            0 => {}
            // Confirmed workstation lock, without calling Windows.
            2 => return decide(true, false),
            // The Windows calls are not made. Neither path confirmed.
            _ => return decide(false, false),
        }
    }
    let _ = unsafe { LockWorkStation() };
    let session = if session == u32::MAX {
        own_session().unwrap_or_else(console_session)
    } else {
        session
    };
    if wait_until_locked(session, Duration::from_secs(1)) {
        return decide(true, false);
    }
    let target = own_session().unwrap_or(session);
    let api_ok = disconnect_session(target);
    let disconnected = api_ok && wait_until_disconnected(target, Duration::from_secs(1));
    decide(false, disconnected)
}

/// `WTS_CURRENT_SESSION` is `0xFFFFFFFF`. Callers that only want to see the
/// API fail must pass some other id.
pub fn disconnect_session(session: u32) -> bool {
    unsafe { WTSDisconnectSession(None, session, false) }.is_ok()
}

pub fn current_session_id() -> Option<u32> {
    own_session()
}

pub(crate) fn console_session() -> u32 {
    unsafe { WTSGetActiveConsoleSessionId() }
}

fn note_os_lock_attempt() {
    #[cfg(feature = "simulated-physical")]
    if let Ok(path) = std::env::var("SOFTLOCK_OS_LOCK_MARK") {
        let _ = std::fs::write(path, b"called");
    }
}

fn own_session() -> Option<u32> {
    let mut session = 0u32;
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session).ok()? };
    Some(session)
}

fn wait_until_locked(session: u32, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if session_is_locked(session) {
            return true;
        }
        pump_one();
        thread::sleep(Duration::from_millis(20));
    }
    session_is_locked(session)
}

fn wait_until_disconnected(session: u32, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if session_is_disconnected(session) {
            return true;
        }
        pump_one();
        thread::sleep(Duration::from_millis(20));
    }
    session_is_disconnected(session)
}

fn session_is_disconnected(session: u32) -> bool {
    session_connect_state(session) == Some(WTSDisconnected.0 as u32)
}

fn session_connect_state(session: u32) -> Option<u32> {
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        if WTSQuerySessionInformationW(None, session, WTSConnectState, &mut buffer, &mut bytes)
            .is_err()
            || buffer.is_null()
        {
            return None;
        }
        let state = *buffer.0.cast::<u32>();
        WTSFreeMemory(buffer.0.cast());
        Some(state)
    }
}

fn session_is_locked(session: u32) -> bool {
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        if WTSQuerySessionInformationW(None, session, WTSSessionInfoEx, &mut buffer, &mut bytes)
            .is_err()
            || buffer.is_null()
        {
            return false;
        }
        let info = &*buffer.0.cast::<WTSINFOEXW>();
        let level = info.Level;
        let flags = info.Data.WTSInfoExLevel1.SessionFlags as u32;
        WTSFreeMemory(buffer.0.cast());
        level == 1 && flags == WTS_SESSIONSTATE_LOCK && flags != WTS_SESSIONSTATE_UNLOCK
    }
}

fn pump_one() {
    unsafe {
        let mut message = MSG::default();
        if PeekMessageW(&mut message, Some(HWND::default()), 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            let _ = DispatchMessageW(&message);
        }
    }
}

/// Publish this process as the armed host. A mapping that already exists is
/// used only when its DACL is SYSTEM-only, so a pre-created object is ignored.
pub(crate) fn publish_armed_host() {
    if !ARMED_MAPPING.load(Ordering::Acquire).is_null() {
        return;
    }
    let Some(mapping) = create_armed_mapping() else {
        return;
    };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS && !mapping_is_system_only(mapping) {
        unsafe {
            let _ = CloseHandle(mapping);
        }
        return;
    }
    if !write_armed_pid(mapping, unsafe { GetCurrentProcessId() }) {
        unsafe {
            let _ = CloseHandle(mapping);
        }
        return;
    }
    ARMED_MAPPING.store(mapping.0, Ordering::Release);
}

/// Another live lock host has published a pid in the SYSTEM-only slot.
pub(crate) fn replacement_host_armed() -> bool {
    let Some(mapping) = open_armed_mapping() else {
        return false;
    };
    let trusted = mapping_is_system_only(mapping);
    let pid = if trusted { read_armed_pid(mapping) } else { 0 };
    unsafe {
        let _ = CloseHandle(mapping);
    }
    trusted && pid != 0 && pid != unsafe { GetCurrentProcessId() } && host_process_alive(pid)
}

pub(crate) fn clear_armed_host() {
    let raw = ARMED_MAPPING.swap(std::ptr::null_mut(), Ordering::AcqRel);
    if raw.is_null() {
        return;
    }
    unsafe {
        let _ = CloseHandle(HANDLE(raw));
    }
}

fn create_armed_mapping() -> Option<HANDLE> {
    let descriptor = armed_descriptor()?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let name = armed_host_name();
    let mapping = unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            Some(&sa),
            PAGE_READWRITE,
            0,
            4,
            PCWSTR(name.as_ptr()),
        )
    };
    free_security(&descriptor);
    mapping.ok()
}

fn open_armed_mapping() -> Option<HANDLE> {
    let name = armed_host_name();
    unsafe {
        OpenFileMappingW(
            FILE_MAP_READ.0 | FILE_MAP_WRITE.0,
            false,
            PCWSTR(name.as_ptr()),
        )
    }
    .ok()
}

fn write_armed_pid(mapping: HANDLE, pid: u32) -> bool {
    let view = unsafe { MapViewOfFile(mapping, FILE_MAP_WRITE, 0, 0, 4) };
    if view.Value.is_null() {
        return false;
    }
    unsafe {
        view.Value.cast::<u32>().write(pid);
        let _ = UnmapViewOfFile(view);
    }
    true
}

fn read_armed_pid(mapping: HANDLE) -> u32 {
    let view = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 4) };
    if view.Value.is_null() {
        return 0;
    }
    let pid = unsafe { view.Value.cast::<u32>().read() };
    unsafe {
        let _ = UnmapViewOfFile(view);
    }
    pid
}

fn host_process_alive(pid: u32) -> bool {
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return false;
    };
    let mut buffer = vec![0u16; 1024];
    let mut len = buffer.len() as u32;
    let queried = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
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
        .is_some_and(|name| name.eq_ignore_ascii_case("softlock-lockhost.exe"))
}

fn mapping_is_system_only(mapping: HANDLE) -> bool {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetSecurityInfo(
            mapping,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            Some(&mut descriptor),
        )
    };
    if status.0 != 0 || descriptor.0.is_null() {
        return false;
    }
    let mut wide = PWSTR::null();
    let converted = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            1,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut wide,
            None,
        )
    };
    let text = if converted.is_ok() {
        wide_to_string(wide)
    } else {
        String::new()
    };
    if !wide.is_null() {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(wide.0.cast())));
        }
    }
    free_security(&descriptor);
    text.contains("O:SY") && text.contains("D:P(A;;FA;;;SY)")
}

fn armed_descriptor() -> Option<PSECURITY_DESCRIPTOR> {
    let sddl: Vec<u16> = "D:P(A;;FA;;;SY)"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1,
            &mut descriptor,
            None,
        )
        .ok()?;
    }
    Some(descriptor)
}

fn free_security(descriptor: &PSECURITY_DESCRIPTOR) {
    if !descriptor.0.is_null() {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
    }
}

fn armed_host_name() -> Vec<u16> {
    format!(r"Local\{}_ArmedHost", softlock_core::SERVICE_NAME)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

fn wide_to_string(wide: PWSTR) -> String {
    if wide.is_null() {
        return String::new();
    }
    let mut units = Vec::new();
    let mut cursor = wide.0;
    for _ in 0..1024 {
        let unit = unsafe { cursor.read() };
        if unit == 0 {
            break;
        }
        units.push(unit);
        cursor = unsafe { cursor.add(1) };
    }
    String::from_utf16_lossy(&units)
}

pub(crate) fn clear_execution_state(shared: &Shared) {
    if shared
        .execution
        .swap(false, std::sync::atomic::Ordering::AcqRel)
    {
        unsafe {
            windows::Win32::System::Power::SetThreadExecutionState(
                windows::Win32::System::Power::ES_CONTINUOUS,
            );
        }
    }
}
