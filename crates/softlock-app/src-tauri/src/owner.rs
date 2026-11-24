//! Tray, global hotkey, and the public-pipe session. Windows only.
//!
//! Quit destroys this process. It does not talk to the Service Control Manager.

use std::fs::File;
use std::os::windows::io::FromRawHandle;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use softlock_core::{LockSource, MAX_CONFIG_BYTES, RequestKind, ResponseKind, Secret};
use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::CancelSynchronousIo;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
use windows::Win32::System::Threading::{GetCurrentThreadId, OpenThread, THREAD_TERMINATE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, HWND_MESSAGE, IDI_APPLICATION, LoadIconW,
    MF_GRAYED, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_HOTKEY,
    WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::PCWSTR;

use crate::actions::{self, autostart_command, autostart_value_name, service_line, status_line};
use crate::client::{self, OwnerView, exchange, public_pipe_name, status_text, view_from_status};

const WM_TRAY: u32 = 0x8000;
const WM_REFRESH: u32 = 0x8001;
/// The settings command posts this. The window proc runs the same lock as the tray.
pub const WM_LOCK_APP: u32 = 0x8002;
pub const CMD_LOCK: usize = 10;
pub const CMD_QUIT: usize = 11;
pub const CMD_HARD_LOCK: usize = 12;

static VIEW: OnceLock<Mutex<OwnerView>> = OnceLock::new();
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static QUIT: AtomicBool = AtomicBool::new(false);
static HOTKEY_OK: AtomicBool = AtomicBool::new(true);
static HOTKEY_LABEL: Mutex<String> = Mutex::new(String::new());
static AUTOSTART_CODE: AtomicU32 = AtomicU32::new(0);
static WATCH_THREAD: AtomicU32 = AtomicU32::new(0);
static STATUS_SEQ: AtomicU64 = AtomicU64::new(0);
static APP: OnceLock<AppHandle> = OnceLock::new();
static LOCK_WAKE: OnceLock<Sender<TrayAction>> = OnceLock::new();
static FALLBACK_NOTICE: Mutex<String> = Mutex::new(String::new());
static CONFIG_ERRORS: Mutex<String> = Mutex::new(String::new());
static CONFIG_OFFER: AtomicBool = AtomicBool::new(false);
static CONFIG_HOTKEY: Mutex<String> = Mutex::new(String::new());
static PENDING_CONFIG: Mutex<String> = Mutex::new(String::new());

enum TrayAction {
    Lock(LockSource),
    HardLock,
}

fn owner_class_name() -> String {
    format!("{}-owner", softlock_core::SERVICE_ID)
}

fn ui_text(key: &str) -> String {
    let parsed: serde_json::Value = serde_json::from_str(include_str!("../../src/strings/ui.json"))
        .unwrap_or(serde_json::Value::Null);
    parsed
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or(key)
        .to_string()
}

fn skip_autostart() -> bool {
    #[cfg(any(feature = "simulated-physical", feature = "live-fail-closed"))]
    {
        if std::env::var("SOFTLOCK_NO_AUTOSTART").ok().as_deref() == Some("1")
            && autostart_override_allowed()
        {
            return true;
        }
    }
    false
}

#[cfg(feature = "simulated-physical")]
fn autostart_override_allowed() -> bool {
    true
}

#[cfg(all(feature = "live-fail-closed", not(feature = "simulated-physical")))]
fn autostart_override_allowed() -> bool {
    softlock_windows::runner_may_lock().is_ok()
}

fn status_file_path() -> Option<String> {
    #[cfg(feature = "simulated-physical")]
    {
        std::env::var("SOFTLOCK_STATUS_FILE")
            .ok()
            .filter(|value| !value.is_empty())
    }
    #[cfg(not(feature = "simulated-physical"))]
    {
        None
    }
}

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
    if !skip_autostart() {
        register_autostart();
    }
    start_lock_worker();
    thread::spawn(watch_states);
    thread::spawn(watch_config_file);
    let code = window_loop();
    // The watch thread blocks in ReadFile until the next StateChanged. Cancelling
    // that read lets the process leave without joining it.
    QUIT.store(true, Ordering::Release);
    cancel_watch_read();
    if let Some(sender) = LOCK_WAKE.get() {
        let _ = sender.send(TrayAction::Lock(LockSource::Hotkey));
    }
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
    unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.is_ok()
}

/// Same lock the settings button uses. The window proc only queues it.
pub fn lock_from_app() {
    enqueue_lock(LockSource::App);
}

static DECLINE_AT: AtomicU8 = AtomicU8::new(0);

/// Which line `decline_config` reached. The seam test reads this when the
/// call does not return.
pub fn decline_at() -> u8 {
    DECLINE_AT.load(Ordering::Relaxed)
}

pub fn decline_config() -> bool {
    // Same rule as accept: a locked session does not write the config file.
    // The service holds the DACL until the owner unlocks.
    DECLINE_AT.store(1, Ordering::Relaxed);
    if current_view().locked() {
        return false;
    }
    DECLINE_AT.store(2, Ordering::Relaxed);
    let effective = current_config();
    DECLINE_AT.store(3, Ordering::Relaxed);
    CONFIG_OFFER.store(false, Ordering::Release);
    *PENDING_CONFIG
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = String::new();
    DECLINE_AT.store(4, Ordering::Relaxed);
    if let Some(document) = effective
        && let Some(path) = profile_config_path()
        && !write_user_file(&path, &document)
    {
        return false;
    }
    DECLINE_AT.store(5, Ordering::Relaxed);
    publish_current();
    DECLINE_AT.store(6, Ordering::Relaxed);
    true
}

pub fn accept_config() -> bool {
    if current_view().locked() || !CONFIG_OFFER.load(Ordering::Acquire) {
        return false;
    }
    let pending = PENDING_CONFIG
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    if pending.is_empty() {
        return false;
    }
    let previous = current_config();
    let previous_hotkey = registered_hotkey();
    let Some(revision) = config_revision() else {
        return false;
    };
    match transact(RequestKind::PutConfig {
        toml: pending.clone(),
        base_revision: revision,
    }) {
        Ok(ResponseKind::Applied) => {
            if !keep_hotkey_registered(&previous_hotkey, &pending, previous) {
                publish_current();
                return false;
            }
            CONFIG_OFFER.store(false, Ordering::Release);
            *PENDING_CONFIG
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = String::new();
            *CONFIG_ERRORS
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = String::new();
            refresh_hotkey();
            publish_current();
            true
        }
        Ok(ResponseKind::Invalid { errors }) => {
            *CONFIG_ERRORS
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = errors.join("\n");
            false
        }
        _ => false,
    }
}

pub fn config_errors() -> String {
    CONFIG_ERRORS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

pub fn config_offer() -> bool {
    CONFIG_OFFER.load(Ordering::Acquire)
}

pub fn config_hotkey() -> String {
    let label = CONFIG_HOTKEY
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    if label.is_empty() {
        actions::hotkey_label("")
    } else {
        label
    }
}

fn enqueue_lock(source: LockSource) {
    enqueue(TrayAction::Lock(source));
}

fn enqueue_hard_lock() {
    enqueue(TrayAction::HardLock);
}

fn enqueue(action: TrayAction) {
    if let Some(sender) = LOCK_WAKE.get() {
        let _ = sender.send(action);
    } else {
        run_tray_action(action);
    }
}

fn start_lock_worker() {
    let (sender, receiver) = mpsc::channel::<TrayAction>();
    let _ = LOCK_WAKE.set(sender);
    thread::spawn(move || {
        while let Ok(action) = receiver.recv() {
            if QUIT.load(Ordering::Acquire) {
                break;
            }
            run_tray_action(action);
        }
    });
}

fn run_tray_action(action: TrayAction) {
    match action {
        TrayAction::Lock(source) => perform_lock(source),
        TrayAction::HardLock => perform_hard_lock(),
    }
}

fn perform_hard_lock() {
    let outcome = transact(actions::hard_lock_request());
    match outcome {
        Ok(ResponseKind::Accepted | ResponseKind::AlreadyLocked) => {
            if let Ok(status) = transact(RequestKind::GetStatus) {
                publish(&view_from_status(&status));
            }
        }
        Ok(_) => {
            if let Ok(status) = transact(RequestKind::GetStatus) {
                publish(&view_from_status(&status));
            } else {
                publish(&OwnerView::unknown());
            }
        }
        Err(_) => publish(&OwnerView::unknown()),
    }
}

pub fn lock_for_real() {
    enqueue_hard_lock();
}

fn perform_lock(source: LockSource) {
    let outcome = transact(actions::lock_request(source));
    match outcome {
        Ok(ResponseKind::Accepted | ResponseKind::AlreadyLocked) => {
            if let Ok(status) = transact(RequestKind::GetStatus) {
                publish(&view_from_status(&status));
                return;
            }
            publish(&OwnerView {
                service_running: true,
                state: Some(softlock_core::StateName::Locked),
                lock_source: Some(source),
            });
        }
        Ok(response) => {
            if let Ok(status) = transact(RequestKind::GetStatus) {
                publish(&view_from_status(&status));
            } else {
                let _ = response;
                publish(&OwnerView::unknown());
            }
        }
        Err(_) => publish(&OwnerView::unknown()),
    }
}

pub enum PinSend {
    Stored,
    NeedsConfirmation,
    Failed,
}

pub fn send_pin(pin: Secret) -> PinSend {
    match transact(RequestKind::SetPin { pin }) {
        Ok(ResponseKind::Ok) => PinSend::Stored,
        Ok(ResponseKind::NeedsConfirmation { .. }) => PinSend::NeedsConfirmation,
        _ => PinSend::Failed,
    }
}

pub fn current_notice() -> String {
    notice_text()
}

fn publish_current() {
    let view = current_view();
    publish(&view);
}

fn publish(view: &OwnerView) {
    {
        let mut slot = view_slot()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *slot = view.clone();
    }
    write_status(view);
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
                notice: notice_text(),
                config_errors: config_errors(),
                config_offer: config_offer(),
                config_hotkey: config_hotkey(),
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
    notice: String,
    config_errors: String,
    config_offer: bool,
    config_hotkey: String,
}

fn state_label(state: Option<softlock_core::StateName>) -> String {
    state
        .map(|state| format!("{state:?}"))
        .unwrap_or_else(|| "Locked".to_string())
}

pub fn set_fallback_notice(notice: String) {
    *FALLBACK_NOTICE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = notice;
}

fn registered_hotkey() -> actions::Hotkey {
    current_config()
        .and_then(|document| actions::hotkey_from_document(&document))
        .unwrap_or_else(actions::default_hotkey)
}

fn current_hotkey_label() -> Option<String> {
    current_config().map(|document| actions::hotkey_label(&document))
}

fn current_config() -> Option<String> {
    match transact(RequestKind::GetConfig) {
        Ok(ResponseKind::Config { toml, .. }) => Some(toml),
        _ => None,
    }
}

fn config_revision() -> Option<u64> {
    match transact(RequestKind::GetConfig) {
        Ok(ResponseKind::Config { revision, .. }) => Some(revision),
        _ => None,
    }
}

fn refresh_hotkey() {
    let label = current_config()
        .map(|document| actions::hotkey_label(&document))
        .unwrap_or_else(|| actions::hotkey_label(""));
    *CONFIG_HOTKEY
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = label;
}

/// After `PutConfig` returns Applied, register the new chord. A failure puts
/// the previous document back and leaves the old chord registered.
fn keep_hotkey_registered(
    previous_hotkey: &actions::Hotkey,
    applied: &str,
    previous_document: Option<String>,
) -> bool {
    let new_hotkey = actions::hotkey_from_document(applied).unwrap_or_else(actions::default_hotkey);
    if replace_registered_hotkey(previous_hotkey, &new_hotkey) {
        return true;
    }
    if let Some(document) = previous_document
        && let Some(revision) = config_revision()
    {
        let _ = transact(RequestKind::PutConfig {
            toml: document,
            base_revision: revision,
        });
    }
    *CONFIG_ERRORS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) =
        "the new hotkey could not be registered".to_string();
    false
}

fn replace_registered_hotkey(previous: &actions::Hotkey, next: &actions::Hotkey) -> bool {
    if same_hotkey(previous, next) {
        return true;
    }
    let raw = WINDOW.load(Ordering::Acquire);
    if raw == 0 {
        return true;
    }
    let hwnd = HWND(raw as *mut core::ffi::c_void);
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), 1);
    }
    if register_hotkey(hwnd, next) {
        HOTKEY_OK.store(true, Ordering::Release);
        return true;
    }
    *CONFIG_ERRORS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) =
        "the new hotkey could not be registered".to_string();
    let restored = register_hotkey(hwnd, previous);
    HOTKEY_OK.store(
        restored || register_hotkey(hwnd, previous),
        Ordering::Release,
    );
    false
}

fn register_hotkey(hwnd: HWND, hotkey: &actions::Hotkey) -> bool {
    unsafe {
        RegisterHotKey(
            Some(hwnd),
            1,
            hotkey_modifiers(hotkey),
            u32::from(hotkey.virtual_key),
        )
        .is_ok()
    }
}

fn hotkey_modifiers(hotkey: &actions::Hotkey) -> HOT_KEY_MODIFIERS {
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    if hotkey.control {
        modifiers |= MOD_CONTROL;
    }
    if hotkey.alt {
        modifiers |= MOD_ALT;
    }
    if hotkey.shift {
        modifiers |= MOD_SHIFT;
    }
    modifiers
}

fn same_hotkey(left: &actions::Hotkey, right: &actions::Hotkey) -> bool {
    left.control == right.control
        && left.alt == right.alt
        && left.shift == right.shift
        && left.virtual_key == right.virtual_key
}

#[cfg(any(test, feature = "simulated-physical", feature = "live-fail-closed"))]
pub fn poll_config_once() {
    static SEEN: Mutex<(String, String)> = Mutex::new((String::new(), String::new()));
    let mut seen = SEEN.lock().unwrap_or_else(|poison| poison.into_inner());
    let path = profile_config_path()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    if seen.0 != path {
        seen.0 = path;
        seen.1.clear();
    }
    observe_config(&mut seen.1);
}

#[cfg(any(test, feature = "simulated-physical", feature = "live-fail-closed"))]
pub fn refresh_from_service() -> bool {
    match transact(RequestKind::GetStatus) {
        Ok(response) => {
            publish(&client::view_from_status(&response));
            true
        }
        Err(_) => false,
    }
}

fn watch_config_file() {
    let mut seen = String::new();
    while !QUIT.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(400));
        observe_config(&mut seen);
    }
}

fn config_watch_ready(view: &OwnerView) -> bool {
    view.service_running && view.state.is_some()
}

fn observe_config(seen: &mut String) {
    if !config_watch_ready(&current_view()) {
        return;
    }
    let Some(path) = profile_config_path() else {
        return;
    };
    let Some(disk) = read_user_file(&path) else {
        return;
    };
    if disk != *seen {
        let locked = current_view().locked();
        // A failed read of the effective config must not swallow the edit.
        // The next poll tries the same bytes again.
        let Some(effective) = current_config() else {
            return;
        };
        let action = actions::config_disk_action(locked, &effective, &disk);
        *seen = disk.clone();
        match action {
            actions::ConfigDisk::Save => {
                let previous_hotkey = registered_hotkey();
                if let Some(revision) = config_revision() {
                    match transact(RequestKind::PutConfig {
                        toml: disk,
                        base_revision: revision,
                    }) {
                        Ok(ResponseKind::Invalid { errors }) => {
                            *CONFIG_ERRORS
                                .lock()
                                .unwrap_or_else(|poison| poison.into_inner()) = errors.join("\n");
                            publish_current();
                        }
                        Ok(ResponseKind::Applied) => {
                            if keep_hotkey_registered(&previous_hotkey, seen, Some(effective)) {
                                *CONFIG_ERRORS
                                    .lock()
                                    .unwrap_or_else(|poison| poison.into_inner()) = String::new();
                            }
                            refresh_hotkey();
                            publish_current();
                        }
                        _ => {}
                    }
                }
            }
            actions::ConfigDisk::Offer => {
                CONFIG_OFFER.store(true, Ordering::Release);
                *PENDING_CONFIG
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = disk;
                publish_current();
            }
            actions::ConfigDisk::Unchanged => {}
        }
    }
    let errors_path = path.with_file_name(softlock_core::CONFIG_ERRORS_NAME);
    if let Some(errors) = read_user_file(&errors_path) {
        let mut slot = CONFIG_ERRORS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if *slot != errors {
            *slot = errors;
            drop(slot);
            publish_current();
        }
    }
}

fn profile_config_path() -> Option<std::path::PathBuf> {
    #[cfg(any(feature = "simulated-physical", feature = "live-fail-closed"))]
    if let Ok(dir) = std::env::var("SOFTLOCK_CONFIG_DIR")
        && !dir.is_empty()
        && config_override_allowed()
    {
        return Some(std::path::PathBuf::from(dir).join(softlock_core::CONFIG_FILE_NAME));
    }
    roaming_config()
}

#[cfg(feature = "simulated-physical")]
fn config_override_allowed() -> bool {
    true
}

#[cfg(all(feature = "live-fail-closed", not(feature = "simulated-physical")))]
fn config_override_allowed() -> bool {
    softlock_windows::runner_may_lock().is_ok()
}

fn roaming_config() -> Option<std::path::PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_RoamingAppData, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath,
    };
    let wide =
        unsafe { SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KNOWN_FOLDER_FLAG(0), None) }
            .ok()?;
    let text = unsafe { wide.to_string() }.ok()?;
    unsafe { CoTaskMemFree(Some(wide.0.cast())) };
    Some(
        std::path::PathBuf::from(text)
            .join(softlock_core::SERVICE_NAME)
            .join(softlock_core::CONFIG_FILE_NAME),
    )
}

fn read_user_file(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let file = open_user_file(path, false)?;
    let read = call_with_deadline(Duration::from_secs(5), move || {
        let mut limited = file.take(MAX_CONFIG_BYTES as u64 + 1);
        let mut body = String::new();
        limited.read_to_string(&mut body).ok()?;
        if body.len() > MAX_CONFIG_BYTES {
            return None;
        }
        Some(body)
    });
    read.flatten()
}

fn write_user_file(path: &std::path::Path, body: &str) -> bool {
    use std::io::Write;
    if body.len() > MAX_CONFIG_BYTES {
        return false;
    }
    let Some(mut file) = open_user_file(path, true) else {
        return false;
    };
    let body = body.to_string();
    call_with_deadline(Duration::from_secs(5), move || {
        file.write_all(body.as_bytes()).is_ok() && file.set_len(body.len() as u64).is_ok()
    })
    .unwrap_or(false)
}

/// `None` when `work` is still running. The worker is left blocked; the
/// caller has to fail closed instead of waiting with it.
fn call_with_deadline<T: Send + 'static>(
    limit: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver.recv_timeout(limit).ok()
}

fn open_user_file(path: &std::path::Path, write: bool) -> Option<File> {
    let path_buf = path.to_path_buf();
    match call_with_deadline(Duration::from_secs(5), move || {
        open_user_file_walk(&path_buf, write)
    }) {
        Some(opened) => opened,
        // The component walk sits in `CreateFileW` or `NtCreateFile` on the
        // runner. A full-path open still refuses a reparse at the final name.
        None => {
            let path_buf = path.to_path_buf();
            call_with_deadline(Duration::from_secs(5), move || {
                open_user_file_whole(&path_buf, write)
            })
            .flatten()
        }
    }
}

fn open_user_file_whole(path: &std::path::Path, write: bool) -> Option<File> {
    use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
    let text = path.to_string_lossy().replace('/', "\\");
    if text.contains("..") {
        return None;
    }
    let wide_path = wide(&text);
    let access = if write {
        FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0
    } else {
        FILE_GENERIC_READ.0
    };
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
        .ok()?
    };
    if handle_is_reparse(handle) {
        close_user_handle(handle);
        return None;
    }
    Some(unsafe { File::from_raw_handle(handle.0) })
}

fn open_user_file_walk(path: &std::path::Path, write: bool) -> Option<File> {
    use windows::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_GENERIC_WRITE};
    let text = path.to_string_lossy().replace('/', "\\");
    if text.contains("..") {
        return None;
    }
    let mut names = text.split('\\').filter(|part| !part.is_empty());
    let drive = names.next()?;
    let bytes = drive.as_bytes();
    if bytes.len() != 2 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return None;
    }
    let names: Vec<&str> = names.collect();
    if names.is_empty() {
        return None;
    }
    let file_access = if write {
        FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0
    } else {
        FILE_GENERIC_READ.0
    };
    // Each component is opened from the parent handle, with the reparse
    // option, so a junction swapped in between checks is not followed.
    let mut current = open_root(drive)?;
    if handle_is_reparse(current) {
        close_user_handle(current);
        return None;
    }
    for (index, name) in names.iter().enumerate() {
        let last = index + 1 == names.len();
        let access = if last { file_access } else { DIR_OPEN };
        let child = match open_relative(Some(current), name, access, !last) {
            Some(child) => child,
            None => {
                close_user_handle(current);
                return None;
            }
        };
        close_user_handle(current);
        if handle_is_reparse(child) {
            close_user_handle(child);
            return None;
        }
        if last {
            return Some(unsafe { File::from_raw_handle(child.0) });
        }
        current = child;
    }
    close_user_handle(current);
    None
}

const DIR_OPEN: u32 = 0x0001 | 0x0020 | 0x0080 | 0x0010_0000;

fn close_user_handle(handle: windows::Win32::Foundation::HANDLE) {
    unsafe {
        let _ = CloseHandle(handle);
    }
}

fn open_relative(
    parent: Option<windows::Win32::Foundation::HANDLE>,
    name: &str,
    access: u32,
    directory: bool,
) -> Option<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    if name.is_empty() || name.contains('\\') || name.contains('/') || name.contains("..") {
        return None;
    }
    let mut wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut unicode = UserUnicodeString {
        length: ((wide.len() - 1) * 2) as u16,
        maximum_length: (wide.len() * 2) as u16,
        buffer: wide.as_mut_ptr(),
    };
    let mut attributes = UserObjectAttributes {
        length: std::mem::size_of::<UserObjectAttributes>() as u32,
        root_directory: parent
            .map(|handle| handle.0)
            .unwrap_or(std::ptr::null_mut()),
        object_name: &mut unicode,
        attributes: 0x0000_0040,
        security_descriptor: std::ptr::null_mut(),
        security_quality_of_service: std::ptr::null_mut(),
    };
    let mut iosb = UserIoStatus {
        status: 0,
        information: 0,
    };
    let mut raw = std::ptr::null_mut();
    let mut options = if directory {
        0x0000_0001 | 0x0000_0020
    } else {
        0x0000_0040 | 0x0000_0020
    };
    options |= 0x0020_0000;
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            access,
            &mut attributes,
            &mut iosb,
            std::ptr::null_mut(),
            FILE_ATTRIBUTE_NORMAL.0,
            0x0000_0007,
            1,
            options,
            std::ptr::null_mut(),
            0,
        )
    };
    if status < 0 || raw.is_null() {
        None
    } else {
        Some(HANDLE(raw))
    }
}

#[repr(C)]
struct UserUnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[repr(C)]
struct UserObjectAttributes {
    length: u32,
    root_directory: *mut core::ffi::c_void,
    object_name: *mut UserUnicodeString,
    attributes: u32,
    security_descriptor: *mut core::ffi::c_void,
    security_quality_of_service: *mut core::ffi::c_void,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<UserObjectAttributes>() == 48);

#[repr(C)]
struct UserIoStatus {
    status: isize,
    information: usize,
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtCreateFile(
        file_handle: *mut *mut core::ffi::c_void,
        desired_access: u32,
        object_attributes: *mut UserObjectAttributes,
        io_status_block: *mut UserIoStatus,
        allocation_size: *mut i64,
        file_attributes: u32,
        share_access: u32,
        create_disposition: u32,
        create_options: u32,
        ea_buffer: *mut core::ffi::c_void,
        ea_length: u32,
    ) -> i32;
}

fn open_root(drive: &str) -> Option<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::core::PCWSTR;
    // `NtCreateFile` on `\??\C:\` plus `FileAttributeTagInfo` fails for the
    // volume root, and a failed query was treated as a reparse. Every config
    // read then stopped. `CreateFileW` is the same root open the service uses.
    let path = wide(&format!("{drive}\\"));
    unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            DIR_OPEN,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
        .ok()
    }
}

fn handle_is_reparse(handle: windows::Win32::Foundation::HANDLE) -> bool {
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(handle, &mut info) }.is_err() {
        return true;
    }
    info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
}

fn notice_text() -> String {
    let mut parts = Vec::new();
    let fallback = FALLBACK_NOTICE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    if !fallback.is_empty() {
        parts.push(fallback);
    }
    if !HOTKEY_OK.load(Ordering::Acquire) {
        parts.push(ui_text("hotkeyUnavailable"));
    }
    if let Some(notice) = actions::autostart_failure_notice(AUTOSTART_CODE.load(Ordering::Acquire))
    {
        parts.push(notice.to_string());
    }
    parts.join("; ")
}

fn write_status(view: &OwnerView) {
    let Some(path) = status_file_path() else {
        return;
    };
    let seq = STATUS_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let hotkey = if HOTKEY_OK.load(Ordering::Acquire) {
        "ok"
    } else {
        "failed"
    };
    let label = HOTKEY_LABEL
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    let mut text = status_text(view);
    text.push_str(&format!("hotkey={hotkey}\n"));
    text.push_str(&format!("chord={label}\n"));
    text.push_str(&format!("notice={}\n", notice_text()));
    text.push_str(&format!("seq={seq}\n"));
    text.push_str(&format!("hwnd={}\n", WINDOW.load(Ordering::Acquire)));
    let _ = std::fs::write(path, text);
}

fn cancel_watch_read() {
    let tid = WATCH_THREAD.load(Ordering::Acquire);
    if tid == 0 {
        return;
    }
    let Ok(thread) = (unsafe { OpenThread(THREAD_TERMINATE, false, tid) }) else {
        return;
    };
    unsafe {
        let _ = CancelSynchronousIo(thread);
        let _ = CloseHandle(thread);
    }
}

fn watch_states() {
    WATCH_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    while !QUIT.load(Ordering::Acquire) {
        let mut io = match connect_retry() {
            Ok(io) => io,
            Err(_) => {
                publish(&OwnerView::unknown());
                thread::sleep(Duration::from_millis(300));
                continue;
            }
        };
        let id = client::next_id();
        if exchange(&mut io, id, &RequestKind::Subscribe).is_err() {
            publish(&OwnerView::unknown());
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
                    publish(&OwnerView::unknown());
                    break;
                }
            };
            let decoded = client::decode_response(&frame);
            if let Ok(ResponseKind::FellBackToOsLock { fallback }) = &decoded {
                set_fallback_notice(actions::fallback_notice(fallback));
            }
            let next = client::view_after(&current_view(), &decoded);
            if decoded.is_ok() {
                publish(&next);
            } else {
                publish(&OwnerView::unknown());
                break;
            }
        }
    }
}

fn window_loop() -> i32 {
    let class_name = wide(&owner_class_name());
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
    publish_owner_hwnd(hwnd);
    let hotkey = registered_hotkey();
    let registered = register_hotkey(hwnd, &hotkey);
    let label = current_hotkey_label().unwrap_or_else(|| "Ctrl+Alt+Shift+L".to_string());
    *HOTKEY_LABEL
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = label;
    HOTKEY_OK.store(registered, Ordering::Release);
    add_tray(hwnd);
    publish(&current_view());
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
            enqueue_lock(LockSource::Hotkey);
            LRESULT(0)
        }
        WM_LOCK_APP => {
            lock_from_app();
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
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
                CMD_LOCK => enqueue_lock(LockSource::Tray),
                CMD_HARD_LOCK => enqueue_hard_lock(),
                CMD_QUIT => {
                    let _ = request_quit();
                }
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
    let lock_now = wide(&ui_text("lockNow"));
    let lock_for_real = wide(&ui_text("lockForReal"));
    let quit = wide(&ui_text("quit"));
    let notice = wide(&notice_text());
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 1, PCWSTR(status.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 2, PCWSTR(service.as_ptr()));
        if !notice_text().is_empty() {
            let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 3, PCWSTR(notice.as_ptr()));
        }
        let _ = AppendMenuW(menu, MF_STRING, CMD_LOCK, PCWSTR(lock_now.as_ptr()));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            CMD_HARD_LOCK,
            PCWSTR(lock_for_real.as_ptr()),
        );
        let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, PCWSTR(quit.as_ptr()));
        let mut point = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, Some(0), hwnd, None);
        let _ = DestroyMenu(menu);
    }
}

fn publish_owner_hwnd(hwnd: HWND) {
    #[cfg(feature = "live-fail-closed")]
    {
        if softlock_windows::runner_may_lock().is_ok()
            && let Ok(path) = std::env::var("SOFTLOCK_OWNER_HWND")
        {
            let _ = std::fs::write(path, (hwnd.0 as isize).to_string());
        }
    }
    #[cfg(not(feature = "live-fail-closed"))]
    {
        let _ = hwnd;
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
    let notice = notice_text();
    let tip = wide(&format!(
        "{} — {}{}",
        status_line(view.service_running, view.state),
        service_line(view.service_running),
        if notice.is_empty() {
            String::new()
        } else {
            format!(" — {notice}")
        }
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
    let code = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            REG_SZ.0,
            Some(command.as_ptr().cast()),
            (command.len() * 2) as u32,
        )
    };
    AUTOSTART_CODE.store(code.0, Ordering::Release);
}

fn transact(kind: RequestKind) -> Result<ResponseKind, String> {
    // `connect` and `exchange` sit in `CreateFileW` / `ReadFile`. The retry
    // deadline is only checked once those calls return, so a busy pipe held
    // the caller until the job was killed. The worker may stay blocked after
    // this returns; the caller must not.
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| {
            let mut io = connect_retry()?;
            exchange(&mut io, client::next_id(), &kind).map_err(|error| format!("{error:?}"))
        })();
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|_| Err("timed out waiting for the service".to_string()))
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
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(open_pipe());
    });
    receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_else(|_| Err("timed out opening the service pipe".to_string()))
}

fn open_pipe() -> Result<File, String> {
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

#[cfg(test)]
mod owner_file_tests {
    use super::{
        CONFIG_ERRORS, CONFIG_OFFER, WINDOW, config_errors, config_offer, config_watch_ready,
        poll_config_once, publish, read_user_file, register_hotkey, replace_registered_hotkey,
    };
    use crate::actions::Hotkey;
    use crate::client::OwnerView;
    use softlock_core::{MAX_CONFIG_BYTES, StateName};
    use std::sync::atomic::Ordering;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_SHIFT, RegisterHotKey, UnregisterHotKey,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, HWND_MESSAGE, RegisterClassW, WNDCLASSW,
    };
    use windows::core::PCWSTR;

    unsafe extern "system" fn probe_wndproc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    #[test]
    fn a_plain_config_file_is_read() {
        let dir = temp_dir("sl-read");
        let path = dir.join("config.toml");
        std::fs::write(&path, "schema_version = 1\n").expect("write");
        assert_eq!(
            read_user_file(&path).as_deref(),
            Some("schema_version = 1\n")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_oversized_config_file_is_not_read() {
        let dir = temp_dir("sl-over");
        let path = dir.join("config.toml");
        std::fs::write(&path, vec![b'a'; MAX_CONFIG_BYTES + 1]).expect("write");
        assert!(read_user_file(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlinked_config_file_is_not_read() {
        let dir = temp_dir("sl-owner-link");
        let target = dir.join("target.toml");
        std::fs::write(&target, "secret").expect("target");
        let link = dir.join("config.toml");
        let status = std::process::Command::new("cmd")
            .args([
                "/c",
                "mklink",
                &link.display().to_string(),
                &target.display().to_string(),
            ])
            .status()
            .expect("mklink");
        assert!(status.success(), "mklink did not plant the symlink");
        assert!(read_user_file(&link).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_view_does_not_offer_a_changed_config_file() {
        let dir = temp_dir("sl-offer");
        unsafe {
            std::env::set_var("SOFTLOCK_CONFIG_DIR", &dir);
        }
        std::fs::write(
            dir.join(softlock_core::CONFIG_FILE_NAME),
            "schema_version = 1\n[lock]\nhotkey = \"Ctrl+Alt+Shift+K\"\n",
        )
        .expect("write");
        publish(&OwnerView::unknown());
        CONFIG_OFFER.store(false, Ordering::Release);
        assert!(!config_watch_ready(&OwnerView::unknown()));
        let live = OwnerView {
            service_running: true,
            state: Some(StateName::Unlocked),
            lock_source: None,
        };
        assert!(config_watch_ready(&live));
        poll_config_once();
        assert!(!config_offer());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_hotkey_registration_keeps_the_old_hotkey() {
        let class_name = super::wide(&format!("sl-hk-{}", std::process::id()));
        let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .expect("module");
        let class = WNDCLASSW {
            lpfnWndProc: Some(probe_wndproc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        assert!(unsafe { RegisterClassW(&class) } != 0, "class");
        let hwnd = unsafe {
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
        }
        .expect("window");
        let blocker = unsafe {
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
        }
        .expect("blocker");
        WINDOW.store(hwnd.0 as isize, Ordering::Release);
        let old = Hotkey {
            control: true,
            alt: true,
            shift: true,
            virtual_key: u16::from(b'8'),
        };
        let new = Hotkey {
            control: true,
            alt: true,
            shift: true,
            virtual_key: u16::from(b'9'),
        };
        assert!(
            register_hotkey(hwnd, &old),
            "the old hotkey did not register"
        );
        assert!(
            unsafe {
                RegisterHotKey(
                    Some(blocker),
                    2,
                    MOD_CONTROL | MOD_ALT | MOD_SHIFT,
                    u32::from(new.virtual_key),
                )
            }
            .is_ok(),
            "the blocker did not take the new hotkey"
        );
        *CONFIG_ERRORS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = String::new();
        assert!(!replace_registered_hotkey(&old, &new));
        unsafe {
            let _ = UnregisterHotKey(Some(blocker), 2);
        }
        assert!(
            unsafe {
                RegisterHotKey(
                    Some(blocker),
                    2,
                    MOD_CONTROL | MOD_ALT | MOD_SHIFT,
                    u32::from(old.virtual_key),
                )
            }
            .is_err(),
            "the old hotkey was no longer registered"
        );
        assert!(
            config_errors().contains("could not be registered"),
            "{}",
            config_errors()
        );
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), 1);
            let _ = UnregisterHotKey(Some(blocker), 2);
        }
        WINDOW.store(0, Ordering::Release);
    }
}
