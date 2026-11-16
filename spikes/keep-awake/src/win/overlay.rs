//! Dim or black overlay. It does not take focus and it does not swallow input.
//! A key-down asks the run to stop, then the key is passed on.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};

use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
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
    UnregisterClassW, WH_KEYBOARD_LL, WINDOW_EX_STYLE, WINDOW_STYLE, WM_PAINT, WNDCLASSEXW,
};
use windows::core::{PCWSTR, w};

use softlock_core::PRODUCT_DISPLAY_NAME;

use crate::plan::{OverlayLook, key_message_stops_run, overlay_ex_style, overlay_style};
use crate::stop::StopReason;

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

pub fn request_stop(reason: StopReason) {
    let _ = STOP_REASON.compare_exchange(0, reason as usize, Ordering::SeqCst, Ordering::SeqCst);
    let raw = STOP_HWND.load(Ordering::SeqCst);
    if raw == 0 {
        return;
    }
    let hwnd = HWND(raw as *mut core::ffi::c_void);
    // SAFETY: the handle was created on the UI thread and is cleared before it is destroyed.
    // PostMessageW is safe to call from the keyboard hook and the watchdog thread.
    let _ = unsafe { PostMessageW(Some(hwnd), STOP_MESSAGE, WPARAM(reason as usize), LPARAM(0)) };
}

pub fn stop_reason() -> StopReason {
    StopReason::from_code(STOP_REASON.load(Ordering::SeqCst) as u8)
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
    shown: bool,
}

impl Overlay {
    pub fn show(look: OverlayLook) -> Result<Self, String> {
        // SAFETY: a per-monitor context is optional. Failure means the process already has one.
        let _ =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let label = format!(
            "{PRODUCT_DISPLAY_NAME} keep-awake spike ({}). Press any key to stop.",
            look.label()
        );
        let _ = LABEL.set(label.encode_utf16().collect());
        // SAFETY: a null module name returns this process. The handle is not freed.
        let module = unsafe { GetModuleHandleW(None) }
            .map_err(|error| format!("Could not find this program: {error}"))?;
        let instance = HINSTANCE(module.0);
        // SAFETY: IDC_ARROW is a system cursor. A missing cursor still leaves a usable window.
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            hCursor: cursor,
            hbrBackground: HBRUSH(
                // SAFETY: BLACK_BRUSH is a stock object and must not be deleted.
                unsafe { GetStockObject(BLACK_BRUSH) }.0,
            ),
            lpszClassName: w!("KeepAwakeSpikeOverlay"),
            ..WNDCLASSEXW::default()
        };
        // SAFETY: the class lives for this process. The wide class name is a constant.
        let atom = unsafe { RegisterClassExW(&class) };
        let class_registered = if atom == 0 {
            // SAFETY: RegisterClassExW just failed, so GetLastError is its error.
            let error = unsafe { GetLastError() };
            if error != ERROR_CLASS_ALREADY_EXISTS {
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
        // SAFETY: the title buffer outlives this call. The styles are the click-through popup bits.
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
        // SAFETY: hwnd was just created as a layered popup. ShowWindow's return is the old state, not success.
        let shown = unsafe {
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
            !hwnd.0.is_null()
        };
        STOP_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        // SAFETY: a low-level keyboard hook on this thread, with no module, sees keys without swallowing them.
        let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0) }
            .map_err(|error| {
                // SAFETY: the window was created above and is not yet owned by Overlay.
                let _ = unsafe { DestroyWindow(hwnd) };
                STOP_HWND.store(0, Ordering::SeqCst);
                format!("Could not listen for a key. The overlay was closed. {error}")
            })?;
        Ok(Self {
            hwnd,
            hook,
            class_registered,
            instance,
            shown,
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        STOP_HWND.store(0, Ordering::SeqCst);
        // SAFETY: the hook and window were created in show, and the class atom is unregistered only if we registered it.
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
    // SAFETY: the virtual-screen metrics are plain integers. A zero size falls back to the primary screen.
    let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if width > 0 && height > 0 {
        return Ok((x, y, width, height));
    }
    // SAFETY: the primary-screen metrics are plain integers, used only when the virtual screen reports no size.
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
        request_stop(StopReason::Key);
    }
    // Always pass the key through. Stopping the run is separate from swallowing input.
    // SAFETY: CallNextHookEx with a null hook uses the current hook chain.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_PAINT {
        // SAFETY: paint pairs BeginPaint and EndPaint on this window.
        unsafe { paint(hwnd) };
        return LRESULT(0);
    }
    if msg == STOP_MESSAGE {
        // SAFETY: posting quit ends the message loop on this thread.
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    // SAFETY: every other message uses the default window procedure.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

unsafe fn paint(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: BeginPaint is paired with EndPaint below, including when the device context is null.
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    if !hdc.0.is_null()
        && let Some(text) = LABEL.get()
    {
        let mut rect = windows::Win32::Foundation::RECT::default();
        // SAFETY: the rectangle belongs to this window.
        let _ = unsafe { GetClientRect(hwnd, &mut rect) };
        rect.left += 32;
        rect.top += 32;
        // SAFETY: hdc is the paint device context and the text buffer lives for this call.
        unsafe {
            let _ = SetBkMode(hdc, TRANSPARENT);
            let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FF_FFFF));
            let mut chars = text.clone();
            let _ = DrawTextW(hdc, &mut chars[..], &mut rect, DT_LEFT);
        }
    }
    // SAFETY: EndPaint matches the BeginPaint above.
    let _ = unsafe { EndPaint(hwnd, &paint) };
}
