//! One topmost, click-through, capture-excluded window per monitor.
//! Drawn with GDI so a layered window stays opaque. Direct2D hwnd targets
//! do not compose with `WS_EX_LAYERED`; the look ticket can revisit that.

use std::sync::OnceLock;

use softlock_core::PRODUCT_DISPLAY_NAME;
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DT_CENTER, DT_NOPREFIX, DT_WORDBREAK, DeleteObject, DrawTextW,
    EndPaint, EnumDisplayMonitors, FillRect, GetStockObject, HBRUSH, HDC, HGDIOBJ, HMONITOR,
    InvalidateRect, NULL_BRUSH, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, HWND_TOPMOST, LWA_ALPHA,
    MA_NOACTIVATE, RegisterClassExW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowPos, ShowWindow,
    WDA_EXCLUDEFROMCAPTURE, WINDOWPOS, WM_DESTROY, WM_DISPLAYCHANGE, WM_ERASEBKGND,
    WM_MOUSEACTIVATE, WM_PAINT, WM_WINDOWPOSCHANGED, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
};
use windows::core::{BOOL, PCWSTR, w};

use super::session::{OVERLAY_MARKER, Shared};

const CLASS_NAME: PCWSTR = w!("softlock_overlay");

pub(crate) fn create(shared: &Shared) -> Result<(), ()> {
    register_class()?;
    let monitors = monitor_rects();
    if monitors.is_empty() {
        return Err(());
    }
    let mut created = Vec::with_capacity(monitors.len());
    for rect in monitors {
        let hwnd = match create_one(rect) {
            Ok(hwnd) => hwnd,
            Err(()) => {
                destroy_handles(&created);
                return Err(());
            }
        };
        if unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) }.is_err() {
            destroy_handles(&created);
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(());
        }
        created.push(hwnd.0 as isize);
    }
    let count = created.len() as u32;
    *shared
        .windows
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = created;
    shared
        .monitors
        .store(count, std::sync::atomic::Ordering::Release);
    shared
        .capture_excluded
        .store(true, std::sync::atomic::Ordering::Release);
    shared
        .topmost
        .store(true, std::sync::atomic::Ordering::Release);
    Ok(())
}

pub(crate) fn destroy(shared: &Shared) {
    let handles = std::mem::take(
        &mut *shared
            .windows
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()),
    );
    destroy_handles(&handles);
    shared
        .monitors
        .store(0, std::sync::atomic::Ordering::Release);
    shared
        .capture_excluded
        .store(false, std::sync::atomic::Ordering::Release);
    shared
        .topmost
        .store(false, std::sync::atomic::Ordering::Release);
}

pub(crate) fn invalidate(shared: &Shared) {
    let windows = shared
        .windows
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    for hwnd in windows.iter().copied() {
        unsafe {
            let _ = InvalidateRect(Some(hwnd_from(hwnd)), None, true);
        }
    }
}

pub(crate) fn minute_now() -> u16 {
    unsafe { GetLocalTime().wMinute }
}

fn destroy_handles(handles: &[isize]) {
    for hwnd in handles {
        unsafe {
            let _ = DestroyWindow(hwnd_from(*hwnd));
        }
    }
}

fn create_one(rect: RECT) -> Result<HWND, ()> {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return Err(());
    }
    let mut title = utf16(PRODUCT_DISPLAY_NAME);
    title.push(0);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_VISIBLE,
            rect.left,
            rect.top,
            width,
            height,
            None,
            None,
            Some(instance()),
            None,
        )
        .map_err(|_| ())?
    };
    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            rect.left,
            rect.top,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = UpdateWindow(hwnd);
    }
    Ok(hwnd)
}

fn monitor_rects() -> Vec<RECT> {
    let mut rects = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(monitor_proc),
            LPARAM(&mut rects as *mut Vec<RECT> as isize),
        );
    }
    rects
}

unsafe extern "system" fn monitor_proc(
    _monitor: HMONITOR,
    _hdc: HDC,
    rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    unsafe {
        if let Some(rect) = rect.as_ref() {
            let list = &mut *(data.0 as *mut Vec<RECT>);
            list.push(*rect);
        }
    }
    BOOL(1)
}

fn register_class() -> Result<(), ()> {
    static CLASS: OnceLock<Result<(), ()>> = OnceLock::new();
    *CLASS.get_or_init(|| {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance(),
            hbrBackground: HBRUSH(unsafe { GetStockObject(NULL_BRUSH) }.0),
            lpszClassName: CLASS_NAME,
            ..WNDCLASSEXW::default()
        };
        let atom = unsafe { RegisterClassExW(&class) };
        if atom == 0 { Err(()) } else { Ok(()) }
    })
}

fn instance() -> HINSTANCE {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.unwrap_or_default();
    HINSTANCE(module.0)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => {
                fill(hwnd, HDC(wparam.0 as *mut _));
                LRESULT(1)
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                paint_screen(hwnd, dc);
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            WM_WINDOWPOSCHANGED => {
                let pos = &*(lparam.0 as *const WINDOWPOS);
                if pos.hwndInsertAfter != HWND_TOPMOST {
                    let _ = SetWindowPos(
                        hwnd,
                        Some(HWND_TOPMOST),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                }
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                if let Some(shared) = super::guard::session_ptr() {
                    shared
                        .display_changed
                        .store(true, std::sync::atomic::Ordering::Release);
                }
                LRESULT(0)
            }
            WM_DESTROY => LRESULT(0),
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

unsafe fn fill(hwnd: HWND, dc: HDC) {
    unsafe {
        let brush = CreateSolidBrush(marker_color());
        if brush.0.is_null() {
            return;
        }
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let _ = FillRect(dc, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

unsafe fn paint_screen(hwnd: HWND, dc: HDC) {
    unsafe {
        fill(hwnd, dc);
        let font = GetStockObject(windows::Win32::Graphics::Gdi::DEFAULT_GUI_FONT);
        let previous = SelectObject(dc, font);
        let _ = SetBkMode(dc, TRANSPARENT);
        let _ = SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let height = rect.bottom - rect.top;
        rect.top += height / 3;
        let mut text = caption();
        let _ = DrawTextW(
            dc,
            &mut text,
            &mut rect,
            DT_CENTER | DT_WORDBREAK | DT_NOPREFIX,
        );
        text.fill(0);
        let _ = SelectObject(dc, previous);
    }
}

/// Drawn when neither Windows lock call could be confirmed. Ctrl+Alt+Del is
/// the secure attention sequence; the hook does not swallow it.
pub(crate) const OS_LOCK_NOTICE: &str =
    "Windows lock did not engage. Enter PIN. Ctrl+Alt+Del still reaches Windows.";

fn caption() -> Vec<u16> {
    let shared = super::guard::session_ptr();
    let prompt = shared
        .map(|shared| {
            shared
                .prompt_open
                .load(std::sync::atomic::Ordering::Acquire)
        })
        .unwrap_or(false);
    let chars = shared
        .map(|shared| shared.pin_chars.load(std::sync::atomic::Ordering::Acquire))
        .unwrap_or(0);
    let failed = shared
        .map(|shared| {
            shared
                .os_lock_error
                .load(std::sync::atomic::Ordering::Acquire)
        })
        .unwrap_or(false);
    let time = unsafe { GetLocalTime() };
    let clock = format!("{:02}:{:02}", time.wHour, time.wMinute);
    let notice = if failed {
        format!("\n{OS_LOCK_NOTICE}")
    } else {
        String::new()
    };
    let text = if prompt {
        let dots = "\u{25cf}".repeat(chars as usize);
        format!("{PRODUCT_DISPLAY_NAME}\n{clock}\nEnter PIN\n{dots}{notice}")
    } else {
        format!("{PRODUCT_DISPLAY_NAME}\n{clock}\nPress any key to unlock{notice}")
    };
    utf16(&text)
}

fn marker_color() -> COLORREF {
    COLORREF(
        u32::from(OVERLAY_MARKER.2) << 16
            | u32::from(OVERLAY_MARKER.1) << 8
            | u32::from(OVERLAY_MARKER.0),
    )
}

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn hwnd_from(value: isize) -> HWND {
    HWND(value as *mut std::ffi::c_void)
}
