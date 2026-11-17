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

#[cfg(feature = "simulated-physical")]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicIsize, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HLOCAL, HWND, INVALID_HANDLE_VALUE,
    LocalFree, STILL_ACTIVE, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
};
use windows::Win32::Security::{
    CreateWellKnownSid, DACL_SECURITY_INFORMATION, EqualSid, GetTokenInformation,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenUser, WinLocalSystemSid,
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
    CreateEventW, EVENT_ALL_ACCESS, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId,
    GetExitCodeProcess, OpenEventW, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, SetEvent, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};
use windows::core::{PCWSTR, PWSTR};
static ARMED_MAPPING: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());
static ARMED_EVENT: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());
/// Process and event handles for the replacement this host has already opened.
/// The PID in the mapping can be reused. The handle cannot.
static HELD_PROCESS: AtomicIsize = AtomicIsize::new(0);
static HELD_EVENT: AtomicIsize = AtomicIsize::new(0);
static HELD_PID: AtomicU32 = AtomicU32::new(0);
static HELD_NONCE: AtomicU64 = AtomicU64::new(0);
const ARMED_SLOT_BYTES: usize = 16;
/// Nonce used only by the simulated-physical impostor seam. It never matches a
/// published arm event, so the seam cannot satisfy the armed-event check.
#[cfg(feature = "simulated-physical")]
const IMPOSTOR_NONCE: u64 = 0x51F7_10C5_A11E_0002;

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
/// The armed event is signaled only after the slot is visible.
pub(crate) fn publish_armed_host() {
    if !ARMED_MAPPING.load(Ordering::Acquire).is_null() {
        return;
    }
    let nonce = fresh_nonce();
    let Some(event) = create_armed_event(nonce) else {
        return;
    };
    let Some(mapping) = create_armed_mapping() else {
        close_handle(event);
        return;
    };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS && !mapping_is_system_only(mapping) {
        close_handle(mapping);
        close_handle(event);
        return;
    }
    if !write_armed_slot(mapping, unsafe { GetCurrentProcessId() }, nonce) {
        close_handle(mapping);
        close_handle(event);
        return;
    }
    if unsafe { SetEvent(event) }.is_err() {
        close_handle(mapping);
        close_handle(event);
        return;
    }
    ARMED_EVENT.store(event.0, Ordering::Release);
    ARMED_MAPPING.store(mapping.0, Ordering::Release);
}

/// Another live lock host has armed. PID and image name are not enough: the
/// PID can be reused by a renamed standard-user process after the replacement
/// dies. Release only while this host still holds that process's handle, the
/// token is SYSTEM, the full image path is this executable, and the per-run
/// SYSTEM-only event is signaled. Any doubt keeps the hooks.
pub(crate) fn replacement_host_armed() -> bool {
    if let Some((pid, nonce)) = read_trusted_slot() {
        if held_matches(pid, nonce) {
            if held_still_armed() {
                return true;
            }
            // The process behind the held handle is gone. Do not open the
            // same PID again in this call: it may already belong to someone else.
            drop_held();
            return consider_impostor();
        }
        drop_held();
        if trust_new_process(pid, nonce, true) {
            return true;
        }
    } else {
        drop_held();
    }
    consider_impostor()
}

pub(crate) fn clear_armed_host() {
    drop_held();
    close_stored(&ARMED_EVENT);
    close_stored(&ARMED_MAPPING);
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
            ARMED_SLOT_BYTES as u32,
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

fn write_armed_slot(mapping: HANDLE, pid: u32, nonce: u64) -> bool {
    let view = unsafe { MapViewOfFile(mapping, FILE_MAP_WRITE, 0, 0, ARMED_SLOT_BYTES) };
    if view.Value.is_null() {
        return false;
    }
    let mut bytes = [0u8; ARMED_SLOT_BYTES];
    bytes[..4].copy_from_slice(&pid.to_le_bytes());
    bytes[8..16].copy_from_slice(&nonce.to_le_bytes());
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), view.Value.cast(), ARMED_SLOT_BYTES);
        let _ = UnmapViewOfFile(view);
    }
    true
}

fn read_trusted_slot() -> Option<(u32, u64)> {
    let mapping = open_armed_mapping()?;
    let trusted = mapping_is_system_only(mapping);
    let slot = if trusted {
        read_armed_slot(mapping)
    } else {
        None
    };
    close_handle(mapping);
    slot
}

fn read_armed_slot(mapping: HANDLE) -> Option<(u32, u64)> {
    let view = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, ARMED_SLOT_BYTES) };
    if view.Value.is_null() {
        return None;
    }
    let mut bytes = [0u8; ARMED_SLOT_BYTES];
    unsafe {
        std::ptr::copy_nonoverlapping(view.Value.cast(), bytes.as_mut_ptr(), ARMED_SLOT_BYTES);
        let _ = UnmapViewOfFile(view);
    }
    let pid = u32::from_le_bytes(bytes[..4].try_into().ok()?);
    let nonce = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    if pid == 0 || nonce == 0 {
        None
    } else {
        Some((pid, nonce))
    }
}

fn create_armed_event(nonce: u64) -> Option<HANDLE> {
    let descriptor = armed_descriptor()?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let name = armed_event_name(nonce);
    let event = unsafe { CreateEventW(Some(&sa), true, false, PCWSTR(name.as_ptr())) };
    free_security(&descriptor);
    event.ok()
}

fn confirmed_event(nonce: u64) -> Option<HANDLE> {
    if nonce == 0 {
        return None;
    }
    let name = armed_event_name(nonce);
    let event = unsafe { OpenEventW(EVENT_ALL_ACCESS, false, PCWSTR(name.as_ptr())) }.ok()?;
    if !mapping_is_system_only(event) || !event_is_signaled(event) {
        close_handle(event);
        return None;
    }
    Some(event)
}

fn event_is_signaled(event: HANDLE) -> bool {
    let waited = unsafe { WaitForSingleObject(event, 0) };
    waited == WAIT_OBJECT_0
}

fn trust_new_process(pid: u32, nonce: u64, require_same_slot: bool) -> bool {
    if pid == 0 || nonce == 0 || pid == current_pid() {
        return false;
    }
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return false;
    };
    if require_same_slot && read_trusted_slot() != Some((pid, nonce)) {
        close_handle(process);
        return false;
    }
    let Some(event) = confirmed_event(nonce) else {
        close_handle(process);
        return false;
    };
    let trusted = process_still_active(process)
        && process_token_is_system(process)
        && image_path_is_current_exe(process)
        && process_still_active(process);
    if !trusted {
        close_handle(event);
        close_handle(process);
        return false;
    }
    store_held(process, event, pid, nonce);
    held_still_armed()
}

fn consider_impostor() -> bool {
    #[cfg(not(feature = "simulated-physical"))]
    {
        false
    }
    #[cfg(feature = "simulated-physical")]
    {
        let Some(pid) = impostor_pid() else {
            return false;
        };
        // A missing process is not a decision yet. The seam writes the pid
        // only after that process is alive.
        let Ok(probe) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return false;
        };
        close_handle(probe);
        let trusted = trust_new_process(pid, IMPOSTOR_NONCE, false);
        note_impostor(trusted);
        trusted
    }
}

#[cfg(feature = "simulated-physical")]
fn impostor_pid() -> Option<u32> {
    let path = std::env::var("SOFTLOCK_IMPOSTOR_PID").ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let pid = text.trim().parse::<u32>().ok()?;
    if pid == 0 { None } else { Some(pid) }
}

#[cfg(feature = "simulated-physical")]
fn note_impostor(trusted: bool) {
    static NOTED: AtomicBool = AtomicBool::new(false);
    if NOTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let Ok(path) = std::env::var("SOFTLOCK_IMPOSTOR_WITNESS") else {
        return;
    };
    let text = if trusted { "released" } else { "rejected" };
    let _ = std::fs::write(path, text);
}

fn held_matches(pid: u32, nonce: u64) -> bool {
    HELD_PROCESS.load(Ordering::Acquire) != 0
        && HELD_PID.load(Ordering::Acquire) == pid
        && HELD_NONCE.load(Ordering::Acquire) == nonce
}

fn held_still_armed() -> bool {
    let process = load_handle(&HELD_PROCESS);
    let event = load_handle(&HELD_EVENT);
    let (Some(process), Some(event)) = (process, event) else {
        return false;
    };
    process_still_active(process)
        && event_is_signaled(event)
        && process_token_is_system(process)
        && image_path_is_current_exe(process)
}

fn store_held(process: HANDLE, event: HANDLE, pid: u32, nonce: u64) {
    store_handle(&HELD_PROCESS, process);
    store_handle(&HELD_EVENT, event);
    HELD_PID.store(pid, Ordering::Release);
    HELD_NONCE.store(nonce, Ordering::Release);
}

fn drop_held() {
    HELD_PID.store(0, Ordering::Release);
    HELD_NONCE.store(0, Ordering::Release);
    close_stored_isize(&HELD_EVENT);
    close_stored_isize(&HELD_PROCESS);
}

fn process_still_active(process: HANDLE) -> bool {
    let mut code = 0u32;
    if unsafe { GetExitCodeProcess(process, &mut code) }.is_err() {
        return false;
    }
    code == STILL_ACTIVE.0 as u32
}

fn process_token_is_system(process: HANDLE) -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 || needed > 4096 {
            let _ = CloseHandle(token);
            return false;
        }
        let mut buffer = vec![0u8; needed as usize];
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

fn image_path_is_current_exe(process: HANDLE) -> bool {
    let Some(theirs) = image_path(process) else {
        return false;
    };
    let Some(own_image) = image_path(unsafe { GetCurrentProcess() }) else {
        return false;
    };
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    let theirs = normalize_path(&theirs);
    theirs == normalize_path(&own_image) && theirs == normalize_path(&current.to_string_lossy())
}

fn image_path(process: HANDLE) -> Option<String> {
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
    if queried.is_err() || len == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..len as usize]))
}

fn normalize_path(path: &str) -> String {
    let trimmed = path
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| path.strip_prefix(r"\\?\"))
        .unwrap_or(path);
    trimmed.replace('/', "\\").to_ascii_lowercase()
}

fn current_pid() -> u32 {
    unsafe { GetCurrentProcessId() }
}

fn fresh_nonce() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(1);
    let mixed = nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (u64::from(unsafe { GetCurrentProcessId() }) << 32)
        ^ u64::from(unsafe { GetCurrentThreadId() });
    if mixed == 0 { 1 } else { mixed }
}

fn load_handle(slot: &AtomicIsize) -> Option<HANDLE> {
    let raw = slot.load(Ordering::Acquire);
    if raw == 0 {
        None
    } else {
        Some(HANDLE(raw as *mut std::ffi::c_void))
    }
}

fn store_handle(slot: &AtomicIsize, handle: HANDLE) {
    let previous = slot.swap(handle.0 as isize, Ordering::AcqRel);
    if previous != 0 {
        close_handle(HANDLE(previous as *mut std::ffi::c_void));
    }
}

fn close_stored(slot: &AtomicPtr<std::ffi::c_void>) {
    let raw = slot.swap(std::ptr::null_mut(), Ordering::AcqRel);
    if !raw.is_null() {
        close_handle(HANDLE(raw));
    }
}

fn close_stored_isize(slot: &AtomicIsize) {
    let raw = slot.swap(0, Ordering::AcqRel);
    if raw != 0 {
        close_handle(HANDLE(raw as *mut std::ffi::c_void));
    }
}

fn close_handle(handle: HANDLE) {
    if !handle.is_invalid() && !handle.0.is_null() {
        unsafe {
            let _ = CloseHandle(handle);
        }
    }
}

fn armed_event_name(nonce: u64) -> Vec<u16> {
    format!(r"Local\{}_Armed_{nonce:016x}", softlock_core::SERVICE_NAME)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
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
