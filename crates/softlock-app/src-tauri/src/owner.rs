//! Tray, global hotkey, and the public-pipe session. Windows only.
//!
//! Quit destroys this process. It does not talk to the Service Control Manager.

use std::fs::File;
use std::os::windows::io::FromRawHandle;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use softlock_core::{LockSource, RequestKind, ResponseKind, Secret};
use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_SHIFT, RegisterHotKey,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, HWND_MESSAGE, IDI_APPLICATION, LoadIconW,
    MF_GRAYED, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_COMMAND, WM_DESTROY, WM_HOTKEY,
    WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::PCWSTR;

use crate::actions::{self, autostart_command, autostart_value_name, service_line, status_line};
use crate::client::{self, OwnerView, exchange, public_pipe_name, status_text, view_from_status};

const WM_TRAY: u32 = 0x8000;
const WM_REFRESH: u32 = 0x8001;
const CMD_LOCK: usize = 10;
const CMD_QUIT: usize = 11;

static VIEW: OnceLock<Mutex<OwnerView>> = OnceLock::new();
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static QUIT: AtomicBool = AtomicBool::new(false);
static HOTKEY: Mutex<Option<&'static str>> = Mutex::new(None);
static APP: OnceLock<AppHandle> = OnceLock::new();

fn view_slot() -> &'static Mutex<OwnerView> {
    VIEW.get_or_init(|| Mutex::new(OwnerView::unknown()))
}

pub fn current_view() -> OwnerView {
    view_slot()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

pub fn run(app: Option<AppHandle>) -> i32 {
    if let Some(app) = app {
        let _ = APP.set(app);
    }
    if std::env::var("SOFTLOCK_NO_AUTOSTART").ok().as_deref() != Some("1") {
        register_autostart();
    }
    if std::env::var("SOFTLOCK_QUIT_AFTER_HELLO").ok().as_deref() == Some("1") {
        let view = match transact(RequestKind::GetStatus) {
            Ok(response) => view_from_status(&response),
            Err(_) => OwnerView::unknown(),
        };
        publish(view.clone());
        return if view.service_running { 0 } else { 1 };
    }
    let watch = thread::spawn(watch_states);
    let code = window_loop();
    QUIT.store(true, Ordering::Release);
    let _ = watch.join();
    if let Some(app) = APP.get() {
        app.exit(0);
    }
    code
}

pub fn request_quit() -> bool {
    let raw = WINDOW.load(Ordering::Acquire);
    if raw == 0 {
        return false;
    }
    let hwnd = HWND(raw as *mut core::ffi::c_void);
    unsafe { PostMessageW(Some(hwnd), WM_DESTROY, WPARAM(0), LPARAM(0)) }.is_ok()
}

pub fn send_lock(source: LockSource) {
    let outcome = transact(actions::lock_request(source));
    match outcome {
        Ok(ResponseKind::Accepted | ResponseKind::AlreadyLocked) => {
            if let Ok(status) = transact(RequestKind::GetStatus) {
                publish(view_from_status(&status));
                return;
            }
            publish(OwnerView {
                service_running: true,
                state: Some(softlock_core::StateName::Locked),
                lock_source: Some(source),
            });
        }
        Ok(response) => {
            if let Ok(status) = transact(RequestKind::GetStatus) {
                publish(view_from_status(&status));
            } else {
                let _ = response;
                publish(OwnerView::unknown());
            }
        }
        Err(_) => publish(OwnerView::unknown()),
    }
}

pub fn send_pin(pin: Secret) -> bool {
    matches!(transact(RequestKind::SetPin { pin }), Ok(ResponseKind::Ok))
}

fn publish(view: OwnerView) {
    {
        let mut slot = view_slot()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *slot = view.clone();
    }
    write_status(&view);
    let raw = WINDOW.load(Ordering::Acquire);
    if raw != 0 {
        let hwnd = HWND(raw as *mut core::ffi::c_void);
        let _ = unsafe { PostMessageW(Some(hwnd), WM_REFRESH, WPARAM(0), LPARAM(0)) };
    }
    if let Some(app) = APP.get() {
        let _ = app.emit(
            "owner-view",
            ViewEvent {
                service_running: view.service_running,
                locked: view.locked(),
                state: state_label(view.state),
                lock_source: view
                    .lock_source
                    .map(client::lock_source_name)
                    .map(str::to_string),
                status_line: status_line(view.service_running, view.state),
                service_line: service_line(view.service_running).to_string(),
            },
        );
    }
}

#[derive(Clone, serde::Serialize)]
struct ViewEvent {
    service_running: bool,
    locked: bool,
    state: String,
    lock_source: Option<String>,
    status_line: String,
    service_line: String,
}

fn state_label(state: Option<softlock_core::StateName>) -> String {
    state
        .map(|state| format!("{state:?}"))
        .unwrap_or_else(|| "Locked".to_string())
}

fn write_status(view: &OwnerView) {
    let Ok(path) = std::env::var("SOFTLOCK_STATUS_FILE") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let hotkey = HOTKEY
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .unwrap_or("pending");
    let mut text = status_text(view);
    text.push_str(&format!("hotkey={hotkey}\n"));
    let _ = std::fs::write(path, text);
}

fn watch_states() {
    while !QUIT.load(Ordering::Acquire) {
        let mut io = match connect_retry() {
            Ok(io) => io,
            Err(_) => {
                publish(OwnerView::unknown());
                thread::sleep(Duration::from_millis(300));
                continue;
            }
        };
        let id = client::next_id();
        if exchange(&mut io, id, &RequestKind::Subscribe).is_err() {
            publish(OwnerView::unknown());
            thread::sleep(Duration::from_millis(300));
            continue;
        }
        loop {
            if QUIT.load(Ordering::Acquire) {
                return;
            }
            let frame = match client::read_one(&mut io) {
                Ok(frame) => frame,
                Err(_) => {
                    publish(OwnerView::unknown());
                    break;
                }
            };
            let decoded = client::decode_response(&frame);
            let next = client::view_after(&current_view(), &decoded);
            if decoded.is_ok() {
                publish(next);
            } else {
                publish(OwnerView::unknown());
                break;
            }
        }
    }
}

fn window_loop() -> i32 {
    let class_name = wide("softlock-owner");
    let instance = match unsafe { GetModuleHandleW(None) } {
        Ok(instance) => instance,
        Err(_) => return 1,
    };
    let class = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return 1;
    }
    let hwnd = match unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            Default::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
    } {
        Ok(hwnd) => hwnd,
        Err(_) => return 1,
    };
    WINDOW.store(hwnd.0 as isize, Ordering::Release);
    let hotkey = actions::default_hotkey();
    let registered = unsafe {
        RegisterHotKey(
            Some(hwnd),
            1,
            MOD_CONTROL | MOD_ALT | MOD_SHIFT,
            u32::from(hotkey.virtual_key),
        )
    };
    {
        let mut slot = HOTKEY.lock().unwrap_or_else(|poison| poison.into_inner());
        *slot = Some(if registered.is_ok() { "ok" } else { "failed" });
    }
    add_tray(hwnd);
    publish(current_view());
    let mut message = MSG::default();
    loop {
        let pulled = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if pulled.0 == 0 || pulled.0 == -1 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    remove_tray(hwnd);
    WINDOW.store(0, Ordering::Release);
    0
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_HOTKEY => {
            send_lock(LockSource::Hotkey);
            LRESULT(0)
        }
        WM_TRAY => {
            let event = (lparam.0 as u32) & 0xffff;
            if event == WM_LBUTTONUP || event == WM_RBUTTONUP {
                show_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 & 0xffff {
                CMD_LOCK => send_lock(LockSource::Tray),
                CMD_QUIT => unsafe {
                    let _ = DestroyWindow(hwnd);
                },
                _ => {}
            }
            LRESULT(0)
        }
        WM_REFRESH => {
            update_tray(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            remove_tray(hwnd);
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn show_menu(hwnd: HWND) {
    let menu = match unsafe { CreatePopupMenu() } {
        Ok(menu) => menu,
        Err(_) => return,
    };
    let view = current_view();
    let status = wide(&status_line(view.service_running, view.state));
    let service = wide(service_line(view.service_running));
    let lock_now = wide("Lock now");
    let quit = wide("Quit");
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 1, PCWSTR(status.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 2, PCWSTR(service.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, CMD_LOCK, PCWSTR(lock_now.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, PCWSTR(quit.as_ptr()));
        let mut point = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, Some(0), hwnd, None);
        let _ = DestroyMenu(menu);
    }
}

fn add_tray(hwnd: HWND) {
    let data = tray_data(hwnd);
    let _ = unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
}

fn update_tray(hwnd: HWND) {
    let data = tray_data(hwnd);
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
}

fn remove_tray(hwnd: HWND) {
    let data = tray_data(hwnd);
    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() };
    let view = current_view();
    let tip = wide(&format!(
        "{} — {}",
        status_line(view.service_running, view.state),
        service_line(view.service_running)
    ));
    for (index, unit) in tip.iter().take(data.szTip.len()).enumerate() {
        data.szTip[index] = *unit;
    }
    data
}

fn register_autostart() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let name = wide(autostart_value_name());
    let command = wide(&autostart_command(&exe));
    let _ = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            REG_SZ.0,
            Some(command.as_ptr().cast()),
            (command.len() * 2) as u32,
        )
    };
}

fn transact(kind: RequestKind) -> Result<ResponseKind, String> {
    let mut io = connect_retry()?;
    exchange(&mut io, client::next_id(), &kind).map_err(|error| format!("{error:?}"))
}

fn connect_retry() -> Result<File, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match connect() {
            Ok(file) => return Ok(file),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn connect() -> Result<File, String> {
    let name = wide(&public_pipe_name());
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(|error| error.to_string())?
    };
    Ok(unsafe { File::from_raw_handle(handle.0) })
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
