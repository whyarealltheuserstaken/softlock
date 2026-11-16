//! Low-level keyboard hook. The fast path does no I/O and no allocation.
//! The blocking wait runs only when the removal probe is armed.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::Sleep;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, WM_KEYDOWN,
};

use crate::verdict::{CountKind, HookAction, HookEvent, classify};

/// Set in the SYSTEM process only. Other hooks pass marker events on.
pub static SWALLOW_MARKERS: AtomicBool = AtomicBool::new(false);
pub static SLOW_ARM: AtomicBool = AtomicBool::new(false);
pub static SLOW_MS: AtomicU32 = AtomicU32::new(0);
pub static SLOW_ENTERED: AtomicU32 = AtomicU32::new(0);
pub static SLOW_RETURNED: AtomicU32 = AtomicU32::new(0);
pub static VIS_SEEN: AtomicU64 = AtomicU64::new(0);
pub static VIS_INJECTED: AtomicU64 = AtomicU64::new(0);
pub static LOAD_SEEN: AtomicU64 = AtomicU64::new(0);
pub static MARKER_SEEN: AtomicU64 = AtomicU64::new(0);
pub static WINDOW_KEYS: AtomicU64 = AtomicU64::new(0);

pub fn reset_counts() {
    VIS_SEEN.store(0, Ordering::Relaxed);
    VIS_INJECTED.store(0, Ordering::Relaxed);
    LOAD_SEEN.store(0, Ordering::Relaxed);
    MARKER_SEEN.store(0, Ordering::Relaxed);
    WINDOW_KEYS.store(0, Ordering::Relaxed);
}

pub fn slow_state() -> (bool, bool) {
    (
        SLOW_ENTERED.load(Ordering::Relaxed) == 1,
        SLOW_RETURNED.load(Ordering::Relaxed) == 1,
    )
}

pub fn counts() -> (u64, u64, u64, u64, u64) {
    (
        VIS_SEEN.load(Ordering::Relaxed),
        VIS_INJECTED.load(Ordering::Relaxed),
        LOAD_SEEN.load(Ordering::Relaxed),
        MARKER_SEEN.load(Ordering::Relaxed),
        WINDOW_KEYS.load(Ordering::Relaxed),
    )
}

pub unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code < 0 {
            return CallNextHookEx(None, code, wparam, lparam);
        }
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let action = classify(HookEvent {
            extra: info.dwExtraInfo,
            key_down: wparam.0 == WM_KEYDOWN as usize,
            injected: info.flags.contains(LLKHF_INJECTED),
            slow_armed: SLOW_ARM.load(Ordering::Acquire),
            slow_ms: SLOW_MS.load(Ordering::Relaxed),
        });
        if !SWALLOW_MARKERS.load(Ordering::Acquire)
            && matches!(action, HookAction::Slow { .. } | HookAction::Swallow { .. })
        {
            return CallNextHookEx(None, code, wparam, lparam);
        }
        match action {
            HookAction::Slow { sleep_ms } => {
                SLOW_ARM.store(false, Ordering::Release);
                SLOW_ENTERED.store(1, Ordering::Release);
                Sleep(sleep_ms);
                SLOW_RETURNED.store(1, Ordering::Release);
                LRESULT(1)
            }
            HookAction::Swallow { count_marker } => {
                if count_marker {
                    MARKER_SEEN.fetch_add(1, Ordering::Relaxed);
                }
                LRESULT(1)
            }
            HookAction::PassCount { kind, injected } => {
                match kind {
                    CountKind::Visible => {
                        VIS_SEEN.fetch_add(1, Ordering::Relaxed);
                        if injected {
                            VIS_INJECTED.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    CountKind::Load => {
                        LOAD_SEEN.fetch_add(1, Ordering::Relaxed);
                    }
                }
                CallNextHookEx(None, code, wparam, lparam)
            }
            HookAction::Pass => CallNextHookEx(None, code, wparam, lparam),
        }
    }
}
