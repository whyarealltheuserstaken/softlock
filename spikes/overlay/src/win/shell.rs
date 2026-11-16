//! Find shell surfaces and dismiss them with Escape. Never post a close message
//! to Explorer or a shell host.

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::PWSTR;

use crate::shell::{ShellSurface, identify, surface_is_open};

use super::input;

pub fn visible_surfaces() -> Vec<ShellSurface> {
    let mut found = Vec::new();
    for hwnd in top_level_windows() {
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        let cloaked = visible && is_cloaked(hwnd);
        if !surface_is_open(visible, cloaked) {
            continue;
        }
        let Some(exe) = process_path(hwnd) else {
            continue;
        };
        let Some(kind) = identify(&class_name(hwnd), &window_title(hwnd), &exe) else {
            continue;
        };
        if !found.contains(&kind) {
            found.push(kind);
        }
    }
    found
}

/// One Escape, and only when a real shell surface is open.
pub fn dismiss_visible() -> bool {
    if visible_surfaces().is_empty() {
        return false;
    }
    input::escape().is_ok()
}

fn top_level_windows() -> Vec<HWND> {
    let mut hwnds = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(enum_windows),
            LPARAM(&mut hwnds as *mut Vec<isize> as isize),
        );
    }
    hwnds
        .into_iter()
        .map(|bits| HWND(bits as *mut core::ffi::c_void))
        .collect()
}

unsafe extern "system" fn enum_windows(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
    unsafe {
        let list = &mut *(data.0 as *mut Vec<isize>);
        list.push(hwnd.0 as isize);
        windows::Win32::Foundation::TRUE
    }
}

fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut core::ffi::c_void,
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = vec![0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    from_wide(&buf, len)
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = vec![0u16; 256];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    from_wide(&buf, len)
}

fn process_path(hwnd: HWND) -> Option<String> {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()? };
    let mut buf = vec![0u16; 512];
    let mut len = buf.len() as u32;
    let read = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    unsafe {
        let _ = CloseHandle(handle);
    }
    read.ok()?;
    Some(from_wide(&buf, len as i32))
}

fn from_wide(buf: &[u16], len: i32) -> String {
    let len = len.max(0) as usize;
    let end = len.min(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
