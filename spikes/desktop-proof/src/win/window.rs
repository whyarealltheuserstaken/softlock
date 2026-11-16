use std::sync::OnceLock;

use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetStockObject, HBRUSH, HDC,
    HGDIOBJ, InvalidateRect, NULL_BRUSH, PAINTSTRUCT, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GW_HWNDPREV, GWL_EXSTYLE,
    GWLP_USERDATA, GetClassNameW, GetSystemMetrics, GetWindow, GetWindowLongPtrW, GetWindowRect,
    HWND_TOPMOST, RegisterClassExW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    WS_VISIBLE, WindowFromPoint,
};
use windows::core::PCWSTR;
use windows::core::w;

use crate::verdict::{Rgb, ScreenRect};

use super::util::{explain, last_error};

pub struct MarkerWindows {
    pub background: HWND,
    pub marker: HWND,
    pub marker_rect: ScreenRect,
    pub adjacent_rect: ScreenRect,
}

pub struct TopmostStatus {
    pub style: bool,
    pub hit: bool,
    pub adjacent_hit: bool,
    pub above: String,
}

pub fn per_monitor_dpi() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn show_markers() -> Result<MarkerWindows, String> {
    register_class()?;
    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    if screen_w < 200 || screen_h < 200 {
        return Err(format!("screen is {screen_w}x{screen_h}"));
    }
    let background_size = 320.min(screen_w - 32).min(screen_h - 32);
    let marker_size = 140.min(background_size / 2);
    if marker_size < 80 || background_size - marker_size < 80 {
        return Err(format!(
            "screen {screen_w}x{screen_h} is too small for a marker and an adjacent control region"
        ));
    }
    // Bottom-right, away from the runner console that usually occupies the top-left.
    let background_at = (
        screen_w - 16 - background_size,
        screen_h - 16 - background_size,
    );
    let background = create_window(
        background_at.0,
        background_at.1,
        background_size,
        background_size,
        crate::verdict::CONTROL,
        true,
    )?;
    let marker = create_window(
        background_at.0,
        background_at.1,
        marker_size,
        marker_size,
        crate::verdict::MARKER,
        true,
    )?;
    unsafe {
        let _ = SetWindowPos(
            marker,
            Some(HWND_TOPMOST),
            background_at.0,
            background_at.1,
            marker_size,
            marker_size,
            SWP_SHOWWINDOW,
        );
        let _ = DwmFlush();
    }
    let marker_rect = screen_rect(marker).unwrap_or(ScreenRect {
        x: background_at.0,
        y: background_at.1,
        width: marker_size,
        height: marker_size,
    });
    let background_rect = screen_rect(background).unwrap_or(ScreenRect {
        x: background_at.0,
        y: background_at.1,
        width: background_size,
        height: background_size,
    });
    let adjacent_rect = ScreenRect {
        x: marker_rect.x + marker_rect.width,
        y: marker_rect.y,
        width: (background_rect.x + background_rect.width) - (marker_rect.x + marker_rect.width),
        height: marker_rect.height,
    };
    Ok(MarkerWindows {
        background,
        marker,
        marker_rect,
        adjacent_rect,
    })
}

pub fn nudge(windows: &MarkerWindows) {
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(windows.marker, &mut rect).is_ok() {
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            let _ = SetWindowPos(
                windows.marker,
                Some(HWND_TOPMOST),
                rect.left + 1,
                rect.top,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = SetWindowPos(
                windows.marker,
                Some(HWND_TOPMOST),
                rect.left,
                rect.top,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        let _ = InvalidateRect(Some(windows.background), None, true);
        let _ = InvalidateRect(Some(windows.marker), None, true);
        let _ = UpdateWindow(windows.background);
        let _ = UpdateWindow(windows.marker);
        let _ = DwmFlush();
    }
}

pub fn set_excluded(windows: &MarkerWindows, excluded: bool) -> Result<(), String> {
    let affinity = if excluded {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    unsafe {
        SetWindowDisplayAffinity(windows.marker, affinity)
            .map_err(|error| explain(error, "SetWindowDisplayAffinity"))
    }
}

pub fn topmost_status(windows: &MarkerWindows) -> TopmostStatus {
    let style = unsafe { GetWindowLongPtrW(windows.marker, GWL_EXSTYLE) };
    let style_topmost = (style as u32) & WS_EX_TOPMOST.0 != 0;
    let hit = point_hits(windows.marker_rect, windows.marker);
    let adjacent_hit = point_hits(windows.adjacent_rect, windows.background);
    let above = window_above(windows.marker);
    TopmostStatus {
        style: style_topmost,
        hit,
        adjacent_hit,
        above,
    }
}

fn point_hits(rect: ScreenRect, expected: HWND) -> bool {
    if rect.width <= 0 || rect.height <= 0 {
        return false;
    }
    let point = POINT {
        x: rect.x + rect.width / 2,
        y: rect.y + rect.height / 2,
    };
    let hit = unsafe { WindowFromPoint(point) };
    hit == expected
}

fn window_above(hwnd: HWND) -> String {
    match unsafe { GetWindow(hwnd, GW_HWNDPREV) } {
        Ok(above) if above != hwnd && !above.0.is_null() => class_name(above),
        _ => "none".to_string(),
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let copied = unsafe { GetClassNameW(hwnd, &mut buf) };
    if copied <= 0 {
        return "unknown".to_string();
    }
    let end = copied as usize;
    String::from_utf16_lossy(&buf[..end.min(buf.len())])
}

fn screen_rect(hwnd: HWND) -> Option<ScreenRect> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect).ok()? };
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(ScreenRect {
        x: rect.left,
        y: rect.top,
        width,
        height,
    })
}

pub fn destroy(windows: &MarkerWindows) {
    unsafe {
        let _ = DestroyWindow(windows.background);
        let _ = DestroyWindow(windows.marker);
    }
}

pub fn pump(milliseconds: u64) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_millis(milliseconds) {
        unsafe {
            let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
            while windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
                &mut message,
                None,
                0,
                0,
                windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
            )
            .as_bool()
            {
                let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&message);
                let _ = windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&message);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn create_window(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: Rgb,
    topmost: bool,
) -> Result<HWND, String> {
    let ex_style = if topmost {
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW
    } else {
        WS_EX_TOOLWINDOW
    };
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            w!("RunnerDesktopProbe"),
            w!("runner-desktop-probe"),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(instance()),
            Some(std::ptr::addr_of!(color).cast()),
        )
        .map_err(|error| explain(error, "CreateWindowExW"))?
    };
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, colorref(color) as isize);
        let _ = ShowWindow(hwnd, SW_SHOW);
        if topmost {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                width,
                height,
                SWP_SHOWWINDOW,
            );
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = UpdateWindow(hwnd);
    }
    Ok(hwnd)
}

fn register_class() -> Result<(), String> {
    static CLASS: OnceLock<Result<(), String>> = OnceLock::new();
    CLASS
        .get_or_init(|| {
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance(),
                hbrBackground: HBRUSH(unsafe { GetStockObject(NULL_BRUSH) }.0),
                lpszClassName: w!("RunnerDesktopProbe"),
                ..WNDCLASSEXW::default()
            };
            let atom = unsafe { RegisterClassExW(&class) };
            if atom == 0 {
                Err(last_error("RegisterClassExW"))
            } else {
                Ok(())
            }
        })
        .clone()
}

fn instance() -> HINSTANCE {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.unwrap_or_default();
    HINSTANCE(module.0)
}

fn colorref(color: Rgb) -> u32 {
    u32::from(color.b) << 16 | u32::from(color.g) << 8 | u32::from(color.r)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            windows::Win32::UI::WindowsAndMessaging::WM_NCCREATE => {
                let create = &*(lparam.0 as *const CREATESTRUCTW);
                if !create.lpCreateParams.is_null() {
                    let color = *create.lpCreateParams.cast::<Rgb>();
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, colorref(color) as isize);
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND => {
                paint(hwnd, HDC(wparam.0 as *mut _));
                LRESULT(1)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_PAINT => {
                let mut paint_struct = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint_struct);
                paint(hwnd, dc);
                let _ = EndPaint(hwnd, &paint_struct);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

unsafe fn paint(hwnd: HWND, dc: HDC) {
    unsafe {
        let color = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u32;
        let brush = CreateSolidBrush(COLORREF(color));
        if brush.0.is_null() {
            return;
        }
        let mut rect = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);
        let _ = FillRect(dc, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}
