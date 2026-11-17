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
#[cfg(feature = "simulated-physical")]
use std::sync::atomic::AtomicU8;
use std::sync::atomic::{AtomicIsize, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_ALREADY_EXISTS, GetLastError,
    HANDLE, HLOCAL, HWND, INVALID_HANDLE_VALUE, LocalFree, STILL_ACTIVE, SetLastError,
    WAIT_OBJECT_0, WIN32_ERROR,
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
    CreateEventW, EVENT_ALL_ACCESS, GetCurrentProcess, GetCurrentProcessId, GetExitCodeProcess,
    OpenEventW, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, ResetEvent, SetEvent,
    WaitForSingleObject,
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
/// Nonce this process published. The impostor seam uses it so the armed event
/// exists and is signaled; token and path are what can still reject.
static PUBLISHED_NONCE: AtomicU64 = AtomicU64::new(0);
/// PID of a replacement this host already held and then saw exit. That number
/// is never trusted again: the operating system can reuse it.
static DEAD_PID: AtomicU32 = AtomicU32::new(0);
/// The dead PID was passed to `trust_new_process` after the handle was dropped.
#[cfg(feature = "simulated-physical")]
static REOPENED_DEAD_PID: AtomicBool = AtomicBool::new(false);
/// The reuse fixture already wrote a specific failure. Later polls must not
/// replace it with the pending line.
#[cfg(feature = "simulated-physical")]
static REUSE_FIXED: AtomicBool = AtomicBool::new(false);
/// Why the last armed-event check failed. 1 open, 2 dacl, 3 signal.
#[cfg(feature = "simulated-physical")]
static EVENT_FAIL: AtomicU8 = AtomicU8::new(0);
/// SDDL of the object that last failed the SYSTEM-only check.
#[cfg(feature = "simulated-physical")]
static LAST_SDDL: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
const ARMED_SLOT_BYTES: usize = 16;

use super::session::Shared;
use crate::real_lock::{RealLock, decide};

/// Last stub seen by a test build. A panic can clear the installed session
/// before a second lock attempt. The default is failure, so that attempt
/// cannot call `LockWorkStation` and lock the runner.
#[cfg(feature = "simulated-physical")]
static OS_LOCK_STUB: AtomicU8 = AtomicU8::new(1);

pub(crate) fn lock_or_disconnect(session: u32) -> RealLock {
    note_os_lock_attempt();
    let mut stub = os_lock_stub_fallback();
    if let Some(shared) = super::session::installed_session() {
        shared
            .os_lock_attempts
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        stub = shared
            .os_lock_stub
            .load(std::sync::atomic::Ordering::Acquire);
        remember_os_lock_stub(stub);
    }
    match stub {
        0 => {}
        // Confirmed workstation lock, without calling Windows.
        2 => return decide(true, false),
        // The Windows calls are not made. Neither path confirmed.
        _ => return decide(false, false),
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

fn os_lock_stub_fallback() -> u8 {
    #[cfg(feature = "simulated-physical")]
    {
        OS_LOCK_STUB.load(Ordering::Acquire)
    }
    #[cfg(not(feature = "simulated-physical"))]
    {
        0
    }
}

fn remember_os_lock_stub(stub: u8) {
    #[cfg(feature = "simulated-physical")]
    OS_LOCK_STUB.store(stub, Ordering::Release);
    #[cfg(not(feature = "simulated-physical"))]
    {
        let _ = stub;
    }
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
    let Some(nonce) = fresh_nonce() else {
        return;
    };
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
    PUBLISHED_NONCE.store(nonce, Ordering::Release);
}

/// Another live lock host has armed. PID and image name are not enough: the
/// PID can be reused by a renamed standard-user process after the replacement
/// dies. Release only while this host still holds that process's handle, the
/// token is SYSTEM, the full image path is this executable, and the per-run
/// SYSTEM-only event is signaled. Any doubt keeps the hooks.
pub(crate) fn replacement_host_armed() -> bool {
    #[cfg(feature = "simulated-physical")]
    prepare_reuse_fixture();
    let armed = replacement_host_armed_inner();
    #[cfg(feature = "simulated-physical")]
    note_reuse_witness();
    armed
}

fn replacement_host_armed_inner() -> bool {
    if let Some((pid, nonce)) = read_trusted_slot() {
        if held_matches(pid, nonce) {
            if held_still_armed() {
                return true;
            }
            let process = load_handle(&HELD_PROCESS);
            let exited = process.is_none_or(|handle| !process_still_active(handle));
            if exited {
                // The handle is the identity. Drop it and never open this PID
                // again, including on a later poll after the number is reused.
                DEAD_PID.store(pid, Ordering::Release);
            }
            drop_held();
            if exited {
                return consider_impostor();
            }
        }
        if dead_pid(pid) {
            return consider_impostor();
        }
        remember_exited_hold();
        drop_held();
        if trust_new_process(pid, nonce, true).is_ok() {
            return true;
        }
    } else {
        remember_exited_hold();
        drop_held();
    }
    consider_impostor()
}

/// A held process that has exited is dead even when the slot cannot be read.
fn remember_exited_hold() {
    let pid = HELD_PID.load(Ordering::Acquire);
    let process = load_handle(&HELD_PROCESS);
    if pid != 0 && process.is_some_and(|handle| !process_still_active(handle)) {
        DEAD_PID.store(pid, Ordering::Release);
    }
}

fn dead_pid(pid: u32) -> bool {
    let dead = DEAD_PID.load(Ordering::Acquire);
    dead != 0 && pid == dead
}

pub(crate) fn clear_armed_host() {
    drop_held();
    let raw = ARMED_EVENT.swap(std::ptr::null_mut(), Ordering::AcqRel);
    if !raw.is_null() {
        let event = HANDLE(raw);
        // Manual-reset. Reset before the last close so signaled means armed.
        unsafe {
            let _ = ResetEvent(event);
        }
        close_handle(event);
    }
    close_stored(&ARMED_MAPPING);
    PUBLISHED_NONCE.store(0, Ordering::Release);
}

fn create_armed_mapping() -> Option<HANDLE> {
    let descriptor = armed_descriptor()?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let name = armed_host_name();
    unsafe { SetLastError(WIN32_ERROR(0)) };
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
    // CreateEvent leaves the previous error in place when it creates a new
    // object. Clear it so a stale ERROR_ALREADY_EXISTS cannot refuse publish.
    unsafe { SetLastError(WIN32_ERROR(0)) };
    let event = unsafe { CreateEventW(Some(&sa), true, false, PCWSTR(name.as_ptr())) };
    // Captured before any other call. An existing object may be a squatter.
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    free_security(&descriptor);
    let event = event.ok()?;
    if already {
        close_handle(event);
        return None;
    }
    Some(event)
}

fn confirmed_event(nonce: u64) -> Option<HANDLE> {
    if nonce == 0 {
        return None;
    }
    // This host already holds the event it published. Re-opening by name can
    // fail even for the creator; the held handle is the one that was signaled.
    if nonce == PUBLISHED_NONCE.load(Ordering::Acquire) {
        let raw = ARMED_EVENT.load(Ordering::Acquire);
        if raw.is_null() {
            note_event_fail(1);
            return None;
        }
        let event = HANDLE(raw);
        if !mapping_is_system_only(event) {
            note_event_fail(2);
            return None;
        }
        if !event_is_signaled(event) {
            note_event_fail(3);
            return None;
        }
        return duplicate_handle(event);
    }
    let name = armed_event_name(nonce);
    let Ok(event) = (unsafe { OpenEventW(EVENT_ALL_ACCESS, false, PCWSTR(name.as_ptr())) }) else {
        note_event_fail(1);
        return None;
    };
    if !mapping_is_system_only(event) {
        close_handle(event);
        note_event_fail(2);
        return None;
    }
    if !event_is_signaled(event) {
        close_handle(event);
        note_event_fail(3);
        return None;
    }
    Some(event)
}

fn note_event_fail(code: u8) {
    #[cfg(feature = "simulated-physical")]
    EVENT_FAIL.store(code, Ordering::Release);
    let _ = code;
}

fn duplicate_handle(handle: HANDLE) -> Option<HANDLE> {
    let mut target = HANDLE::default();
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &mut target,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
        .ok()?;
    }
    Some(target)
}

fn event_is_signaled(event: HANDLE) -> bool {
    let waited = unsafe { WaitForSingleObject(event, 0) };
    waited == WAIT_OBJECT_0
}

enum ArmReject {
    Event,
    Token,
    Path,
    Inactive,
}

fn trust_new_process(pid: u32, nonce: u64, require_same_slot: bool) -> Result<(), ArmReject> {
    if pid == 0 || nonce == 0 || pid == current_pid() {
        return Err(ArmReject::Inactive);
    }
    if dead_pid(pid) {
        #[cfg(feature = "simulated-physical")]
        REOPENED_DEAD_PID.store(true, Ordering::Release);
        return Err(ArmReject::Inactive);
    }
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return Err(ArmReject::Inactive);
    };
    if require_same_slot && read_trusted_slot() != Some((pid, nonce)) {
        close_handle(process);
        return Err(ArmReject::Inactive);
    }
    let Some(event) = confirmed_event(nonce) else {
        close_handle(process);
        return Err(ArmReject::Event);
    };
    if !process_still_active(process) {
        close_handle(event);
        close_handle(process);
        return Err(ArmReject::Inactive);
    }
    if !image_path_is_current_exe(process) {
        close_handle(event);
        close_handle(process);
        return Err(ArmReject::Path);
    }
    if !process_still_active(process) {
        close_handle(event);
        close_handle(process);
        return Err(ArmReject::Inactive);
    }
    store_held(process, event, pid, nonce);
    if held_still_armed() {
        Ok(())
    } else {
        drop_held();
        Err(ArmReject::Inactive)
    }
}

fn consider_impostor() -> bool {
    #[cfg(not(feature = "simulated-physical"))]
    {
        false
    }
    #[cfg(feature = "simulated-physical")]
    {
        match impostor_case() {
            ImpostorCase::None | ImpostorCase::Reuse => false,
            ImpostorCase::Token => judge_impostor_pid(false),
            ImpostorCase::Path => judge_impostor_pid(true),
        }
    }
}

#[cfg(feature = "simulated-physical")]
enum ImpostorCase {
    None,
    Token,
    Path,
    Reuse,
}

#[cfg(feature = "simulated-physical")]
fn impostor_case() -> ImpostorCase {
    match std::env::var("SOFTLOCK_IMPOSTOR_CASE").ok().as_deref() {
        Some("path") => ImpostorCase::Path,
        Some("reuse") => ImpostorCase::Reuse,
        Some("token") => ImpostorCase::Token,
        _ => ImpostorCase::None,
    }
}

/// The published nonce's event is ours: it exists, is SYSTEM-only, and is
/// signaled. A standard-user copy fails the token check. A SYSTEM process
/// started from another path fails the path check.
#[cfg(feature = "simulated-physical")]
fn judge_impostor_pid(spawn_other_path: bool) -> bool {
    let Some(nonce) = published_nonce() else {
        let reason = event_witness();
        note_impostor(&reason);
        return false;
    };
    let mut creator: Option<HANDLE> = None;
    let pid = if spawn_other_path {
        let Some((process, pid)) = other_path_process() else {
            note_impostor("reason=setup");
            return false;
        };
        creator = Some(process);
        pid
    } else {
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
        pid
    };
    let decision = trust_new_process(pid, nonce, false);
    if let Some(process) = creator {
        terminate_handle(process);
    }
    let reason = match decision {
        Ok(()) => "released".to_string(),
        Err(ArmReject::Token) => "reason=token".to_string(),
        Err(ArmReject::Path) => "reason=path".to_string(),
        Err(ArmReject::Event) => event_witness(),
        Err(ArmReject::Inactive) => "reason=inactive".to_string(),
    };
    note_impostor(&reason);
    false
}

#[cfg(feature = "simulated-physical")]
fn impostor_pid() -> Option<u32> {
    let path = std::env::var("SOFTLOCK_IMPOSTOR_PID").ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let pid = text.trim().parse::<u32>().ok()?;
    if pid == 0 { None } else { Some(pid) }
}

#[cfg(feature = "simulated-physical")]
fn note_impostor(text: &str) {
    static NOTED: AtomicBool = AtomicBool::new(false);
    if NOTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let Ok(path) = std::env::var("SOFTLOCK_IMPOSTOR_WITNESS") else {
        return;
    };
    let _ = std::fs::write(path, text);
}

#[cfg(feature = "simulated-physical")]
fn event_witness() -> String {
    match EVENT_FAIL.load(Ordering::Acquire) {
        1 => "reason=event-open".to_string(),
        2 => {
            let sddl = LAST_SDDL
                .lock()
                .map(|text| text.clone())
                .unwrap_or_default();
            format!("reason=event-dacl:{sddl}")
        }
        3 => "reason=event-signal".to_string(),
        _ => "reason=event".to_string(),
    }
}

#[cfg(feature = "simulated-physical")]
fn published_nonce() -> Option<u64> {
    let nonce = PUBLISHED_NONCE.load(Ordering::Acquire);
    if nonce == 0 { None } else { Some(nonce) }
}

/// Hold a second SYSTEM process at this exact image, then kill it, so the
/// next check sees a dead held PID. Windows will not recycle that number
/// during the test; the proof is that the handle is dropped and the PID is
/// not opened again.
#[cfg(feature = "simulated-physical")]
fn prepare_reuse_fixture() {
    static ONCE: AtomicBool = AtomicBool::new(false);
    if !matches!(impostor_case(), ImpostorCase::Reuse) || ONCE.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(nonce) = published_nonce() else {
        note_reuse_blocked("reason=setup:nonce");
        return;
    };
    let Some((process, pid)) = spawn_suspended_image(&current_image()) else {
        note_reuse_blocked("reason=setup:spawn");
        return;
    };
    if let Err(reject) = trust_new_process(pid, nonce, false) {
        let reason = match reject {
            ArmReject::Token => "reason=token".to_string(),
            ArmReject::Path => "reason=path".to_string(),
            ArmReject::Event => event_witness(),
            ArmReject::Inactive => "reason=inactive".to_string(),
        };
        note_reuse_blocked(&reason);
        terminate_handle(process);
        return;
    }
    let mapping = ARMED_MAPPING.load(Ordering::Acquire);
    if mapping.is_null() || !write_armed_slot(HANDLE(mapping), pid, nonce) {
        note_reuse_blocked("reason=setup:slot");
        terminate_handle(process);
        drop_held();
        return;
    }
    terminate_handle(process);
}

#[cfg(feature = "simulated-physical")]
fn note_reuse_witness() {
    if !matches!(impostor_case(), ImpostorCase::Reuse) {
        return;
    }
    if DEAD_PID.load(Ordering::Acquire) == 0 {
        if REUSE_FIXED.load(Ordering::Acquire) {
            return;
        }
        let sddl = LAST_SDDL
            .lock()
            .map(|text| text.clone())
            .unwrap_or_default();
        note_reuse_file(&format!(
            "reason=reuse-pending:nonce={} held={} fail={} sddl={sddl}",
            PUBLISHED_NONCE.load(Ordering::Acquire),
            HELD_PID.load(Ordering::Acquire),
            EVENT_FAIL.load(Ordering::Acquire),
        ));
        return;
    }
    let dropped = HELD_PROCESS.load(Ordering::Acquire) == 0;
    let reopened = REOPENED_DEAD_PID.load(Ordering::Acquire);
    note_reuse_file(&format!(
        "dropped={} reopened={} rejected=1",
        u8::from(dropped),
        u8::from(reopened)
    ));
}

#[cfg(feature = "simulated-physical")]
fn note_reuse_blocked(text: &str) {
    REUSE_FIXED.store(true, Ordering::Release);
    note_reuse_file(text);
}

#[cfg(feature = "simulated-physical")]
fn note_reuse_file(text: &str) {
    let Ok(path) = std::env::var("SOFTLOCK_IMPOSTOR_WITNESS") else {
        return;
    };
    let _ = std::fs::write(path, text);
}

#[cfg(feature = "simulated-physical")]
fn other_path_process() -> Option<(HANDLE, u32)> {
    static PID: AtomicU32 = AtomicU32::new(0);
    let existing = PID.load(Ordering::Acquire);
    if existing != 0 {
        return None;
    }
    let source = current_image();
    if source.as_os_str().is_empty() {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("softlock-path-{}", current_pid()));
    std::fs::create_dir_all(&dir).ok()?;
    let dest = dir.join("other-lockhost.exe");
    std::fs::copy(&source, &dest).ok()?;
    let spawned = spawn_suspended_image(&dest)?;
    PID.store(spawned.1, Ordering::Release);
    Some(spawned)
}

#[cfg(feature = "simulated-physical")]
fn current_image() -> std::path::PathBuf {
    std::env::current_exe().unwrap_or_default()
}

#[cfg(feature = "simulated-physical")]
fn spawn_suspended_image(image: &std::path::Path) -> Option<(HANDLE, u32)> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Threading::{
        CREATE_SUSPENDED, CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW,
    };
    let wide: Vec<u16> = image
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..STARTUPINFOW::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR(wide.as_ptr()),
            None,
            None,
            None,
            false,
            CREATE_SUSPENDED,
            None,
            PCWSTR::null(),
            &startup,
            &mut info,
        )
        .ok()?;
        let _ = CloseHandle(info.hThread);
    }
    Some((info.hProcess, info.dwProcessId))
}

#[cfg(feature = "simulated-physical")]
fn terminate_handle(process: HANDLE) {
    use windows::Win32::System::Threading::TerminateProcess;
    unsafe {
        let _ = TerminateProcess(process, 1);
        let _ = WaitForSingleObject(process, 2000);
    }
    close_handle(process);
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

fn fresh_nonce() -> Option<u64> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).ok()?;
    // Zero is not a nonce. Forcing the low bit keeps the value non-zero.
    Some(u64::from_le_bytes(bytes) | 1)
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
    let text = security_sddl(mapping);
    remember_sddl(&text);
    sddl_is_system_only(&text)
}

fn security_sddl(mapping: HANDLE) -> String {
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
        return String::new();
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
    text
}

/// Owner is SYSTEM and the DACL is one protected allow ACE for SYSTEM.
/// `CreateEvent` rewrites `FA` to the event's own access mask, so the string
/// that comes back is not the literal `D:P(A;;FA;;;SY)`.
fn sddl_is_system_only(text: &str) -> bool {
    if !text.contains("O:SY") {
        return false;
    }
    let Some(dacl) = text.split("D:").nth(1) else {
        return false;
    };
    let Some(open) = dacl.find('(') else {
        return false;
    };
    if &dacl[..open] != "P" {
        return false;
    }
    let Some(close) = dacl[open + 1..].find(')') else {
        return false;
    };
    let ace = &dacl[open + 1..open + 1 + close];
    if dacl[open + 1 + close + 1..].contains('(') {
        return false;
    }
    let mut parts = ace.split(';');
    let kind = parts.next();
    let flags = parts.next();
    let rights = parts.next();
    let _object = parts.next();
    let _inherit = parts.next();
    let sid = parts.next();
    kind == Some("A")
        && flags == Some("")
        && sid == Some("SY")
        && parts.next().is_none()
        && rights.is_some_and(system_control_rights)
}

fn system_control_rights(mask: &str) -> bool {
    if matches!(mask, "FA" | "GA" | "KA") {
        return true;
    }
    let Some(hex) = mask.strip_prefix("0x").or_else(|| mask.strip_prefix("0X")) else {
        return false;
    };
    let Ok(value) = u32::from_str_radix(hex, 16) else {
        return false;
    };
    // Event ALL_ACCESS is 0x1F0003. A section's FILE_MAP_ALL_ACCESS is
    // 0x000F001F and does not include SYNCHRONIZE, so either full mask counts.
    const EVENT_ALL: u32 = 0x1F0003;
    const FILE_MAP_ALL: u32 = 0x000F001F;
    (value & EVENT_ALL) == EVENT_ALL || (value & FILE_MAP_ALL) == FILE_MAP_ALL
}

fn remember_sddl(text: &str) {
    #[cfg(feature = "simulated-physical")]
    if let Ok(mut slot) = LAST_SDDL.lock() {
        *slot = text.to_string();
    }
    let _ = text;
}

fn armed_descriptor() -> Option<PSECURITY_DESCRIPTOR> {
    // O:SY is explicit. LocalSystem's default owner is Administrators, and a
    // non-SYSTEM caller cannot create an object with this owner.
    let sddl: Vec<u16> = "O:SYD:P(A;;FA;;;SY)"
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

#[cfg(test)]
mod sddl_tests {
    use super::sddl_is_system_only;

    #[test]
    fn literal_and_rewritten_masks_are_system_only() {
        assert!(sddl_is_system_only("O:SYD:P(A;;FA;;;SY)"));
        assert!(sddl_is_system_only("O:SYD:P(A;;0x1F0003;;;SY)"));
        assert!(sddl_is_system_only("O:SYD:P(A;;0x1f0003;;;SY)"));
        assert!(sddl_is_system_only("O:SYG:SYD:P(A;;0x1F01FF;;;SY)"));
        assert!(sddl_is_system_only("O:SYD:P(A;;0x000f001f;;;SY)"));
    }

    #[test]
    fn a_second_ace_or_another_owner_is_rejected() {
        assert!(!sddl_is_system_only("O:BAD:P(A;;FA;;;SY)"));
        assert!(!sddl_is_system_only("O:BAD:P(A;;0x1f0003;;;SY)"));
        assert!(!sddl_is_system_only("O:SYD:P(A;;FA;;;SY)(A;;FA;;;WD)"));
        assert!(!sddl_is_system_only("O:SYD:(A;;FA;;;SY)"));
        assert!(!sddl_is_system_only("O:SYD:P(A;;0x1;;;SY)"));
    }
}
