//! Low-level hooks. The callback does no allocation and no I/O.
//! Unhook runs on drop, on panic, on a crash, and from the watchdog.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use softlock_hello_spike::{Action, InputOrigin, ModifierKeys, on_key, on_pointer};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Diagnostics::Debug::{
    EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS, SetUnhandledExceptionFilter,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};
use windows::core::{BOOL, PCWSTR};

pub static HELLO_RUNNING: AtomicBool = AtomicBool::new(false);
pub static STOP: AtomicBool = AtomicBool::new(false);

static PASS_PHYSICAL: AtomicBool = AtomicBool::new(false);
static ESCAPE: AtomicBool = AtomicBool::new(false);
static INJECTED_PASSES: AtomicU32 = AtomicU32::new(0);
static KEYBOARD: AtomicIsize = AtomicIsize::new(0);
static MOUSE: AtomicIsize = AtomicIsize::new(0);

struct HookMods(UnsafeCell<ModifierKeys>);

// SAFETY: low-level hook callbacks are serialized on the thread that
// installed them, and no other thread reads this state.
unsafe impl Sync for HookMods {}

static MODS: HookMods = HookMods(UnsafeCell::new(ModifierKeys::new()));

const LLKHF_INJECTED: u32 = 0x0010;
const LLMHF_INJECTED: u32 = 0x0001;

pub struct HookGuard;

impl Drop for HookGuard {
    fn drop(&mut self) {
        unhook_global();
    }
}

pub fn install() -> windows::core::Result<HookGuard> {
    // SAFETY: the module handle is the current executable and is not freed.
    let module = unsafe { GetModuleHandleW(PCWSTR::null())? };
    let instance = windows::Win32::Foundation::HINSTANCE(module.0);
    // SAFETY: the callback is a static function and the hook is removed by HookGuard.
    let keyboard =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(instance), 0)? };
    KEYBOARD.store(keyboard.0 as isize, Ordering::SeqCst);
    let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(instance), 0) };
    let mouse = match mouse {
        Ok(handle) => handle,
        Err(error) => {
            unhook_global();
            return Err(error);
        }
    };
    MOUSE.store(mouse.0 as isize, Ordering::SeqCst);
    Ok(HookGuard)
}

pub fn unhook_global() {
    unhook_one(&KEYBOARD);
    unhook_one(&MOUSE);
}

pub fn close_passthrough() {
    PASS_PHYSICAL.store(false, Ordering::SeqCst);
}

pub fn set_passthrough(open: bool) {
    PASS_PHYSICAL.store(open, Ordering::SeqCst);
}

pub fn passthrough_is_open() -> bool {
    HELLO_RUNNING.load(Ordering::SeqCst) && PASS_PHYSICAL.load(Ordering::SeqCst)
}

pub fn escape_pressed() -> bool {
    ESCAPE.load(Ordering::SeqCst)
}

pub fn injected_passes() -> u32 {
    INJECTED_PASSES.load(Ordering::SeqCst)
}

pub fn raise_priority() -> windows::core::Result<()> {
    // SAFETY: the handle is the current thread pseudo-handle.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) }
}

pub fn watchdog(limit: Duration) {
    thread::spawn(move || {
        thread::sleep(limit);
        eprintln!("block deadline reached; removing the hook");
        unhook_global();
        STOP.store(true, Ordering::SeqCst);
        thread::sleep(Duration::from_secs(2));
        eprintln!("deadline backstop; exiting");
        std::process::exit(3);
    });
}

pub fn install_crash_handlers() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        unhook_global();
        STOP.store(true, Ordering::SeqCst);
        previous(info);
    }));
    // SAFETY: the filter only removes the hook, then continues the search.
    unsafe {
        SetUnhandledExceptionFilter(Some(crash_filter));
    }
}

pub fn install_console_handler() {
    // SAFETY: the handler only removes the hook and asks the loop to stop.
    unsafe {
        let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(Some(console_ctrl), true);
    }
}

fn unhook_one(slot: &AtomicIsize) {
    let raw = slot.swap(0, Ordering::SeqCst);
    if raw != 0 {
        // SAFETY: the pointer was returned by SetWindowsHookExW and is removed once.
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(raw as *mut std::ffi::c_void));
        }
    }
}

unsafe extern "system" fn crash_filter(_info: *const EXCEPTION_POINTERS) -> i32 {
    unhook_global();
    STOP.store(true, Ordering::SeqCst);
    EXCEPTION_CONTINUE_SEARCH
}

unsafe extern "system" fn console_ctrl(_kind: u32) -> BOOL {
    unhook_global();
    STOP.store(true, Ordering::SeqCst);
    BOOL(1)
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        // SAFETY: forwarding a hook the system asked us not to handle.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    // SAFETY: for a low-level keyboard hook, lParam points at KBDLLHOOKSTRUCT.
    let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let origin = if info.flags.0 & LLKHF_INJECTED != 0 {
        InputOrigin::Injected
    } else {
        InputOrigin::Physical
    };
    let vk = u16::try_from(info.vkCode).unwrap_or(u16::MAX);
    let down = wparam.0 == WM_KEYDOWN as usize || wparam.0 == WM_SYSKEYDOWN as usize;
    // SAFETY: only this hook thread uses MODS, and callbacks are not nested.
    let mods = unsafe { &mut *MODS.0.get() };
    let allow = passthrough_is_open();
    let action = on_key(mods, allow, origin, vk, down);
    apply_action(origin, action, code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        // SAFETY: forwarding a hook the system asked us not to handle.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    // SAFETY: for a low-level mouse hook, lParam points at MSLLHOOKSTRUCT.
    let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
    let origin = if info.flags & LLMHF_INJECTED != 0 {
        InputOrigin::Injected
    } else {
        InputOrigin::Physical
    };
    let action = on_pointer(passthrough_is_open(), origin);
    apply_action(origin, action, code, wparam, lparam)
}

fn apply_action(
    origin: InputOrigin,
    action: Action,
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if origin == InputOrigin::Injected && action == Action::Pass {
        INJECTED_PASSES.fetch_add(1, Ordering::Relaxed);
    }
    if action == Action::Escape {
        ESCAPE.store(true, Ordering::SeqCst);
        STOP.store(true, Ordering::SeqCst);
    }
    if action == Action::Pass {
        // SAFETY: forwarding to the next hook with the original arguments.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    } else {
        LRESULT(1)
    }
}
