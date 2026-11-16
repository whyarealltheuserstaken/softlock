//! Lock and disconnect calls, plus reconnecting the console session afterwards.

use std::thread;
use std::time::Duration;

use windows::Win32::System::RemoteDesktop::{
    WTS_SESSIONSTATE_LOCK, WTS_SESSIONSTATE_UNKNOWN, WTS_SESSIONSTATE_UNLOCK, WTSConnectSessionW,
    WTSConnectState, WTSDisconnectSession, WTSFreeMemory, WTSGetActiveConsoleSessionId, WTSINFOEXW,
    WTSQuerySessionInformationW, WTSSessionInfoEx,
};
use windows::core::{PCWSTR, PWSTR};

use super::util::{self, wide};

pub fn disconnect(session: u32, wait: bool) -> Result<(), String> {
    unsafe { WTSDisconnectSession(None, session, wait) }
        .map_err(|error| util::explain(error, "WTSDisconnectSession"))
}

pub fn lock_workstation() -> Result<(), String> {
    unsafe { windows::Win32::System::Shutdown::LockWorkStation() }
        .map_err(|error| util::explain(error, "LockWorkStation"))
}

pub fn win32_of(error: &str) -> u32 {
    error
        .rsplit("win32 ")
        .next()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

pub fn reconnect(session: u32) -> Result<String, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let mut last = String::from("no console session to attach");
    while std::time::Instant::now() < deadline {
        if connect_state(session).ok() == Some(0) {
            let console = unsafe { WTSGetActiveConsoleSessionId() };
            if console == session {
                return Ok("already-active".to_string());
            }
        }
        let console = unsafe { WTSGetActiveConsoleSessionId() };
        if console != u32::MAX && console != 0 && console != session {
            match connect(session, console) {
                Ok(()) => {
                    if wait_until_active(session, Duration::from_secs(2)).is_ok() {
                        return Ok(format!("session-to-console:{console}"));
                    }
                    last = format!(
                        "session-to-console:{console} returned success but session {session} stayed disconnected"
                    );
                }
                Err(error) => last = format!("session-to-console:{console}: {error}"),
            }
        } else {
            last = format!("console session is {console}");
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err(last)
}

fn connect(logon: u32, target: u32) -> Result<(), String> {
    let password = wide("");
    unsafe { WTSConnectSessionW(logon, target, PCWSTR(password.as_ptr()), false) }
        .map_err(|error| util::explain(error, "WTSConnectSessionW"))
}

pub fn wait_until_active(session: u32, timeout: Duration) -> Result<(), String> {
    let start = std::time::Instant::now();
    let mut last = String::new();
    while start.elapsed() < timeout {
        match connect_state(session) {
            Ok(0) => return Ok(()),
            Ok(state) => last = format!("state {state}"),
            Err(error) => last = error,
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(format!("session {session} did not become active ({last})"))
}

fn connect_state(session: u32) -> Result<i32, String> {
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        WTSQuerySessionInformationW(None, session, WTSConnectState, &mut buffer, &mut bytes)
            .map_err(|error| util::explain(error, "WTSQuerySessionInformationW"))?;
        if buffer.is_null() {
            return Err("session state buffer was empty".to_string());
        }
        let state = *buffer.0.cast::<i32>();
        WTSFreeMemory(buffer.0.cast());
        Ok(state)
    }
}

pub fn console_session() -> u32 {
    unsafe { WTSGetActiveConsoleSessionId() }
}

/// `Ok(Some(true))` when the session lock flag says locked.
/// `Ok(None)` when Windows reports the flag as unknown.
pub fn session_lock_state(session: u32) -> Result<Option<bool>, String> {
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        WTSQuerySessionInformationW(None, session, WTSSessionInfoEx, &mut buffer, &mut bytes)
            .map_err(|error| util::explain(error, "WTSQuerySessionInformationW"))?;
        if buffer.is_null() {
            return Err("session info buffer was empty".to_string());
        }
        let info = &*buffer.0.cast::<WTSINFOEXW>();
        let level = info.Level;
        let flags = info.Data.WTSInfoExLevel1.SessionFlags as u32;
        WTSFreeMemory(buffer.0.cast());
        if level != 1 {
            return Err(format!("session info level {level}"));
        }
        if flags == WTS_SESSIONSTATE_LOCK {
            Ok(Some(true))
        } else if flags == WTS_SESSIONSTATE_UNLOCK {
            Ok(Some(false))
        } else if flags == WTS_SESSIONSTATE_UNKNOWN {
            Ok(None)
        } else {
            Err(format!("session lock flag {flags}"))
        }
    }
}
