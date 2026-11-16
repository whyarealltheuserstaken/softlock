//! Topmost overlay and the small probe window used by the smoke run.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DT_NOPREFIX, DT_WORDBREAK, DeleteObject, DrawTextW, EndPaint,
    FillRect, HBRUSH, HGDIOBJ, PAINTSTRUCT, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_USERDATA, GetForegroundWindow,
    GetSystemMetrics, GetWindowLongPtrW, HICON, HWND_TOPMOST, LWA_ALPHA, MSG, PM_REMOVE,
    PeekMessageW, RegisterClassW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SetForegroundWindow, SetLayeredWindowAttributes, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY, WM_KEYDOWN, WM_PAINT,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    WS_OVERLAPPEDWINDOW, WS_POPUP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

pub const TEXT_BLOCKED: &str = "\
Keyboard and mouse are blocked.\n\
\n\
Escape: hold Ctrl, Alt, and Shift, then press Q.\n\
That works even if Windows Hello does not.\n\
Ctrl+Alt+Del still works.\n\
This closes by itself within 3 minutes.\n\
\n\
When the Hello window is in front, you can type a PIN.\n\
Win, Alt+Tab, and Ctrl+Shift+Esc do nothing.\n\
The mouse does nothing.";

pub const TEXT_PASS: &str = "\
The Hello window should be in front of this dark screen.\n\
Type the PIN here. The mouse does nothing.\n\
Win, Alt+Tab, and Ctrl+Shift+Esc do nothing.\n\
\n\
After one minute, PIN keys stop working too.\n\
Escape: hold Ctrl, Alt, and Shift, then press Q.";

pub const TEXT_TIMEOUT: &str = "\
One minute is up. Passthrough is off.\n\
PIN keys should do nothing now, even if the Hello window is still there.\n\
\n\
Escape: hold Ctrl, Alt, and Shift, then press Q.\n\
This closes by itself within 3 minutes.";

pub const TEXT_OBSERVE: &str = "\
The dialog closed. Passthrough is off.\n\
Type a key. It should not show up anywhere.\n\
This window closes in a few seconds.\n\
\n\
Escape: hold Ctrl, Alt, and Shift, then press Q.";

static HWND_SLOT: AtomicIsize = AtomicIsize::new(0);
static PROBE_SAW: AtomicBool = AtomicBool::new(false);

struct UiText {
    text: std::cell::UnsafeCell<Vec<u16>>,
}

// SAFETY: only the thread that created the window reads or writes this text.
unsafe impl Sync for UiText {}

static UI_TEXT: UiText = UiText {
    text: std::cell::UnsafeCell::new(Vec::new()),
};

pub fn create_overlay() -> windows::core::Result<HWND> {
    let hwnd = create_window(
        w!("HelloSpikeOverlay"),
        w!("Hello consent spike"),
        WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        WS_POPUP | WS_VISIBLE,
        virtual_origin(),
    )?;
    set_text(TEXT_BLOCKED);
    // SAFETY: the window is a layered window we just created.
    unsafe {
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 210, LWA_ALPHA)?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    raise_topmost(hwnd);
    Ok(hwnd)
}

pub fn create_probe() -> windows::core::Result<HWND> {
    create_window(
        w!("HelloSpikeProbe"),
        w!("hello-spike probe"),
        WINDOW_EX_STYLE(0),
        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        (200, 200, 480, 240),
    )
}

pub fn focus(hwnd: HWND) {
    // SAFETY: hwnd is a window this process created.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

pub fn is_foreground(hwnd: HWND) -> bool {
    // SAFETY: reads the desktop foreground window.
    unsafe { GetForegroundWindow() == hwnd }
}

pub fn raise_topmost(hwnd: HWND) {
    // SAFETY: hwnd is the overlay. SWP_NOACTIVATE keeps it from taking focus.
    unsafe {
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
}

pub fn set_text(text: &str) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    // SAFETY: only the window thread calls set_text and paint.
    unsafe {
        *UI_TEXT.text.get() = wide;
    }
    let raw = HWND_SLOT.load(Ordering::SeqCst);
    if raw != 0 {
        let hwnd = HWND(raw as *mut std::ffi::c_void);
        // SAFETY: the handle belongs to this process.
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
        }
    }
}

pub fn pump() {
    // SAFETY: the message loop reads the thread queue of this thread.
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
    }
}

pub fn probe_saw_key() -> bool {
    PROBE_SAW.load(Ordering::SeqCst)
}

pub fn send_key(vk: u16, down: bool) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if down {
                    windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(0)
                } else {
                    KEYEVENTF_KEYUP
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // SAFETY: SendInput reads one keyboard input we just filled in.
    unsafe {
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn create_window(
    class_name: PCWSTR,
    title: PCWSTR,
    ex_style: WINDOW_EX_STYLE,
    style: WINDOW_STYLE,
    place: (i32, i32, i32, i32),
) -> windows::core::Result<HWND> {
    // SAFETY: registering a private window class for this process.
    // The brush is stored on the window and deleted in WM_DESTROY.
    unsafe {
        let module = GetModuleHandleW(PCWSTR::null())?;
        let instance = windows::Win32::Foundation::HINSTANCE(module.0);
        let brush = CreateSolidBrush(COLORREF(0x00301814));
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            hIcon: HICON::default(),
            hCursor: windows::Win32::UI::WindowsAndMessaging::HCURSOR::default(),
            hbrBackground: HBRUSH::default(),
            lpszClassName: class_name,
            ..WNDCLASSW::default()
        };
        RegisterClassW(&class);
        let hwnd = match CreateWindowExW(
            ex_style,
            class_name,
            title,
            style,
            place.0,
            place.1,
            place.2,
            place.3,
            None,
            None,
            Some(instance),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                let _ = DeleteObject(HGDIOBJ(brush.0));
                return Err(error);
            }
        };
        let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, brush.0 as isize);
        HWND_SLOT.store(hwnd.0 as isize, Ordering::SeqCst);
        Ok(hwnd)
    }
}

fn virtual_origin() -> (i32, i32, i32, i32) {
    // SAFETY: GetSystemMetrics reads process-global desktop metrics.
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_KEYDOWN && wparam.0 == 0x41 {
        PROBE_SAW.store(true, Ordering::SeqCst);
    }
    if msg == WM_PAINT {
        paint(hwnd);
        return LRESULT(0);
    }
    if msg == WM_DESTROY {
        // SAFETY: the brush was stored on this window and is deleted once.
        unsafe {
            let brush = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if brush != 0 {
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                let _ = DeleteObject(HGDIOBJ(brush as *mut _));
            }
        }
    }
    // SAFETY: finish only forwards this message to the default handler.
    unsafe { finish(hwnd, msg, wparam, lparam) }
}

fn paint(hwnd: HWND) {
    // SAFETY: BeginPaint/EndPaint bracket a WM_PAINT for this window.
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut paint);
        if dc.0.is_null() {
            return;
        }
        let mut rect = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);
        let brush = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if brush != 0 {
            let _ = FillRect(dc, &rect, HBRUSH(brush as *mut _));
        }
        let _ = SetBkMode(dc, TRANSPARENT);
        let _ = SetTextColor(dc, COLORREF(0x00FFFFFF));
        let text = &*UI_TEXT.text.get();
        if text.len() > 1 {
            let mut owned = text.clone();
            if owned.last() == Some(&0) {
                owned.pop();
            }
            rect.left += 48;
            rect.top += 48;
            rect.right -= 48;
            rect.bottom -= 48;
            let _ = DrawTextW(dc, &mut owned, &mut rect, DT_WORDBREAK | DT_NOPREFIX);
        }
        let _ = EndPaint(hwnd, &paint);
    }
}

unsafe extern "system" fn finish(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the arguments came from the window procedure.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
