//! Measurement calls live in `probe.m`. This module copies the result across.

use std::os::raw::c_char;

use crate::args::REASON_PREFIX;
use crate::report::AssertionNames;

#[repr(C)]
struct NamesOut {
    prevent_user_idle_system_sleep: i32,
    prevent_user_idle_display_sleep: i32,
    no_idle_sleep: i32,
    no_display_sleep: i32,
}

#[repr(C)]
struct AssertOut {
    created: i32,
    released: i32,
    system_id: u32,
    display_id: u32,
    reason: [u8; 128],
    error: [u8; 256],
}

#[repr(C)]
struct WindowOut {
    ordered_front: i32,
    display_count: i32,
    error: [u8; 256],
}

#[repr(C)]
struct TimerOut {
    ticks: u64,
    error: [u8; 256],
}

unsafe extern "C" {
    fn m07_assertion_names(out: *mut NamesOut) -> i32;
    fn m07_assertions_create(reason: *const c_char, out: *mut AssertOut) -> i32;
    fn m07_assertions_release(system_id: u32, display_id: u32) -> i32;
    fn m07_display_count() -> i32;
    fn m07_overlay_open(out: *mut WindowOut) -> i32;
    fn m07_overlay_close() -> i32;
    fn m07_overlay_escape_seen() -> i32;
    fn m07_overlay_pump(millis: u32);
    fn m07_timer_open(out: *mut WindowOut) -> i32;
    fn m07_timer_close() -> i32;
    fn m07_timer_tick(out: *mut TimerOut) -> i32;
    fn m07_hw_model(dest: *mut c_char, cap: usize) -> i32;
}

pub fn assertion_names() -> AssertionNames {
    let mut out = NamesOut {
        prevent_user_idle_system_sleep: 0,
        prevent_user_idle_display_sleep: 0,
        no_idle_sleep: 0,
        no_display_sleep: 0,
    };
    unsafe {
        m07_assertion_names(&mut out);
    }
    AssertionNames {
        prevent_user_idle_system_sleep: out.prevent_user_idle_system_sleep != 0,
        prevent_user_idle_display_sleep: out.prevent_user_idle_display_sleep != 0,
        no_idle_sleep: out.no_idle_sleep != 0,
        no_display_sleep: out.no_display_sleep != 0,
    }
}

pub struct HeldAssertions {
    pub created: bool,
    pub released: bool,
    pub reason: String,
    pub error: Option<String>,
    system_id: u32,
    display_id: u32,
}

impl HeldAssertions {
    pub fn create() -> Self {
        let reason = format!("{REASON_PREFIX}-{}", std::process::id());
        let c_reason = std::ffi::CString::new(reason.clone()).expect("reason");
        let mut out = AssertOut {
            created: 0,
            released: 0,
            system_id: 0,
            display_id: 0,
            reason: [0; 128],
            error: [0; 256],
        };
        unsafe {
            m07_assertions_create(c_reason.as_ptr(), &mut out);
        }
        let error = text(&out.error);
        HeldAssertions {
            created: out.created != 0,
            released: false,
            reason: {
                let copied = text(&out.reason);
                if copied.is_empty() { reason } else { copied }
            },
            error: if error.is_empty() { None } else { Some(error) },
            system_id: out.system_id,
            display_id: out.display_id,
        }
    }

    pub fn release(&mut self) {
        if self.system_id == 0 && self.display_id == 0 {
            self.released = true;
            return;
        }
        let rc = unsafe { m07_assertions_release(self.system_id, self.display_id) };
        self.released = rc == 0;
        self.system_id = 0;
        self.display_id = 0;
    }
}

impl Drop for HeldAssertions {
    fn drop(&mut self) {
        if !self.released {
            self.release();
        }
    }
}

pub struct Overlay {
    pub ordered_front: bool,
    pub display_count: i32,
    pub error: Option<String>,
}

impl Overlay {
    pub fn open() -> Self {
        let mut out = WindowOut {
            ordered_front: 0,
            display_count: 0,
            error: [0; 256],
        };
        unsafe {
            m07_overlay_open(&mut out);
        }
        Overlay {
            ordered_front: out.ordered_front != 0,
            display_count: out.display_count,
            error: optional_text(&out.error),
        }
    }

    pub fn pump(millis: u32) {
        unsafe {
            m07_overlay_pump(millis);
        }
    }

    pub fn escape_seen() -> bool {
        unsafe { m07_overlay_escape_seen() != 0 }
    }

    pub fn close() {
        unsafe {
            m07_overlay_close();
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        Overlay::close();
    }
}

pub struct TimerWindow {
    pub ordered_front: bool,
    pub error: Option<String>,
}

impl TimerWindow {
    pub fn open() -> Self {
        let mut out = WindowOut {
            ordered_front: 0,
            display_count: 0,
            error: [0; 256],
        };
        unsafe {
            m07_timer_open(&mut out);
        }
        TimerWindow {
            ordered_front: out.ordered_front != 0,
            error: optional_text(&out.error),
        }
    }

    pub fn tick() -> Result<u64, String> {
        let mut out = TimerOut {
            ticks: 0,
            error: [0; 256],
        };
        let rc = unsafe { m07_timer_tick(&mut out) };
        if rc != 0 {
            return Err(
                optional_text(&out.error).unwrap_or_else(|| "timer tick failed".to_string())
            );
        }
        Ok(out.ticks)
    }

    pub fn close() {
        unsafe {
            m07_timer_close();
        }
    }
}

impl Drop for TimerWindow {
    fn drop(&mut self) {
        TimerWindow::close();
    }
}

pub fn display_count() -> i32 {
    unsafe { m07_display_count() }
}

pub fn hw_model() -> String {
    let mut buf = [0u8; 64];
    unsafe {
        m07_hw_model(buf.as_mut_ptr() as *mut c_char, buf.len());
    }
    let text = text(&buf);
    if crate::scrub::contains_serial(&text) {
        String::new()
    } else {
        text
    }
}

fn text(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn optional_text(bytes: &[u8]) -> Option<String> {
    let value = text(bytes);
    if value.is_empty() { None } else { Some(value) }
}
