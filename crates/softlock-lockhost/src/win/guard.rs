//! Low-level keyboard and mouse hooks. The callback does no I/O and no
//! allocation. Liveness is a marker this process injects, not a guess that
//! Windows will remove a slow hook.
//!
//! A split token on client Windows is ticket 06. The input-hook finding
//! measured Server images only: the console token there is already high
//! integrity and has no linked token. This dev launch installs the hooks in
//! the calling process. Ticket 13 is the SYSTEM-in-session launch.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, HMODULE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_F24,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_UP, LLMHF_INJECTED, MSG,
    MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_SYSKEYDOWN,
    WM_XBUTTONDOWN,
};
use windows::core::PCWSTR;

use crate::classify::{HookInput, InputClass, classify};

use super::queue::{KIND_CLICK, KIND_KEY, KIND_WHEEL, RawEvent};
use super::session::{SESSION, Shared};

/// `dwExtraInfo` on the no-op liveness marker. Not the simulated-physical nonce.
const LIVENESS_EXTRA: u64 = 0x50F7_10C5_A11E_0001;
const LIVENESS_PERIOD: Duration = Duration::from_millis(250);
/// Our own deadline after the marker is injected. This is not a Windows
/// hook-removal budget.
const LIVENESS_DEADLINE: Duration = Duration::from_millis(750);

pub(crate) fn install(shared: &Arc<Shared>) -> Result<(), softlock_core::GuardError> {
    SESSION.store(
        Arc::as_ptr(shared).cast_mut(),
        std::sync::atomic::Ordering::Release,
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let worker_shared = Arc::clone(shared);
    let worker = thread::spawn(move || hook_thread(worker_shared, tx));
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(thread_id)) => {
            shared
                .hook_thread_id
                .store(thread_id, std::sync::atomic::Ordering::Release);
            *shared
                .hook
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = Some(worker);
            Ok(())
        }
        Ok(Err(())) => {
            let _ = worker.join();
            SESSION.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
            Err(softlock_core::GuardError::Failed)
        }
        Err(_) => {
            shared
                .stop
                .store(true, std::sync::atomic::Ordering::Release);
            let _ = worker.join();
            SESSION.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
            Err(softlock_core::GuardError::Failed)
        }
    }
}

pub(crate) fn remove(shared: &Shared) {
    shared
        .stop
        .store(true, std::sync::atomic::Ordering::Release);
    if let Some(liveness) = shared
        .liveness
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
    {
        let _ = liveness.join();
    }
    let thread_id = shared
        .hook_thread_id
        .load(std::sync::atomic::Ordering::Acquire);
    if thread_id != 0 {
        unsafe {
            let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
    if let Some(hook) = shared
        .hook
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
    {
        let _ = hook.join();
    }
    shared
        .hook_thread_id
        .store(0, std::sync::atomic::Ordering::Release);
    SESSION.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
}

pub(crate) fn liveness(shared: Arc<Shared>) {
    let mut seen = shared.last_event.load(std::sync::atomic::Ordering::Acquire);
    while !shared.stop.load(std::sync::atomic::Ordering::Acquire) {
        if !sleep_until(LIVENESS_PERIOD, &shared) {
            return;
        }
        let now = shared.last_event.load(std::sync::atomic::Ordering::Acquire);
        if now != seen {
            seen = now;
            continue;
        }
        let before = shared
            .marker_seen
            .load(std::sync::atomic::Ordering::Acquire);
        inject_marker();
        let deadline = Instant::now() + LIVENESS_DEADLINE;
        let mut observed = false;
        while Instant::now() < deadline {
            if shared.stop.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            if shared
                .marker_seen
                .load(std::sync::atomic::Ordering::Acquire)
                != before
            {
                observed = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if !observed {
            shared
                .fail
                .store(true, std::sync::atomic::Ordering::Release);
            return;
        }
        seen = shared.last_event.load(std::sync::atomic::Ordering::Acquire);
    }
}

fn sleep_until(total: Duration, shared: &Shared) -> bool {
    let started = Instant::now();
    while started.elapsed() < total {
        if shared.stop.load(std::sync::atomic::Ordering::Acquire) {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    !shared.stop.load(std::sync::atomic::Ordering::Acquire)
}

fn inject_marker() {
    let inputs = [key_event(VK_F24.0, false), key_event(VK_F24.0, true)];
    unsafe {
        let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn key_event(virtual_key: u16, up: bool) -> INPUT {
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
                dwExtraInfo: LIVENESS_EXTRA as usize,
            },
        },
    }
}

fn hook_thread(shared: Arc<Shared>, ready: std::sync::mpsc::Sender<Result<u32, ()>>) {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
    }
    let keyboard = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_proc),
            Some(module_instance()),
            0,
        )
    };
    let keyboard = match keyboard {
        Ok(hook) => hook,
        Err(_) => {
            let _ = ready.send(Err(()));
            return;
        }
    };
    let mouse =
        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(module_instance()), 0) };
    let mouse = match mouse {
        Ok(hook) => hook,
        Err(_) => {
            unsafe {
                let _ = UnhookWindowsHookEx(keyboard);
            }
            let _ = ready.send(Err(()));
            return;
        }
    };
    let mut message = MSG::default();
    unsafe {
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
    }
    let thread_id = unsafe { GetCurrentThreadId() };
    let _ = ready.send(Ok(thread_id));
    // Keep the arc alive for the callbacks, which load the raw pointer.
    let _keep = shared;
    loop {
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 == 0 || status.0 == -1 {
            break;
        }
    }
    unsafe {
        let _ = UnhookWindowsHookEx(keyboard);
        let _ = UnhookWindowsHookEx(mouse);
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code < 0 {
            return CallNextHookEx(None, code, wparam, lparam);
        }
        let Some(shared) = session_ptr() else {
            return CallNextHookEx(None, code, wparam, lparam);
        };
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        shared
            .last_event
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        let extra = info.dwExtraInfo as u64;
        let vk = info.vkCode as u16;
        if vk == VK_F24.0 && extra == LIVENESS_EXTRA {
            shared
                .marker_seen
                .fetch_add(1, std::sync::atomic::Ordering::Release);
            return LRESULT(1);
        }
        let injected = info.flags.contains(LLKHF_INJECTED);
        if !blocks(injected, extra) {
            return CallNextHookEx(None, code, wparam, lparam);
        }
        note_physical(shared);
        let down = !info.flags.contains(LLKHF_UP)
            && (wparam.0 == WM_KEYDOWN as usize || wparam.0 == WM_SYSKEYDOWN as usize);
        if down {
            shared
                .wake
                .fetch_add(1, std::sync::atomic::Ordering::Release);
            shared.queue.push(RawEvent {
                kind: KIND_KEY,
                vk,
                scan: info.scanCode as u16,
            });
        }
        LRESULT(1)
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code < 0 {
            return CallNextHookEx(None, code, wparam, lparam);
        }
        let Some(shared) = session_ptr() else {
            return CallNextHookEx(None, code, wparam, lparam);
        };
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        shared
            .last_event
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        let extra = info.dwExtraInfo as u64;
        let injected = info.flags & LLMHF_INJECTED != 0;
        if !blocks(injected, extra) {
            return CallNextHookEx(None, code, wparam, lparam);
        }
        note_physical(shared);
        let message = wparam.0 as u32;
        if message == WM_LBUTTONDOWN
            || message == WM_RBUTTONDOWN
            || message == WM_MBUTTONDOWN
            || message == WM_XBUTTONDOWN
        {
            shared
                .wake
                .fetch_add(1, std::sync::atomic::Ordering::Release);
            shared.queue.push(RawEvent {
                kind: KIND_CLICK,
                vk: 0,
                scan: 0,
            });
        } else if message == WM_MOUSEWHEEL || message == WM_MOUSEHWHEEL {
            shared
                .wake
                .fetch_add(1, std::sync::atomic::Ordering::Release);
            shared.queue.push(RawEvent {
                kind: KIND_WHEEL,
                vk: 0,
                scan: 0,
            });
        }
        let _ = WM_MOUSEMOVE;
        LRESULT(1)
    }
}

fn blocks(injected: bool, extra_info: u64) -> bool {
    !matches!(
        classify(HookInput {
            injected,
            extra_info,
        }),
        InputClass::Injected
    )
}

fn note_physical(shared: &Shared) {
    let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
    shared
        .last_physical_ms
        .store(now, std::sync::atomic::Ordering::Release);
    shared
        .saw_physical
        .store(true, std::sync::atomic::Ordering::Release);
}

pub(crate) fn session_ptr() -> Option<&'static Shared> {
    let pointer = SESSION.load(std::sync::atomic::Ordering::Acquire);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { &*pointer })
    }
}

fn module_instance() -> HINSTANCE {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.unwrap_or(HMODULE::default());
    HINSTANCE(module.0)
}
