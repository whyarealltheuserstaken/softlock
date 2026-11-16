//! The measurement calls live in `probe.m`. This module only copies the result across.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::report::{
    CaptureReport, TapBits, TapReport, WindowReport, capture_detail, capture_opened,
    capture_unavailable, prompt_token, tap_from_bits,
};

#[repr(C)]
struct WindowOut {
    ordered_front: i32,
    key_window: i32,
    key_received: i32,
    post_location: [u8; 64],
    error: [u8; 768],
}

#[repr(C)]
struct TapOut {
    created: i32,
    callback_fired: i32,
    accessibility_trusted: i32,
    listen_preflight: i32,
    prompt: i32,
    detail: [u8; 768],
}

#[repr(C)]
struct CaptureOut {
    opened: i32,
    width: i32,
    height: i32,
    prompt: i32,
    detail: [u8; 768],
}

#[repr(C)]
struct SessionOut {
    uid: i32,
    pid: i32,
    graphic: i32,
    status: i32,
    error: [u8; 256],
}

unsafe extern "C" {
    fn macos_prove_window(out: *mut WindowOut) -> i32;
    fn macos_prove_tap(location: *const c_char, out: *mut TapOut) -> i32;
    fn macos_prove_cg_window_list(out: *mut CaptureOut) -> i32;
    fn macos_prove_cg_display(out: *mut CaptureOut) -> i32;
    fn macos_prove_screen_capture_kit(out: *mut CaptureOut) -> i32;
    fn macos_prompt_code() -> i32;
    fn macos_session_info(out: *mut SessionOut) -> i32;
}

pub struct SessionInfo {
    pub graphic: bool,
    pub status: i32,
}

pub fn prove_window() -> WindowReport {
    let mut out = zero_window();
    let rc = unsafe { macos_prove_window(&mut out) };
    if rc != 0 {
        return WindowReport {
            ordered_front: false,
            key_window: false,
            key_received: false,
            post_location: String::new(),
            error: Some("NSWindow proof was not called".to_string()),
        };
    }
    let ordered_front = out.ordered_front != 0;
    let key_received = out.key_received != 0;
    let error = text(&out.error);
    WindowReport {
        ordered_front,
        key_window: out.key_window != 0,
        key_received,
        post_location: text(&out.post_location),
        error: if (ordered_front && key_received) || error.is_empty() {
            None
        } else {
            Some(error)
        },
    }
}

pub fn prove_tap(location: &str, privilege: &str) -> TapReport {
    let Ok(location_c) = CString::new(location) else {
        return null_tap(location, privilege, "CGEventTapCreate was not called");
    };
    let mut out = zero_tap();
    let rc = unsafe { macos_prove_tap(location_c.as_ptr(), &mut out) };
    let _detail = text(&out.detail);
    if rc != 0 {
        return null_tap(location, privilege, "CGEventTapCreate was not called");
    }
    tap_from_bits(
        location,
        privilege,
        TapBits {
            created: out.created != 0,
            callback_fired: out.callback_fired != 0,
            accessibility_trusted: flag(out.accessibility_trusted),
            listen_preflight: flag(out.listen_preflight),
            prompt: prompt_token(out.prompt),
        },
    )
}

pub fn prove_cg_window_list() -> CaptureReport {
    map_capture("CGWindowListCreateImage", macos_prove_cg_window_list)
}

pub fn prove_cg_display() -> CaptureReport {
    map_capture("CGDisplayCreateImage", macos_prove_cg_display)
}

pub fn prove_screen_capture_kit() -> CaptureReport {
    map_capture("ScreenCaptureKit", macos_prove_screen_capture_kit)
}

pub fn prompt_after_capture(detail: &str) -> String {
    capture_detail(detail, prompt_token(unsafe { macos_prompt_code() }))
}

pub fn session_info() -> SessionInfo {
    let mut out = SessionOut {
        uid: 0,
        pid: 0,
        graphic: 0,
        status: -1,
        error: [0; 256],
    };
    let rc = unsafe { macos_session_info(&mut out) };
    let _observed = (out.uid, out.pid, text(&out.error));
    if rc != 0 {
        return SessionInfo {
            graphic: false,
            status: -1,
        };
    }
    SessionInfo {
        graphic: out.graphic != 0,
        status: out.status,
    }
}

fn map_capture(api: &str, prove: unsafe extern "C" fn(*mut CaptureOut) -> i32) -> CaptureReport {
    let mut out = zero_capture();
    let rc = unsafe { prove(&mut out) };
    if rc != 0 {
        return capture_unavailable(api, "probe was not called");
    }
    if out.opened != 0 {
        return capture_opened(api, out.width, out.height);
    }
    let detail = text(&out.detail);
    let detail = if detail.is_empty() {
        "returned NULL".to_string()
    } else {
        detail
    };
    capture_unavailable(api, &capture_detail(&detail, prompt_token(out.prompt)))
}

fn null_tap(location: &str, privilege: &str, detail: &str) -> TapReport {
    let mut tap = tap_from_bits(
        location,
        privilege,
        TapBits {
            created: false,
            callback_fired: false,
            accessibility_trusted: None,
            listen_preflight: None,
            prompt: None,
        },
    );
    tap.error = Some(format!("{detail}; TCC AXIsProcessTrusted=unknown"));
    tap
}

fn text(buf: &[u8]) -> String {
    CStr::from_bytes_until_nul(buf)
        .map(|value| value.to_string_lossy().trim().to_string())
        .unwrap_or_default()
}

fn flag(value: i32) -> Option<bool> {
    match value {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    }
}

fn zero_window() -> WindowOut {
    WindowOut {
        ordered_front: 0,
        key_window: 0,
        key_received: 0,
        post_location: [0; 64],
        error: [0; 768],
    }
}

fn zero_tap() -> TapOut {
    TapOut {
        created: 0,
        callback_fired: 0,
        accessibility_trusted: -1,
        listen_preflight: -1,
        prompt: 0,
        detail: [0; 768],
    }
}

fn zero_capture() -> CaptureOut {
    CaptureOut {
        opened: 0,
        width: 0,
        height: 0,
        prompt: 0,
        detail: [0; 768],
    }
}
