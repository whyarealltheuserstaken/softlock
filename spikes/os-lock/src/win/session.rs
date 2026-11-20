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
    WTSUserName,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcessToken};
use windows::Win32::System::WindowsProgramming::GetUserNameW;
use windows::core::{PCWSTR, PWSTR, w};

use super::util::success;
use crate::school::{self, SchoolCheck};

const NO_CONSOLE: u32 = u32::MAX;

pub fn school_check() -> SchoolCheck {
    let bios = w!("HARDWARE\\DESCRIPTION\\System\\BIOS");
    school::classify(
        reg_sz(bios, w!("SystemFamily")).as_deref(),
        reg_sz(bios, w!("SystemVersion")).as_deref(),
        reg_sz(bios, w!("SystemProductName")).as_deref(),
    )
}

pub fn school_workstation() -> bool {
    !matches!(school_check(), SchoolCheck::No)
}

/// Field names that could not be read. Values are not included.
pub fn missing_identity_fields() -> Vec<&'static str> {
    let bios = w!("HARDWARE\\DESCRIPTION\\System\\BIOS");
    let mut missing = Vec::new();
    if reg_sz(bios, w!("SystemFamily"))
        .filter(|value| !value.is_empty())
        .is_none()
    {
        missing.push("SystemFamily");
    }
    if reg_sz(bios, w!("SystemVersion"))
        .filter(|value| !value.is_empty())
        .is_none()
    {
        missing.push("SystemVersion");
    }
    if reg_sz(bios, w!("SystemProductName"))
        .filter(|value| !value.is_empty())
        .is_none()
    {
        missing.push("SystemProductName");
    }
    missing
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
    identity_unreadable: bool,
    protocol: i32,
    session: u32,
    console: u32,
    console_present: bool,
    remote: bool,
    console_user_known: bool,
    console_user_match: bool,
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
    let identity = school_check();
    let account =
        school::accounts_match(process_user().as_deref(), session_user(console).as_deref());
    let console_user_known = account.is_ok();
    let console_user_match = account.unwrap_or(false);
    let ok = elevated()
        && identity == SchoolCheck::No
        && console_present
        && !remote
        && console_user_match;
    Preflight {
        ok,
        elevated: elevated(),
        school: identity == SchoolCheck::Yes,
        identity_unreadable: identity == SchoolCheck::Unreadable,
        protocol: protocol.unwrap_or(-1),
        session,
        console,
        console_present,
        remote,
        console_user_known,
        console_user_match,
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

/// SYSTEM session reconnect. The service calls this only when it was installed
/// with `--ci-reconnect`, which the installer adds only for a GitHub-hosted
/// Actions runner. The password is empty because the service token is what
/// the call accepts. This is not used on a laptop run.
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

fn process_user() -> Option<String> {
    let mut buffer = [0u16; 256];
    let mut len = buffer.len() as u32;
    unsafe {
        GetUserNameW(Some(PWSTR(buffer.as_mut_ptr())), &mut len).ok()?;
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    let name = String::from_utf16_lossy(&buffer[..end]);
    if name.is_empty() { None } else { Some(name) }
}

fn session_user(session: u32) -> Option<String> {
    if session == NO_CONSOLE {
        return None;
    }
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        if WTSQuerySessionInformationW(None, session, WTSUserName, &mut buffer, &mut bytes).is_err()
            || buffer.is_null()
        {
            return None;
        }
        let mut end = 0usize;
        while *buffer.0.add(end) != 0 {
            end += 1;
            if end > 256 {
                break;
            }
        }
        let name = String::from_utf16_lossy(std::slice::from_raw_parts(buffer.0, end));
        WTSFreeMemory(buffer.0.cast());
        if name.is_empty() { None } else { Some(name) }
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
