//! Real Windows lock when the guard can no longer be guaranteed.
//!
//! Finding 04: `LockWorkStation` does not lock while `DisableLockWorkstation`
//! is set, and it does lock once that value is clear. This dev launch never
//! sets that value and never clears it. Ticket 15 clears it before calling
//! `LockWorkStation`. If the session is not actually locked, the in-session
//! fall-back is `WTSDisconnectSession`, which does not depend on the policy.
//! If that fails too, the caller releases the overlay and the hooks. Leaving
//! them up would block input with no OS lock and no way out.

use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTS_SESSIONSTATE_LOCK, WTS_SESSIONSTATE_UNLOCK, WTSConnectState,
    WTSDisconnectSession, WTSDisconnected, WTSFreeMemory, WTSGetActiveConsoleSessionId, WTSINFOEXW,
    WTSQuerySessionInformationW, WTSSessionInfoEx,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};
use windows::core::PWSTR;

use super::session::Shared;
use crate::real_lock::{RealLock, decide};

pub(crate) fn lock_or_disconnect(session: u32) -> RealLock {
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
