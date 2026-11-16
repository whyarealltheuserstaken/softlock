//! Low-level keyboard and mouse hooks. The callback does no allocation and
//! does not log. A swallowed move only stores an atomic: the cursor has
//! already moved by the time `WH_MOUSE_LL` runs, and the window thread puts
//! it back. An injected move reads the cursor once, after the event is
//! delivered, so that undo has the position it actually landed on. Liveness
//! is a marker this process injects, not a guess that Windows will remove a
//! slow hook.
//!
//! A split token on client Windows is ticket 06. The input-hook finding
//! measured Server images only: the console token there is already high
//! integrity and has no linked token. This dev launch installs the hooks in
//! the calling process. Ticket 13 is the SYSTEM-in-session launch.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, HMODULE, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_F24,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetCursorPos, GetMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_UP,
    LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    SetCursorPos, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT,
    WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_XBUTTONDOWN,
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
    if let Some((x, y)) = cursor_now() {
        note_allowed_cursor(&shared, x, y);
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
        let message = wparam.0 as u32;
        note_mouse_sample(shared, info);
        let injected = info.flags & LLMHF_INJECTED != 0;
        let extra = mouse_extra(info, injected);
        if !blocks(injected, extra) {
            let forwarded = CallNextHookEx(None, code, wparam, lparam);
            // Injected moves are allowed to move the cursor. Remember where
            // this event put it so a later swallowed move can be undone.
            if message == WM_MOUSEMOVE {
                note_allowed_cursor(shared, info.pt.x, info.pt.y);
            }
            return forwarded;
        }
        shared
            .mouse_blocked
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        note_physical(shared);
        // The cursor has already moved by the time this hook runs. Ask the
        // window thread to put it back. The callback itself does not call
        // out to user32.
        if message == WM_MOUSEMOVE {
            let generation = shared.cursor_gen.load(std::sync::atomic::Ordering::Acquire);
            if generation != 0 {
                shared
                    .snap_gen
                    .store(generation, std::sync::atomic::Ordering::Release);
            }
        } else if message == WM_LBUTTONDOWN
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

/// `dwExtraInfo` from a mouse hook. The keyboard struct is naturally aligned
/// and its extra field arrives intact. The mouse struct is not: an 8-aligned
/// view and a 4-packed view put the extra field on different offsets, and a
/// 32-bit copy keeps only the low half. Return the value the classifier
/// treats as blocked.
fn mouse_extra(info: &MSLLHOOKSTRUCT, injected: bool) -> u64 {
    let aligned = info.dwExtraInfo as u64;
    if blocks(injected, aligned) {
        return aligned;
    }
    #[cfg(feature = "simulated-physical")]
    {
        let nonce = crate::classify::current_run_nonce();
        if nonce != 0 && mouse_bytes_carry_nonce(info, nonce) {
            return nonce;
        }
    }
    let packed = unsafe {
        let raw = (info as *const MSLLHOOKSTRUCT).cast::<u8>();
        std::ptr::read_unaligned(raw.add(20).cast::<u64>())
    };
    if packed != aligned && blocks(injected, packed) {
        packed
    } else {
        aligned
    }
}

#[cfg(feature = "simulated-physical")]
fn mouse_bytes_carry_nonce(info: &MSLLHOOKSTRUCT, nonce: u64) -> bool {
    let mut raw = [0u8; 32];
    unsafe {
        std::ptr::copy_nonoverlapping(
            (info as *const MSLLHOOKSTRUCT).cast::<u8>(),
            raw.as_mut_ptr(),
            raw.len(),
        );
    }
    let needle = nonce.to_le_bytes();
    (0..=24).any(|start| raw[start..start + 8] == needle)
}

fn note_mouse_sample(shared: &Shared, info: &MSLLHOOKSTRUCT) {
    shared
        .mouse_seen
        .fetch_add(1, std::sync::atomic::Ordering::Release);
    shared
        .mouse_flags
        .store(info.flags, std::sync::atomic::Ordering::Release);
    shared.mouse_extra.store(
        info.dwExtraInfo as u64,
        std::sync::atomic::Ordering::Release,
    );
    let mut raw = [0u8; 32];
    unsafe {
        std::ptr::copy_nonoverlapping(
            (info as *const MSLLHOOKSTRUCT).cast::<u8>(),
            raw.as_mut_ptr(),
            raw.len(),
        );
    }
    let (chunks, _) = raw.as_chunks::<8>();
    for (word, chunk) in shared.mouse_words.iter().zip(chunks) {
        word.store(
            u64::from_le_bytes(*chunk),
            std::sync::atomic::Ordering::Release,
        );
    }
}

/// Last mouse event the hook observed. Zero when no session is armed.
pub fn last_mouse_hook_sample() -> MouseHookSample {
    let Some(shared) = session_ptr() else {
        return MouseHookSample::default();
    };
    MouseHookSample {
        seen: shared.mouse_seen.load(std::sync::atomic::Ordering::Acquire),
        blocked: shared
            .mouse_blocked
            .load(std::sync::atomic::Ordering::Acquire),
        flags: shared
            .mouse_flags
            .load(std::sync::atomic::Ordering::Acquire),
        extra: shared
            .mouse_extra
            .load(std::sync::atomic::Ordering::Acquire),
        words: [
            shared.mouse_words[0].load(std::sync::atomic::Ordering::Acquire),
            shared.mouse_words[1].load(std::sync::atomic::Ordering::Acquire),
            shared.mouse_words[2].load(std::sync::atomic::Ordering::Acquire),
            shared.mouse_words[3].load(std::sync::atomic::Ordering::Acquire),
        ],
    }
}

/// Bytes the mouse hook copied out of `MSLLHOOKSTRUCT`, plus the counters.
#[derive(Clone, Copy, Debug, Default)]
pub struct MouseHookSample {
    pub seen: u64,
    pub blocked: u64,
    pub flags: u32,
    pub extra: u64,
    pub words: [u64; 4],
}

fn note_allowed_cursor(shared: &Shared, x: i32, y: i32) {
    shared
        .cursor_x
        .store(x, std::sync::atomic::Ordering::Release);
    shared
        .cursor_y
        .store(y, std::sync::atomic::Ordering::Release);
    shared
        .cursor_gen
        .fetch_add(1, std::sync::atomic::Ordering::Release);
}

fn cursor_now() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()? };
    Some((point.x, point.y))
}

/// Put the cursor back after a swallowed move. Runs on the window thread.
pub(crate) fn restore_cursor(shared: &Shared) {
    let snap = shared.snap_gen.load(std::sync::atomic::Ordering::Acquire);
    let generation = shared.cursor_gen.load(std::sync::atomic::Ordering::Acquire);
    if snap == 0 || snap != generation {
        return;
    }
    let x = shared.cursor_x.load(std::sync::atomic::Ordering::Acquire);
    let y = shared.cursor_y.load(std::sync::atomic::Ordering::Acquire);
    unsafe {
        let _ = SetCursorPos(x, y);
    }
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
