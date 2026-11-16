use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::StationsAndDesktops::{
    GetProcessWindowStation, GetThreadDesktop, GetUserObjectInformationW, HDESK, HWINSTA, UOI_NAME,
};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::System::WindowsProgramming::GetUserNameW;
use windows::core::w;

use super::util::{from_wide, pwstr};

pub struct Identity {
    pub pid: u32,
    pub session: u32,
    pub user: String,
    pub station: String,
    pub desktop: String,
    pub timeout_ms: u64,
    pub timeout_raw: String,
}

pub fn current() -> Identity {
    let (timeout_ms, timeout_raw) = low_level_hooks_timeout();
    Identity {
        pid: unsafe { GetCurrentProcessId() },
        session: current_session(),
        user: user_name(),
        station: object_name(window_station_handle()),
        desktop: object_name(desktop_handle()),
        timeout_ms,
        timeout_raw,
    }
}

pub fn low_level_hooks_timeout() -> (u64, String) {
    let mut buf = [0u16; 32];
    let mut bytes = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\Desktop"),
            w!("LowLevelHooksTimeout"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status != ERROR_SUCCESS {
        return (300, format!("absent:{}", status.0));
    }
    let text = from_wide(&buf);
    let trimmed = text.trim();
    match trimmed.parse::<u64>() {
        Ok(value) if value >= 50 => (value, trimmed.to_string()),
        Ok(value) => (300, format!("ignored:{value}")),
        Err(_) => (300, format!("unparsed:{trimmed}")),
    }
}

fn current_session() -> u32 {
    let mut session = 0u32;
    let ok = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
    if ok.is_ok() { session } else { u32::MAX }
}

fn user_name() -> String {
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    let ok = unsafe { GetUserNameW(Some(pwstr(&mut buf)), &mut len) };
    if ok.is_ok() {
        from_wide(&buf)
    } else {
        String::new()
    }
}

fn window_station_handle() -> HANDLE {
    match unsafe { GetProcessWindowStation() } {
        Ok(HWINSTA(value)) => HANDLE(value),
        Err(_) => HANDLE::default(),
    }
}

fn desktop_handle() -> HANDLE {
    match unsafe { GetThreadDesktop(GetCurrentThreadId()) } {
        Ok(HDESK(value)) => HANDLE(value),
        Err(_) => HANDLE::default(),
    }
}

fn object_name(handle: HANDLE) -> String {
    if handle.0.is_null() {
        return String::new();
    }
    let mut buf = [0u16; 128];
    let mut needed = 0u32;
    let ok = unsafe {
        GetUserObjectInformationW(
            handle,
            UOI_NAME,
            Some(buf.as_mut_ptr().cast()),
            (buf.len() * 2) as u32,
            Some(&mut needed),
        )
    };
    if ok.is_ok() {
        from_wide(&buf)
    } else {
        String::new()
    }
}
