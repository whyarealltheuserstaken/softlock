use std::sync::OnceLock;

use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
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
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_EXSTYLE, GWLP_USERDATA,
    GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, RegisterClassExW,
    SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowDisplayAffinity,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WNDCLASSEXW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::PCWSTR;
use windows::core::w;

use crate::verdict::Rgb;

use super::util::{explain, last_error};

pub struct MarkerWindows {
    pub background: HWND,
    pub marker: HWND,
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
    let background_size = 360.min(screen_w - 16).min(screen_h - 16);
    let marker_size = 180.min(background_size - 80);
    let background_at = (8, 8);
    let marker_at = (background_at.0 + 40, background_at.1 + 40);
    let background = create_window(
        background_at.0,
        background_at.1,
        background_size,
        background_size,
        crate::verdict::CONTROL,
        false,
    )?;
    let marker = create_window(
        marker_at.0,
        marker_at.1,
        marker_size,
        marker_size,
        crate::verdict::MARKER,
        true,
    )?;
    unsafe {
        let _ = DwmFlush();
    }
    Ok(MarkerWindows { background, marker })
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

pub fn is_topmost(windows: &MarkerWindows) -> bool {
    let style = unsafe { GetWindowLongPtrW(windows.marker, GWL_EXSTYLE) };
    (style as u32) & WS_EX_TOPMOST.0 != 0
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
            w!("P7DesktopProof"),
            w!("p7"),
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
                lpszClassName: w!("P7DesktopProof"),
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
