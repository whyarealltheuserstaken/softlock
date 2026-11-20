use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
};
use windows::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTS_SESSIONSTATE_LOCK, WTS_SESSIONSTATE_UNLOCK, WTSActive,
    WTSClientProtocolType, WTSConnectSessionW, WTSConnectState, WTSDisconnected, WTSFreeMemory,
    WTSGetActiveConsoleSessionId, WTSINFOEXW, WTSQuerySessionInformationW, WTSSessionInfoEx,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcessToken};
use windows::core::{PCWSTR, PWSTR, w};

use super::util::success;

const NO_CONSOLE: u32 = u32::MAX;

pub fn school_workstation() -> bool {
    reg_sz(
        w!("HARDWARE\\DESCRIPTION\\System\\BIOS"),
        w!("SystemProductName"),
    )
    .is_some_and(|name| name.to_ascii_lowercase().contains("thinkstation p7"))
}

pub fn preflight() -> u8 {
    let report = describe();
    if crate::emit(&report) { 0 } else { 1 }
}

pub fn status() -> u8 {
    let session = own_session().unwrap_or(NO_CONSOLE);
    let locked = session_is_locked(session);
    let connected = session_connect_state(session) == Some(WTSActive.0 as u32);
    let report = Status {
        locked: locked.unwrap_or(false),
        lock_known: locked.is_some(),
        connected,
        flags: session_flags(session).unwrap_or(-1),
        session,
        console: unsafe { WTSGetActiveConsoleSessionId() },
    };
    if crate::emit(&report) { 0 } else { 1 }
}

pub fn own_session() -> Option<u32> {
    let mut session = 0u32;
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session).ok()? };
    Some(session)
}

pub fn console_session() -> u32 {
    unsafe { WTSGetActiveConsoleSessionId() }
}

#[derive(Serialize)]
struct Preflight {
    ok: bool,
    elevated: bool,
    school: bool,
    protocol: i32,
    session: u32,
    console: u32,
    console_present: bool,
    remote: bool,
    build: String,
    installation_type: String,
}

#[derive(Serialize)]
struct Status {
    locked: bool,
    lock_known: bool,
    connected: bool,
    flags: i32,
    session: u32,
    console: u32,
}

fn describe() -> Preflight {
    let session = own_session().unwrap_or(NO_CONSOLE);
    let console = console_session();
    let console_present = console != NO_CONSOLE;
    let protocol = client_protocol(session).map(i32::from);
    let remote = match protocol {
        Some(value) if value > 0 => true,
        _ => console_present && session != console,
    };
    let ok = elevated() && !school_workstation() && console_present && !remote;
    Preflight {
        ok,
        elevated: elevated(),
        school: school_workstation(),
        protocol: protocol.unwrap_or(-1),
        session,
        console,
        console_present,
        remote,
        build: reg_sz(
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            w!("CurrentBuild"),
        )
        .unwrap_or_default(),
        installation_type: reg_sz(
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            w!("InstallationType"),
        )
        .unwrap_or_default(),
    }
}

pub fn elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        let read = GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        let _ = CloseHandle(token);
        read.is_ok() && elevation.TokenIsElevated != 0
    }
}

pub fn session_is_locked(session: u32) -> Option<bool> {
    let flags = session_flags(session)?;
    if flags != WTS_SESSIONSTATE_LOCK as i32 && flags != WTS_SESSIONSTATE_UNLOCK as i32 {
        return None;
    }
    Some(flags == WTS_SESSIONSTATE_LOCK as i32 && flags != WTS_SESSIONSTATE_UNLOCK as i32)
}

pub fn session_is_disconnected(session: u32) -> bool {
    session_connect_state(session) == Some(WTSDisconnected.0 as u32)
}

pub fn session_is_active(session: u32) -> bool {
    session_connect_state(session) == Some(WTSActive.0 as u32)
}

/// Attach `session` to the console. The service calls this so a disconnect
/// measurement can finish and the proof can still record the result. It is
/// not an unlock API; the password is empty because the service token, not a
/// credential, is what the call accepts.
pub fn connect_session(session: u32) -> bool {
    if session == NO_CONSOLE {
        return false;
    }
    let connected = unsafe { WTSConnectSessionW(session, u32::MAX, w!(""), true).is_ok() };
    connected && wait_until_active(session, std::time::Duration::from_secs(8))
}

fn session_flags(session: u32) -> Option<i32> {
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        if WTSQuerySessionInformationW(None, session, WTSSessionInfoEx, &mut buffer, &mut bytes)
            .is_err()
            || buffer.is_null()
        {
            return None;
        }
        let info = &*buffer.0.cast::<WTSINFOEXW>();
        let level = info.Level;
        let flags = info.Data.WTSInfoExLevel1.SessionFlags;
        WTSFreeMemory(buffer.0.cast());
        if level == 1 { Some(flags) } else { None }
    }
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

fn client_protocol(session: u32) -> Option<u16> {
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        if WTSQuerySessionInformationW(
            None,
            session,
            WTSClientProtocolType,
            &mut buffer,
            &mut bytes,
        )
        .is_err()
            || buffer.is_null()
        {
            return None;
        }
        let protocol = *buffer.0.cast::<u16>();
        WTSFreeMemory(buffer.0.cast());
        Some(protocol)
    }
}

fn reg_sz(subkey: PCWSTR, value: PCWSTR) -> Option<String> {
    unsafe {
        let mut key = HKEY::default();
        if !success(RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            subkey,
            Some(0),
            KEY_READ,
            &mut key,
        )) {
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
        Some(String::from_utf16_lossy(&data[..end]))
    }
}

pub fn wait_until_active(session: u32, timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if session_is_active(session) && !session_is_disconnected(session) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    session_is_active(session)
}
