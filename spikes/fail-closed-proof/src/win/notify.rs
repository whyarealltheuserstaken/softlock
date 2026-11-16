//! Session-notification window. The callback only records the note and the time.

use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_ALL_SESSIONS, NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
    WTSUnRegisterSessionNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_USERDATA, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassExW, SetWindowLongPtrW, TranslateMessage, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_NCCREATE, WNDCLASSEXW,
};
use windows::core::{PCWSTR, w};

use super::util::{self, qpc};

pub const WM_WTSSESSION_CHANGE: u32 = 0x02B1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    pub kind: u32,
    pub session: u32,
    pub qpc: u64,
}

static NOTES: Mutex<Vec<Note>> = Mutex::new(Vec::new());
static CURSOR: AtomicUsize = AtomicUsize::new(0);
static HWND_SLOT: AtomicIsize = AtomicIsize::new(0);

pub struct Listener {
    thread: Option<JoinHandle<()>>,
}

pub fn start(all_sessions: bool) -> Result<Listener, String> {
    let (tx, rx) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("session-notify".to_string())
        .spawn(move || thread_main(all_sessions, tx))
        .map_err(|error| format!("spawn listener: {error}"))?;
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(Listener {
            thread: Some(thread),
        }),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error)
        }
        Err(_) => Err("session listener did not start".to_string()),
    }
}

impl Listener {
    pub fn wait(
        &self,
        kind: u32,
        session: u32,
        not_before: u64,
        timeout: Duration,
    ) -> Option<Note> {
        let start = std::time::Instant::now();
        loop {
            if let Some(note) = find(kind, session, not_before) {
                return Some(note);
            }
            if start.elapsed() >= timeout {
                return find(kind, session, not_before);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    pub fn stop(mut self) {
        let raw = HWND_SLOT.load(Ordering::SeqCst);
        if raw != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(raw as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let raw = HWND_SLOT.load(Ordering::SeqCst);
        if raw != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(raw as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn find(kind: u32, session: u32, not_before: u64) -> Option<Note> {
    let notes = NOTES.lock().unwrap_or_else(|poison| poison.into_inner());
    notes
        .iter()
        .find(|note| note.kind == kind && note.session == session && note.qpc >= not_before)
        .copied()
}

pub fn since(not_before: u64) -> Vec<Note> {
    let notes = NOTES.lock().unwrap_or_else(|poison| poison.into_inner());
    notes
        .iter()
        .copied()
        .filter(|note| note.qpc >= not_before)
        .collect()
}

pub fn drain() -> Vec<Note> {
    let notes = NOTES.lock().unwrap_or_else(|poison| poison.into_inner());
    let cursor = CURSOR.load(Ordering::SeqCst).min(notes.len());
    let fresh = notes[cursor..].to_vec();
    CURSOR.store(notes.len(), Ordering::SeqCst);
    fresh
}

fn thread_main(all_sessions: bool, ready: mpsc::Sender<Result<(), String>>) {
    let flags = if all_sessions {
        NOTIFY_FOR_ALL_SESSIONS
    } else {
        NOTIFY_FOR_THIS_SESSION
    };
    let hwnd = match create_window() {
        Ok(hwnd) => hwnd,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    HWND_SLOT.store(hwnd.0 as isize, Ordering::SeqCst);
    if let Err(error) = unsafe { WTSRegisterSessionNotification(hwnd, flags) } {
        let _ = ready.send(Err(util::explain(error, "WTSRegisterSessionNotification")));
        return;
    }
    let _ = ready.send(Ok(()));
    let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 == 0 || status.0 == -1 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    unsafe {
        let _ = WTSUnRegisterSessionNotification(hwnd);
    }
    HWND_SLOT.store(0, Ordering::SeqCst);
}

fn create_window() -> Result<HWND, String> {
    register_class()?;
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("FailClosedProbeNotify"),
            w!("fail-closed-probe"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance()),
            None,
        )
        .map_err(|error| util::explain(error, "CreateWindowExW"))
    }
}

fn register_class() -> Result<(), String> {
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance(),
        lpszClassName: w!("FailClosedProbeNotify"),
        ..WNDCLASSEXW::default()
    };
    let atom = unsafe { RegisterClassExW(&class) };
    if atom == 0 {
        let error = util::last_error("RegisterClassExW");
        if error.contains("win32 1410") {
            return Ok(());
        }
        return Err(error);
    }
    Ok(())
}

fn instance() -> HINSTANCE {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.unwrap_or(HMODULE::default());
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
            WM_NCCREATE => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 1);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_WTSSESSION_CHANGE => {
                let note = Note {
                    kind: wparam.0 as u32,
                    session: lparam.0 as u32,
                    qpc: qpc(),
                };
                if let Ok(mut notes) = NOTES.lock() {
                    notes.push(note);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
