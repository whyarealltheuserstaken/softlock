//! Low-level keyboard hook used to read a real `SendInput` event back.

use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, HMODULE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, PM_NOREMOVE, PeekMessageW,
    PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_QUIT,
};
use windows::core::PCWSTR;

use super::error::{HarnessError, explain};
use super::probe::send_virtual_key;

struct Seen {
    expected: u16,
    virtual_key: Option<u16>,
    injected: bool,
    extra_info: u64,
}

static SEEN: Mutex<Seen> = Mutex::new(Seen {
    expected: 0,
    virtual_key: None,
    injected: false,
    extra_info: 0,
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookObservation {
    pub virtual_key: u16,
    pub injected: bool,
    pub extra_info: u64,
}

#[cfg(feature = "simulated-physical")]
pub fn classify_observed(observed: HookObservation) -> softlock_lockhost::InputClass {
    softlock_lockhost::classify(softlock_lockhost::HookInput {
        injected: observed.injected,
        extra_info: observed.extra_info,
    })
}

pub fn observe_injected_key(
    virtual_key: u16,
    extra_info: u64,
) -> Result<HookObservation, HarnessError> {
    reset(virtual_key);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || hook_thread(ready_tx));
    let ready = ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| HarnessError::new("hook thread did not start"))?;
    let thread_id = ready?;
    thread::sleep(Duration::from_millis(50));
    let sent = send_virtual_key(virtual_key, extra_info);
    if let Err(error) = sent {
        let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        let _ = worker.join();
        return Err(error);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut seen = snapshot();
    while Instant::now() < deadline && seen.virtual_key != Some(virtual_key) {
        thread::sleep(Duration::from_millis(10));
        seen = snapshot();
    }
    let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    let joined = worker.join();
    if let Err(panic) = joined {
        let text = panic
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "hook thread panicked".to_string());
        return Err(HarnessError::new(text));
    }
    let virtual_key = seen
        .virtual_key
        .ok_or_else(|| HarnessError::new(format!("hook did not see virtual key {virtual_key}")))?;
    Ok(HookObservation {
        virtual_key,
        injected: seen.injected,
        extra_info: seen.extra_info,
    })
}

fn hook_thread(ready: std::sync::mpsc::Sender<Result<u32, HarnessError>>) {
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_proc),
            Some(module_instance()),
            0,
        )
    };
    let hook = match hook {
        Ok(hook) => hook,
        Err(error) => {
            let _ = ready.send(Err(explain(error, "keyboard hook")));
            return;
        }
    };
    let mut message = MSG::default();
    unsafe {
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
    }
    let thread_id = unsafe { GetCurrentThreadId() };
    let _ = ready.send(Ok(thread_id));
    loop {
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 == 0 || status.0 == -1 {
            break;
        }
    }
    unsafe {
        let _ = UnhookWindowsHookEx(hook);
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 && wparam.0 == WM_KEYDOWN as usize {
            let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let mut seen = SEEN.lock().unwrap_or_else(|poison| poison.into_inner());
            if seen.expected == info.vkCode as u16 {
                seen.virtual_key = Some(info.vkCode as u16);
                seen.injected = info.flags.contains(LLKHF_INJECTED);
                seen.extra_info = info.dwExtraInfo as u64;
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

fn reset(expected: u16) {
    let mut seen = SEEN.lock().unwrap_or_else(|poison| poison.into_inner());
    *seen = Seen {
        expected,
        virtual_key: None,
        injected: false,
        extra_info: 0,
    };
}

fn snapshot() -> Seen {
    let seen = SEEN.lock().unwrap_or_else(|poison| poison.into_inner());
    Seen {
        expected: seen.expected,
        virtual_key: seen.virtual_key,
        injected: seen.injected,
        extra_info: seen.extra_info,
    }
}

fn module_instance() -> HINSTANCE {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.unwrap_or(HMODULE::default());
    HINSTANCE(module.0)
}
