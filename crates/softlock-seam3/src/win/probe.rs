//! Probe window. Records keys that reach it and whether it is the foreground window.

use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetStockObject, HBRUSH, HDC,
    HGDIOBJ, InvalidateRect, NULL_BRUSH, PAINTSTRUCT, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK,
    MOUSEINPUT, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, BringWindowToTop, CreateWindowExW, DefWindowProcW,
    DestroyWindow, DispatchMessageW, GUITHREADINFO, GetClientRect, GetCursorPos,
    GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetSystemMetrics, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId, HWND_TOPMOST, MSG, RegisterClassExW, SM_CXSCREEN,
    SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SPI_GETFOREGROUNDLOCKTIMEOUT, SPI_SETFOREGROUNDLOCKTIMEOUT, SW_SHOW, SWP_SHOWWINDOW,
    SetForegroundWindow, SetWindowPos, ShowWindow, SystemParametersInfoW, TranslateMessage,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_PAINT, WM_SYSKEYDOWN, WNDCLASSEXW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::pixels::{MARKER, Rgb};

use super::error::{HarnessError, explain, last_error};

const CLASS_NAME: PCWSTR = w!("seam3_probe");
const WINDOW_TITLE: PCWSTR = w!("seam3 probe");
const MARKER_SIZE: i32 = 180;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedInput {
    pub virtual_key: u16,
    pub reached_probe: bool,
    pub probe_was_foreground: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusState {
    pub probe_is_foreground: bool,
    pub foreground_title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordedClick {
    pub message: u32,
}

/// Foreground window and the window that owns keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyboardFocus {
    pub foreground: isize,
    pub focus: isize,
}

struct Shared {
    inputs: Vec<RecordedInput>,
    clicks: Vec<RecordedClick>,
}

pub struct Probe {
    hwnd: isize,
    thread_id: u32,
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
    previous_lock_timeout: Option<u32>,
}

impl Probe {
    pub fn open() -> Result<Self, HarnessError> {
        per_monitor_dpi();
        let previous_lock_timeout = relax_foreground_lock();
        let shared = Arc::new(Mutex::new(Shared {
            inputs: Vec::new(),
            clicks: Vec::new(),
        }));
        let (tx, rx) = std::sync::mpsc::channel();
        let state = Arc::clone(&shared);
        let thread = thread::spawn(move || window_thread(state, tx));
        let ready = rx
            .recv_timeout(Duration::from_secs(8))
            .map_err(|_| HarnessError::new("probe window did not start"))?;
        let (hwnd, thread_id) = ready?;
        Ok(Self {
            hwnd,
            thread_id,
            shared,
            thread: Some(thread),
            previous_lock_timeout,
        })
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    fn window(&self) -> HWND {
        HWND(self.hwnd as *mut std::ffi::c_void)
    }

    pub fn inputs(&self) -> Vec<RecordedInput> {
        self.shared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .inputs
            .clone()
    }

    pub fn clicks(&self) -> Vec<RecordedClick> {
        self.shared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clicks
            .clone()
    }

    pub fn center(&self) -> (i32, i32) {
        let mut rect = RECT::default();
        unsafe {
            if GetWindowRect(self.window(), &mut rect).is_err() {
                return (0, 0);
            }
        }
        (
            rect.left + (rect.right - rect.left) / 2,
            rect.top + (rect.bottom - rect.top) / 2,
        )
    }

    pub fn wait_for_click(&self, timeout: Duration) -> Result<usize, HarnessError> {
        let started = self.clicks().len();
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let count = self.clicks().len();
            if count > started {
                return Ok(count);
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err(HarnessError::new(format!(
            "probe did not receive a click; focus={:?}; clicks={:?}",
            self.focus(),
            self.clicks()
        )))
    }

    pub fn focus(&self) -> FocusState {
        focus_of(self.window())
    }

    pub fn claim_focus(&self) {
        claim_foreground(self.window());
    }

    pub fn repaint(&self) {
        nudge_window(self.hwnd);
    }

    pub fn wait_for_key(
        &self,
        virtual_key: u16,
        timeout: Duration,
    ) -> Result<RecordedInput, HarnessError> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(input) = self
                .inputs()
                .into_iter()
                .find(|input| input.virtual_key == virtual_key)
            {
                return Ok(input);
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err(HarnessError::new(format!(
            "probe did not receive virtual key {virtual_key}; focus={:?}; inputs={:?}",
            self.focus(),
            self.inputs()
        )))
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                self.thread_id,
                windows::Win32::UI::WindowsAndMessaging::WM_QUIT,
                WPARAM(0),
                LPARAM(0),
            );
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some(timeout) = self.previous_lock_timeout {
            restore_foreground_lock(timeout);
        }
    }
}

pub fn cursor_position() -> Result<(i32, i32), HarnessError> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).map_err(|error| explain(error, "GetCursorPos"))? };
    Ok((point.x, point.y))
}

pub fn keyboard_focus() -> KeyboardFocus {
    let foreground = unsafe { GetForegroundWindow() };
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..GUITHREADINFO::default()
    };
    unsafe {
        let _ = GetGUIThreadInfo(0, &mut info);
    }
    KeyboardFocus {
        foreground: foreground.0 as isize,
        focus: info.hwndFocus.0 as isize,
    }
}

pub fn send_mouse_move(dx: i32, dy: i32, extra_info: u64) -> Result<(), HarnessError> {
    send_mouse(&[mouse_event(dx, dy, MOUSEEVENTF_MOVE, extra_info)])
}

pub fn send_mouse_absolute(x: i32, y: i32, extra_info: u64) -> Result<(), HarnessError> {
    let origin_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let origin_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) }.max(1);
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) }.max(1);
    let dx = (x.saturating_sub(origin_x)).clamp(0, width - 1) * 65535 / (width - 1).max(1);
    let dy = (y.saturating_sub(origin_y)).clamp(0, height - 1) * 65535 / (height - 1).max(1);
    send_mouse(&[mouse_event(
        dx,
        dy,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        extra_info,
    )])
}

pub fn send_left_click(extra_info: u64) -> Result<(), HarnessError> {
    send_mouse(&[
        mouse_event(0, 0, MOUSEEVENTF_LEFTDOWN, extra_info),
        mouse_event(0, 0, MOUSEEVENTF_LEFTUP, extra_info),
    ])
}

fn send_mouse(inputs: &[INPUT]) -> Result<(), HarnessError> {
    let inserted = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if inserted == 0 {
        Err(last_error("SendInput"))
    } else {
        Ok(())
    }
}

fn mouse_event(
    dx: i32,
    dy: i32,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
    extra_info: u64,
) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: extra_info as usize,
            },
        },
    }
}

pub fn send_virtual_key(virtual_key: u16, extra_info: u64) -> Result<(), HarnessError> {
    let inputs = [
        key_event(virtual_key, false, extra_info),
        key_event(virtual_key, true, extra_info),
    ];
    let inserted = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if inserted == 0 {
        Err(last_error("SendInput"))
    } else {
        Ok(())
    }
}

fn window_thread(
    shared: Arc<Mutex<Shared>>,
    ready: std::sync::mpsc::Sender<Result<(isize, u32), HarnessError>>,
) {
    let outcome = (|| {
        register_class()?;
        let hwnd = create_probe()?;
        claim_foreground(hwnd);
        let thread_id = unsafe { GetCurrentThreadId() };
        install_state(hwnd, shared);
        Ok((hwnd.0 as isize, thread_id))
    })();
    let hwnd = match outcome {
        Ok((hwnd, thread_id)) => {
            let _ = ready.send(Ok((hwnd, thread_id)));
            HWND(hwnd as *mut std::ffi::c_void)
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut message = MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 == 0 || status.0 == -1 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            let _ = DispatchMessageW(&message);
        }
    }
    remove_state(hwnd);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn create_probe() -> Result<HWND, HarnessError> {
    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    if screen_w < MARKER_SIZE + 32 || screen_h < MARKER_SIZE + 32 {
        return Err(HarnessError::new(format!(
            "screen is {screen_w}x{screen_h}"
        )));
    }
    let x = screen_w - 16 - MARKER_SIZE;
    let y = screen_h - 16 - MARKER_SIZE;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            WINDOW_TITLE,
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            MARKER_SIZE,
            MARKER_SIZE,
            None,
            None,
            Some(instance()),
            None,
        )
        .map_err(|error| explain(error, "CreateWindowExW"))?
    };
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            MARKER_SIZE,
            MARKER_SIZE,
            SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = UpdateWindow(hwnd);
        let _ = DwmFlush();
    }
    Ok(hwnd)
}

/// Moves the window by a pixel and back so desktop duplication and
/// Windows.Graphics.Capture have a new frame to deliver.
pub fn nudge_window(hwnd: isize) {
    let window = HWND(hwnd as *mut std::ffi::c_void);
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(window, &mut rect).is_err() {
            return;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let _ = SetWindowPos(
            window,
            Some(HWND_TOPMOST),
            rect.left + 1,
            rect.top,
            width,
            height,
            SWP_SHOWWINDOW,
        );
        let _ = SetWindowPos(
            window,
            Some(HWND_TOPMOST),
            rect.left,
            rect.top,
            width,
            height,
            SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(window), None, true);
        let _ = UpdateWindow(window);
        let _ = DwmFlush();
    }
    thread::sleep(Duration::from_millis(30));
}

fn claim_foreground(hwnd: HWND) {
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        let foreground = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let current = GetCurrentThreadId();
        let attached = foreground_thread != 0
            && foreground_thread != current
            && AttachThreadInput(current, foreground_thread, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(current, foreground_thread, false);
        }
    }
}

fn focus_of(hwnd: HWND) -> FocusState {
    let foreground = unsafe { GetForegroundWindow() };
    let mut title = [0u16; 256];
    let copied = unsafe { GetWindowTextW(foreground, &mut title) };
    let end = if copied < 0 { 0 } else { copied as usize };
    FocusState {
        probe_is_foreground: foreground == hwnd,
        foreground_title: String::from_utf16_lossy(&title[..end.min(title.len())]),
    }
}

fn key_event(virtual_key: u16, up: bool, extra_info: u64) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(virtual_key),
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                time: 0,
                dwExtraInfo: extra_info as usize,
            },
        },
    }
}

fn install_state(hwnd: HWND, shared: Arc<Mutex<Shared>>) {
    let mut map = states().lock().unwrap_or_else(|poison| poison.into_inner());
    map.insert(hwnd.0 as isize, shared);
}

fn remove_state(hwnd: HWND) {
    let mut map = states().lock().unwrap_or_else(|poison| poison.into_inner());
    map.remove(&(hwnd.0 as isize));
}

fn states() -> &'static Mutex<std::collections::HashMap<isize, Arc<Mutex<Shared>>>> {
    static STATES: OnceLock<Mutex<std::collections::HashMap<isize, Arc<Mutex<Shared>>>>> =
        OnceLock::new();
    STATES.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn register_class() -> Result<(), HarnessError> {
    static CLASS: OnceLock<Result<(), String>> = OnceLock::new();
    let result = CLASS.get_or_init(|| {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance(),
            hbrBackground: HBRUSH(unsafe { GetStockObject(NULL_BRUSH) }.0),
            lpszClassName: CLASS_NAME,
            ..WNDCLASSEXW::default()
        };
        let atom = unsafe { RegisterClassExW(&class) };
        if atom == 0 {
            Err(last_error("RegisterClassExW").to_string())
        } else {
            Ok(())
        }
    });
    result.clone().map_err(HarnessError::new)
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
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                record_key(hwnd, wparam.0 as u16);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                record_click(hwnd, message);
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND => {
                paint(hwnd, HDC(wparam.0 as *mut _));
                LRESULT(1)
            }
            WM_PAINT => {
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

fn record_key(hwnd: HWND, virtual_key: u16) {
    let map = states().lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(shared) = map.get(&(hwnd.0 as isize)) else {
        return;
    };
    let foreground = focus_of(hwnd).probe_is_foreground;
    let mut state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
    state.inputs.push(RecordedInput {
        virtual_key,
        reached_probe: true,
        probe_was_foreground: foreground,
    });
}

fn record_click(hwnd: HWND, message: u32) {
    let map = states().lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(shared) = map.get(&(hwnd.0 as isize)) else {
        return;
    };
    let mut state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
    state.clicks.push(RecordedClick { message });
}

unsafe fn paint(hwnd: HWND, dc: HDC) {
    unsafe {
        let brush = CreateSolidBrush(colorref(MARKER));
        if brush.0.is_null() {
            return;
        }
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let _ = FillRect(dc, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

fn colorref(color: Rgb) -> COLORREF {
    COLORREF(u32::from(color.b) << 16 | u32::from(color.g) << 8 | u32::from(color.r))
}

fn per_monitor_dpi() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

fn relax_foreground_lock() -> Option<u32> {
    unsafe {
        let mut timeout = 0u32;
        if SystemParametersInfoW(
            SPI_GETFOREGROUNDLOCKTIMEOUT,
            0,
            Some(&mut timeout as *mut u32 as *mut _),
            Default::default(),
        )
        .is_err()
        {
            return None;
        }
        if SystemParametersInfoW(
            SPI_SETFOREGROUNDLOCKTIMEOUT,
            0,
            None,
            windows::Win32::UI::WindowsAndMessaging::SPIF_SENDCHANGE,
        )
        .is_err()
        {
            return None;
        }
        Some(timeout)
    }
}

fn restore_foreground_lock(timeout: u32) {
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_SETFOREGROUNDLOCKTIMEOUT,
            timeout,
            None,
            windows::Win32::UI::WindowsAndMessaging::SPIF_SENDCHANGE,
        );
    }
}
