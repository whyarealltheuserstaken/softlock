//! Measurement calls live in `probe.m`. This module copies the result across.

use std::ffi::CString;
use std::os::raw::c_char;
use std::path::Path;

#[repr(C)]
pub struct WaitOut {
    pub saw: i32,
    pub state_id: i64,
    pub source_pid: i64,
    pub event_type: i64,
    pub disable_type: i64,
    pub error: [u8; 256],
}

#[repr(C)]
pub struct InventoryOut {
    pub keyboards: i32,
    pub pointing: i32,
    pub trackpad: i32,
    pub error: i32,
}

pub struct WaitResult {
    pub saw: bool,
    pub state_id: i64,
    pub source_pid: i64,
    pub disable_type: i64,
    pub error: Option<String>,
}

pub struct Inventory {
    pub keyboards: i32,
    pub pointing: i32,
    pub trackpad: i32,
}

unsafe extern "C" {
    fn m05_hw_model(dest: *mut c_char, cap: usize) -> i32;
    fn m05_tap_create(error: *mut c_char, cap: usize) -> i32;
    fn m05_tap_release() -> i32;
    fn m05_tap_released() -> i32;
    fn m05_clear_wait() -> i32;
    fn m05_wait_saw() -> i32;
    fn m05_last_event(out: *mut WaitOut) -> i32;
    fn m05_tap_wait(kind: *const c_char, timeout_ms: i32, out: *mut WaitOut) -> i32;
    fn m05_last_disable() -> i32;
    fn m05_hid_inventory(out: *mut InventoryOut) -> i32;
    fn m05_secure_enabled() -> i32;
    fn m05_classify_target(dir: *const c_char) -> i32;
    fn m05_classify_post(source: *const c_char, dir: *const c_char) -> i32;
    fn m05_read_target_count(dir: *const c_char) -> i32;
    fn m05_pump(millis: i32) -> i32;
}

fn cstr(text: &str) -> CString {
    CString::new(text).unwrap_or_else(|_| CString::new("").unwrap())
}

fn optional_text(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    if end == 0 {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

pub fn hw_model() -> String {
    let mut buf = [0u8; 128];
    unsafe {
        m05_hw_model(buf.as_mut_ptr() as *mut c_char, buf.len());
    }
    optional_text(&buf).unwrap_or_default()
}

pub fn tap_create() -> Result<(), String> {
    let mut error = [0u8; 256];
    let created = unsafe { m05_tap_create(error.as_mut_ptr() as *mut c_char, error.len()) };
    if created != 0 {
        Ok(())
    } else {
        Err(optional_text(&error).unwrap_or_else(|| "CGEventTapCreate returned NULL".into()))
    }
}

pub fn tap_release() {
    unsafe {
        m05_tap_release();
    }
}

pub fn tap_released() -> bool {
    unsafe { m05_tap_released() != 0 }
}

pub fn clear_wait() {
    unsafe {
        m05_clear_wait();
    }
}

pub fn wait_saw() -> bool {
    unsafe { m05_wait_saw() != 0 }
}

pub fn last_event() -> WaitResult {
    let mut out = WaitOut {
        saw: 0,
        state_id: 0,
        source_pid: 0,
        event_type: 0,
        disable_type: 0,
        error: [0; 256],
    };
    unsafe {
        m05_last_event(&mut out);
    }
    WaitResult {
        saw: out.saw != 0,
        state_id: out.state_id,
        source_pid: out.source_pid,
        disable_type: out.disable_type,
        error: optional_text(&out.error),
    }
}

pub fn tap_wait(kind: &str, timeout_ms: i32) -> WaitResult {
    let kind = cstr(kind);
    let mut out = WaitOut {
        saw: 0,
        state_id: 0,
        source_pid: 0,
        event_type: 0,
        disable_type: 0,
        error: [0; 256],
    };
    unsafe {
        m05_tap_wait(kind.as_ptr(), timeout_ms, &mut out);
    }
    WaitResult {
        saw: out.saw != 0,
        state_id: out.state_id,
        source_pid: out.source_pid,
        disable_type: out.disable_type,
        error: optional_text(&out.error),
    }
}

pub fn last_disable() -> i64 {
    i64::from(unsafe { m05_last_disable() })
}

pub fn hid_inventory() -> Inventory {
    let mut out = InventoryOut {
        keyboards: 0,
        pointing: 0,
        trackpad: -1,
        error: 0,
    };
    unsafe {
        m05_hid_inventory(&mut out);
    }
    Inventory {
        keyboards: out.keyboards,
        pointing: out.pointing,
        trackpad: out.trackpad,
    }
}

pub fn secure_enabled() -> Option<bool> {
    match unsafe { m05_secure_enabled() } {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    }
}

pub fn classify_target(dir: &Path) -> i32 {
    let dir = cstr(&dir.to_string_lossy());
    unsafe { m05_classify_target(dir.as_ptr()) }
}

pub fn classify_post(source: &str, dir: &Path) -> i32 {
    let source = cstr(source);
    let dir = cstr(&dir.to_string_lossy());
    unsafe { m05_classify_post(source.as_ptr(), dir.as_ptr()) }
}

pub fn read_target_count(dir: &Path) -> i32 {
    let dir = cstr(&dir.to_string_lossy());
    unsafe { m05_read_target_count(dir.as_ptr()) }
}

pub fn pump(millis: i32) {
    unsafe {
        m05_pump(millis);
    }
}

pub struct TapGuard;

impl TapGuard {
    pub fn create() -> Result<Self, String> {
        tap_create()?;
        Ok(Self)
    }
}

impl Drop for TapGuard {
    fn drop(&mut self) {
        tap_release();
    }
}
