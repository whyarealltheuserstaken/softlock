use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, HWND_MESSAGE, PM_NOREMOVE, PeekMessageW,
    RegisterClassExW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY, WNDCLASSEXW,
};
use windows::core::w;

use super::util::last_error;

const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_CONSOLE_DISCONNECT: u32 = 0x2;
const WTS_SESSION_LOCK: u32 = 0x7;
const NOTIFY_FOR_ALL_SESSIONS: u32 = 1;
const KIND_NONE: u8 = 0;
const KIND_LOCK: u8 = 1;
const KIND_DISCONNECT: u8 = 2;

static KIND: AtomicU8 = AtomicU8::new(KIND_NONE);
static READY: AtomicBool = AtomicBool::new(false);
static WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

pub struct Note {
    pub notified: bool,
    pub kind: &'static str,
}

pub fn arm() -> Result<(), String> {
    if READY.load(Ordering::Acquire) {
        return Ok(());
    }
    unsafe {
        let mut discard = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        let _ = PeekMessageW(&mut discard, None, 0, 0, PM_NOREMOVE);
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance(),
            lpszClassName: w!("OsLockProbeListener"),
            ..WNDCLASSEXW::default()
        };
        if RegisterClassExW(&class) == 0 && !already_registered() {
            return Err(last_error("RegisterClassExW"));
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("OsLockProbeListener"),
            w!("os-lock-probe"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance()),
            None,
        )
        .map_err(|error| super::util::explain(error, "CreateWindowExW"))?;
        WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_ALL_SESSIONS)
            .map_err(|error| super::util::explain(error, "WTSRegisterSessionNotification"))?;
        WINDOW.store(hwnd.0 as isize, Ordering::Release);
        READY.store(true, Ordering::Release);
    }
    Ok(())
}

pub fn reset() {
    KIND.store(KIND_NONE, Ordering::SeqCst);
}

pub fn snapshot() -> Note {
    match KIND.load(Ordering::SeqCst) {
        KIND_LOCK => Note {
            notified: true,
            kind: "lock",
        },
        KIND_DISCONNECT => Note {
            notified: true,
            kind: "console-disconnect",
        },
        _ => Note {
            notified: false,
            kind: "none",
        },
    }
}

pub fn pump() {
    unsafe {
        let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        while PeekMessageW(
            &mut message,
            None,
            0,
            0,
            windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
        )
        .as_bool()
        {
            let _ = TranslateMessage(&message);
            let _ = DispatchMessageW(&message);
        }
    }
}

fn already_registered() -> bool {
    // 1410 is ERROR_CLASS_ALREADY_EXISTS.
    unsafe { windows::Win32::Foundation::GetLastError().0 == 1410 }
}

fn instance() -> HINSTANCE {
    let module = unsafe { GetModuleHandleW(None).unwrap_or_default() };
    HINSTANCE(module.0)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_WTSSESSION_CHANGE {
            let kind = wparam.0 as u32;
            if kind == WTS_SESSION_LOCK {
                let _ =
                    KIND.compare_exchange(KIND_NONE, KIND_LOCK, Ordering::SeqCst, Ordering::SeqCst);
            } else if kind == WTS_CONSOLE_DISCONNECT {
                let _ = KIND.compare_exchange(
                    KIND_NONE,
                    KIND_DISCONNECT,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
            }
            let _ = lparam;
        }
        if message == WM_DESTROY {
            let _ = WTSUnRegisterSessionNotification(hwnd);
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}
