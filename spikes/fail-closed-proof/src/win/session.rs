//! Lock and disconnect calls, plus reconnecting the console session afterwards.

use std::thread;
use std::time::Duration;

use windows::Win32::System::RemoteDesktop::{
    WTSConnectSessionW, WTSConnectState, WTSDisconnectSession, WTSFreeMemory,
    WTSGetActiveConsoleSessionId, WTSQuerySessionInformationW,
};
use windows::core::{PCWSTR, PWSTR};

use super::util::{self, wide};

pub const LOGONID_CURRENT: u32 = 0xFFFF_FFFF;

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
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    let mut attempts = vec![
        (session, LOGONID_CURRENT, "session-to-current"),
        (session, session, "session-to-self"),
    ];
    if console != u32::MAX && console != 0 && console != session {
        attempts.push((session, console, "session-to-console"));
        attempts.push((LOGONID_CURRENT, console, "current-to-console"));
    }
    let mut errors = Vec::new();
    for (logon, target, label) in attempts {
        if logon == LOGONID_CURRENT && target == LOGONID_CURRENT {
            continue;
        }
        match connect(logon, target) {
            Ok(()) => {
                if wait_until_active(session, Duration::from_secs(5)).is_ok() {
                    return Ok(label.to_string());
                }
                errors.push(format!(
                    "{label}: the call returned success but session {session} stayed disconnected"
                ));
            }
            Err(error) => errors.push(format!("{label}: {error}")),
        }
    }
    Err(errors.join("; "))
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
