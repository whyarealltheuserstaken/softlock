//! Dim or black overlay. It does not take focus and it does not swallow input.
//! A key-down asks the run to stop, then the key is passed on.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};

use windows::Win32::Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BLACK_BRUSH, BeginPaint, DT_LEFT, DrawTextW, EndPaint, GetStockObject, HBRUSH, PAINTSTRUCT,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
    GetSystemMetrics, HHOOK, IDC_ARROW, LWA_ALPHA, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetWindowPos, SetWindowsHookExW, ShowWindow, UnhookWindowsHookEx,
    UnregisterClassW, WH_KEYBOARD_LL, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSEXW,
};
use windows::core::{PCWSTR, w};

use softlock_core::PRODUCT_DISPLAY_NAME;

use crate::plan::{OverlayLook, key_message_stops_run, overlay_ex_style, overlay_style};

pub const STOP_KEY: usize = 1;
pub const STOP_CTRL: usize = 2;
pub const STOP_TIMER: usize = 3;
pub const STOP_MESSAGE: u32 = 0x8000 + 1;

static STOP_HWND: AtomicIsize = AtomicIsize::new(0);
static STOP_REASON: AtomicUsize = AtomicUsize::new(0);
static STOP_SEEN: AtomicBool = AtomicBool::new(false);
static LABEL: OnceLock<Vec<u16>> = OnceLock::new();

pub fn reset_stop_state() {
    STOP_HWND.store(0, Ordering::SeqCst);
    STOP_REASON.store(0, Ordering::SeqCst);
    STOP_SEEN.store(false, Ordering::SeqCst);
}

pub fn request_stop(reason: usize) {
    let _ = STOP_REASON.compare_exchange(0, reason, Ordering::SeqCst, Ordering::SeqCst);
    let raw = STOP_HWND.load(Ordering::SeqCst);
    if raw == 0 {
        return;
    }
    let hwnd = HWND(raw as *mut core::ffi::c_void);
    // SAFETY: the handle was created on the UI thread and is cleared before it is destroyed.
    let _ = unsafe { PostMessageW(Some(hwnd), STOP_MESSAGE, WPARAM(reason), LPARAM(0)) };
}

pub fn stop_reason() -> usize {
    STOP_REASON.load(Ordering::SeqCst)
}

pub fn mark_loop_finished() {
    STOP_SEEN.store(true, Ordering::SeqCst);
}

pub fn loop_finished() -> bool {
    STOP_SEEN.load(Ordering::SeqCst)
}

pub struct Overlay {
    hwnd: HWND,
    hook: HHOOK,
    class_registered: bool,
    instance: HINSTANCE,
}

impl Overlay {
    pub fn show(look: OverlayLook) -> Result<Self, String> {
        // Per-monitor DPI makes the overlay cover the real panel. Failure is ignored
        // because the process may already have a context.
        let _ =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let label = format!(
            "{PRODUCT_DISPLAY_NAME} keep-awake spike ({}). Press any key to stop.",
            look.label()
        );
        let _ = LABEL.set(label.encode_utf16().collect());
        let module = unsafe { GetModuleHandleW(None) }
            .map_err(|error| format!("Could not find this program: {error}"))?;
        let instance = HINSTANCE(module.0);
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            hCursor: cursor,
            hbrBackground: HBRUSH(unsafe { GetStockObject(BLACK_BRUSH) }.0),
            lpszClassName: w!("KeepAwakeSpikeOverlay"),
            ..WNDCLASSEXW::default()
        };
        let atom = unsafe { RegisterClassExW(&class) };
        let class_registered = if atom == 0 {
            let error = unsafe { GetLastError() };
            if error.0 != 1410 {
                return Err(format!(
                    "Could not register the overlay window ({}).",
                    error.0
                ));
            }
            false
        } else {
            true
        };
        let bounds = virtual_screen()?;
        let title = wide(&label);
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(overlay_ex_style()),
                w!("KeepAwakeSpikeOverlay"),
                PCWSTR(title.as_ptr()),
                WINDOW_STYLE(overlay_style()),
                bounds.0,
                bounds.1,
                bounds.2,
                bounds.3,
                None,
                None,
                Some(instance),
                None,
            )
        }
        .map_err(|error| format!("Could not open the overlay window: {error}"))?;
        unsafe {
            let _ = SetLayeredWindowAttributes(
                hwnd,
                windows::Win32::Foundation::COLORREF(0),
                look.alpha(),
                LWA_ALPHA,
            );
            let _ = SetWindowPos(
                hwnd,
                Some(windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST),
                bounds.0,
                bounds.1,
                bounds.2,
                bounds.3,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        STOP_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0) }
            .map_err(|error| {
                let _ = unsafe { DestroyWindow(hwnd) };
                STOP_HWND.store(0, Ordering::SeqCst);
                format!("Could not listen for a key. The overlay was closed. {error}")
            })?;
        Ok(Self {
            hwnd,
            hook,
            class_registered,
            instance,
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        STOP_HWND.store(0, Ordering::SeqCst);
        unsafe {
            let _ = UnhookWindowsHookEx(self.hook);
            let _ = DestroyWindow(self.hwnd);
            if self.class_registered {
                let _ = UnregisterClassW(w!("KeepAwakeSpikeOverlay"), Some(self.instance));
            }
        }
    }
}

fn virtual_screen() -> Result<(i32, i32, i32, i32), String> {
    let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if width > 0 && height > 0 {
        return Ok((x, y, width, height));
    }
    let width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    if width > 0 && height > 0 {
        return Ok((0, 0, width, height));
    }
    Err("Windows did not report a screen size, so the overlay was not shown.".to_string())
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn size_of<T>() -> usize {
    std::mem::size_of::<T>()
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && key_message_stops_run(wparam.0 as u32) {
        request_stop(STOP_KEY);
    }
    // Always pass the key through. Stopping the run is separate from swallowing input.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == 0x000F {
        unsafe { paint(hwnd) };
        return LRESULT(0);
    }
    if msg == STOP_MESSAGE {
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

unsafe fn paint(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    if !hdc.0.is_null()
        && let Some(text) = LABEL.get()
    {
        let mut rect = windows::Win32::Foundation::RECT::default();
        let _ = unsafe { GetClientRect(hwnd, &mut rect) };
        rect.left += 32;
        rect.top += 32;
        unsafe {
            let _ = SetBkMode(hdc, TRANSPARENT);
            let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FF_FFFF));
            let mut chars = text.clone();
            let _ = DrawTextW(hdc, &mut chars[..], &mut rect, DT_LEFT);
        }
    }
    let _ = unsafe { EndPaint(hwnd, &paint) };
}
