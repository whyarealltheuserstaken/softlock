use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::StationsAndDesktops::{
    GetProcessWindowStation, GetThreadDesktop, GetUserObjectInformationW, HDESK, HWINSTA, UOI_NAME,
};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::System::WindowsProgramming::GetUserNameW;
use windows::core::w;

use super::util::{from_wide, last_error};

pub struct DesktopIdentity {
    pub session_id: u32,
    pub pid: u32,
    pub username: String,
    pub window_station: String,
    pub desktop: String,
    pub os_product: String,
    pub os_build: String,
    pub installation_type: String,
}

pub fn identity() -> DesktopIdentity {
    DesktopIdentity {
        session_id: current_session(),
        pid: unsafe { GetCurrentProcessId() },
        username: user_name(),
        window_station: object_name(window_station_handle(), "window-station"),
        desktop: object_name(desktop_handle(), "desktop"),
        os_product: registry_string(w!("ProductName")),
        os_build: registry_string(w!("CurrentBuild")),
        installation_type: registry_string(w!("InstallationType")),
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
    let ok = unsafe { GetUserNameW(Some(super::util::pwstr(&mut buf)), &mut len) };
    if ok.is_ok() {
        from_wide(&buf)
    } else {
        last_error("GetUserNameW")
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

fn object_name(handle: HANDLE, label: &str) -> String {
    if handle.0.is_null() {
        return format!("{label} unavailable");
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
        last_error(label)
    }
}

fn registry_string(value: windows::core::PCWSTR) -> String {
    let mut buf = [0u16; 256];
    let mut bytes = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            value,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status == ERROR_SUCCESS {
        from_wide(&buf)
    } else {
        format!("registry {}", status.0)
    }
}
