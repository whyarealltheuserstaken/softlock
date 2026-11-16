//! Close known shell surfaces. Ordinary windows are left alone.

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
};
use windows::core::PWSTR;

use crate::shell::should_close;

pub fn close_matching(extra_exe: Option<&str>) -> usize {
    let mut hwnds = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(enum_windows),
            LPARAM(&mut hwnds as *mut Vec<isize> as isize),
        );
    }
    let mut closed = 0;
    for bits in hwnds {
        let hwnd = HWND(bits as *mut core::ffi::c_void);
        let class = class_name(hwnd);
        let title = window_title(hwnd);
        let Some(exe) = process_path(hwnd) else {
            continue;
        };
        if should_close(&class, &title, &exe, extra_exe).is_none() {
            continue;
        }
        if unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok() } {
            closed += 1;
        }
    }
    closed
}

unsafe extern "system" fn enum_windows(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
    unsafe {
        let list = &mut *(data.0 as *mut Vec<isize>);
        list.push(hwnd.0 as isize);
        windows::Win32::Foundation::TRUE
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
