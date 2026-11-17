//! The measurement calls live in `probe.m`. This module only copies the result across.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::classify::{disable_event_name, fail_closed_end};
use crate::labels::SCREEN_CAPTURE_ENTRY;
use crate::report::{
    CaptureReport, ClassifyReport, MarkerObservation, PostedObservation, SharingWindowReport,
    SlowObservation, TapBits, TapReport, WindowReport, active_tap, capture_detail, capture_opened,
    capture_unavailable, prompt_token, tap_from_bits,
};

#[repr(C)]
struct WindowOut {
    ordered_front: i32,
    key_window: i32,
    key_received: i32,
    post_location: [u8; 64],
    error: [u8; 768],
    stage: [u8; 64],
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
    nonblack: i32,
    marker: i32,
    control: i32,
    prompt: i32,
    detail: [u8; 768],
}

#[repr(C)]
struct ActiveOut {
    created: i32,
    deleted: i32,
    listen_saw: i32,
    control_saw: i32,
    accessibility_trusted: i32,
    listen_preflight: i32,
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

#[repr(C)]
struct PostRow {
    state_id: i64,
    source_pid: i64,
    poster_pid: i64,
    window_received: i32,
    tap_saw: i32,
    same_process: i32,
    fields_read: i32,
    source: [u8; 64],
    error: [u8; 256],
}

#[repr(C)]
struct M03Out {
    posts: [PostRow; 6],
    post_count: i32,
    pad: i32,
    sleep_ms: i64,
    disable_wait_ms: i64,
    dead_ms: i64,
    healthy_ms: i64,
    disable_type: i64,
    key_reached_during: i32,
    key_reached_after: i32,
    dead_misses: i32,
    reenabled: i32,
    healthy_seen: i32,
    no_tap: i32,
    disable_event: [u8; 64],
    slow_error: [u8; 256],
    marker_error: [u8; 256],
    error: [u8; 512],
}

#[repr(C)]
struct SharingWindowOut {
    ordered_front: i32,
    ignores_mouse_events: i32,
    shielding_called: i32,
    window_area: i32,
    level: [u8; 64],
    collection: [u8; 128],
    sharing_default: [u8; 64],
    display_image_api: [u8; 128],
    error: [u8; 768],
}

#[repr(C)]
struct SharingPresenceOut {
    visible: i32,
    occlusion: i32,
    listed: i32,
    detail: [u8; 256],
}

unsafe extern "C" {
    fn macos_window_open(out: *mut WindowOut) -> i32;
    fn macos_sharing_open(out: *mut SharingWindowOut) -> i32;
    fn macos_sharing_set_none(buf: *mut c_char, len: usize) -> i32;
    fn macos_sharing_control_open() -> i32;
    fn macos_sharing_presence(out: *mut SharingPresenceOut) -> i32;
    fn macos_sharing_order_out() -> i32;
    fn macos_sharing_close() -> i32;
    fn macos_prove_sc_stream(out: *mut CaptureOut) -> i32;
    fn macos_window_close() -> i32;
    fn macos_prove_window(out: *mut WindowOut) -> i32;
    fn macos_prove_tap(location: *const c_char, out: *mut TapOut) -> i32;
    fn macos_prove_active_tap(out: *mut ActiveOut) -> i32;
    fn macos_prove_cg_window_list(out: *mut CaptureOut) -> i32;
    fn macos_prove_cg_display(out: *mut CaptureOut) -> i32;
    fn macos_prove_screen_capture_kit(out: *mut CaptureOut) -> i32;
    fn macos_inspect_png(
        path: *const c_char,
        nonblack: *mut i32,
        marker: *mut i32,
        control: *mut i32,
    ) -> i32;
    fn macos_prompt_code() -> i32;
    fn macos_session_info(out: *mut SessionOut) -> i32;
    fn macos_classify_post(source: *const c_char, dir: *const c_char) -> i32;
    fn macos_classify_target(dir: *const c_char) -> i32;
    fn macos_prove_m03(exe: *const c_char, dir: *const c_char, out: *mut M03Out) -> i32;
}

pub struct SessionInfo {
    pub graphic: bool,
    pub status: i32,
}

pub fn prove_window() -> WindowReport {
    let mut out = zero_window();
    let rc = unsafe { macos_prove_window(&mut out) };
    window_from_out(rc, &out)
}

pub fn window_open() -> WindowReport {
    let mut out = zero_window();
    let rc = unsafe { macos_window_open(&mut out) };
    window_from_out(rc, &out)
}

pub fn window_close() {
    unsafe { macos_window_close() };
}

pub fn sharing_open() -> SharingWindowReport {
    let mut out = SharingWindowOut {
        ordered_front: 0,
        ignores_mouse_events: 0,
        shielding_called: 0,
        window_area: 0,
        level: [0; 64],
        collection: [0; 128],
        sharing_default: [0; 64],
        display_image_api: [0; 128],
        error: [0; 768],
    };
    let rc = unsafe { macos_sharing_open(&mut out) };
    let error = text(&out.error);
    SharingWindowReport {
        ordered_front: out.ordered_front != 0,
        level: text(&out.level),
        collection: text(&out.collection),
        ignores_mouse_events: out.ignores_mouse_events != 0,
        shielding_called: out.shielding_called != 0,
        sharing_default: text(&out.sharing_default),
        sharing_none: String::new(),
        window_area: out.window_area,
        visible_after_none: false,
        occlusion_after_none: false,
        listed_on_screen: false,
        display_image_api: text(&out.display_image_api),
        screen_capture_entry: SCREEN_CAPTURE_ENTRY.to_string(),
        error: if rc != 0 {
            Some("NSWindow sharing probe was not called".to_string())
        } else if error.is_empty() {
            None
        } else {
            Some(error)
        },
    }
}

pub fn sharing_set_none() -> String {
    let mut buf = [0u8; 64];
    let rc = unsafe { macos_sharing_set_none(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return String::new();
    }
    text(&buf)
}

pub struct SharingPresence {
    pub visible: bool,
    pub occlusion: bool,
    pub listed: bool,
    pub detail: String,
}

pub fn sharing_control_open() -> bool {
    unsafe { macos_sharing_control_open() == 0 }
}

pub fn sharing_presence() -> SharingPresence {
    let mut out = SharingPresenceOut {
        visible: 0,
        occlusion: 0,
        listed: 0,
        detail: [0; 256],
    };
    let rc = unsafe { macos_sharing_presence(&mut out) };
    SharingPresence {
        visible: out.visible != 0,
        occlusion: out.occlusion != 0,
        listed: out.listed != 0,
        detail: if rc != 0 {
            "NSWindow presence was not called".to_string()
        } else {
            text(&out.detail)
        },
    }
}

pub fn sharing_order_out() {
    unsafe { macos_sharing_order_out() };
}

pub fn sharing_close() {
    unsafe { macos_sharing_close() };
}

fn window_from_out(rc: i32, out: &WindowOut) -> WindowReport {
    if rc != 0 {
        return WindowReport {
            ordered_front: false,
            key_window: false,
            key_received: false,
            post_location: String::new(),
            stage: text(&out.stage),
            error: Some("NSWindow proof was not called".to_string()),
        };
    }
    let ordered_front = out.ordered_front != 0;
    let key_received = out.key_received != 0;
    let error = text(&out.error);
    let stage = text(&out.stage);
    WindowReport {
        ordered_front,
        key_window: out.key_window != 0,
        key_received,
        post_location: text(&out.post_location),
        stage: if stage.is_empty() {
            "none".to_string()
        } else {
            stage
        },
        error: if (ordered_front && key_received) || error.is_empty() {
            None
        } else {
            Some(error)
        },
    }
}

pub fn prove_active_tap() -> TapReport {
    let mut out = zero_active();
    let rc = unsafe { macos_prove_active_tap(&mut out) };
    if rc != 0 {
        return active_tap(
            TapBits {
                created: false,
                callback_fired: false,
                accessibility_trusted: None,
                listen_preflight: None,
                prompt: None,
            },
            None,
            None,
            None,
            "kCGEventTapOptionDefault was not called",
        );
    }
    let deleted = match out.deleted {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    };
    let listen_saw = match out.listen_saw {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    };
    let control_saw = match out.control_saw {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    };
    active_tap(
        TapBits {
            created: out.created != 0,
            callback_fired: out.deleted == 1,
            accessibility_trusted: flag(out.accessibility_trusted),
            listen_preflight: flag(out.listen_preflight),
            prompt: prompt_token(out.prompt),
        },
        deleted,
        listen_saw,
        control_saw,
        &text(&out.detail),
    )
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

pub fn prove_sc_stream() -> CaptureReport {
    map_capture("SCStream", macos_prove_sc_stream)
}

pub fn inspect_png(path: &std::path::Path) -> Option<(i32, i32, i32)> {
    let Ok(path) = CString::new(path.to_string_lossy().as_bytes()) else {
        return None;
    };
    let mut nonblack = -1;
    let mut marker = -1;
    let mut control = -1;
    let rc = unsafe { macos_inspect_png(path.as_ptr(), &mut nonblack, &mut marker, &mut control) };
    if rc != 0 || nonblack < 0 || marker < 0 || control < 0 {
        None
    } else {
        Some((nonblack, marker, control))
    }
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
        if out.nonblack < 0 || out.marker < 0 {
            let mut report = capture_opened(api, out.width, out.height, 0, 0);
            report.nonblack_pixels = None;
            report.marker_pixels = None;
            return report;
        }
        let mut report = capture_opened(api, out.width, out.height, out.nonblack, out.marker);
        if out.control >= 0 {
            report.control_pixels = Some(out.control);
        }
        return report;
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
        stage: [0; 64],
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
        nonblack: -1,
        marker: -1,
        control: -1,
        prompt: 0,
        detail: [0; 768],
    }
}

pub fn classify_post(source: &str, dir: &std::path::Path) -> i32 {
    let source = CString::new(source).unwrap_or_else(|_| CString::new("").unwrap());
    let dir = CString::new(dir.to_string_lossy().as_bytes())
        .unwrap_or_else(|_| CString::new("").unwrap());
    unsafe { macos_classify_post(source.as_ptr(), dir.as_ptr()) }
}

pub fn classify_target(dir: &std::path::Path) -> i32 {
    let dir = CString::new(dir.to_string_lossy().as_bytes())
        .unwrap_or_else(|_| CString::new("").unwrap());
    unsafe { macos_classify_target(dir.as_ptr()) }
}

pub fn prove_m03(exe: &std::path::Path, dir: &std::path::Path) -> ClassifyReport {
    let exe = CString::new(exe.to_string_lossy().as_bytes())
        .unwrap_or_else(|_| CString::new("").unwrap());
    let dir = CString::new(dir.to_string_lossy().as_bytes())
        .unwrap_or_else(|_| CString::new("").unwrap());
    let mut out = zero_m03();
    let _rc = unsafe { macos_prove_m03(exe.as_ptr(), dir.as_ptr(), &mut out) };
    let mut posts = Vec::new();
    let count = out.post_count.clamp(0, 6) as usize;
    for row in out.posts.iter().take(count) {
        let error = optional_text(&row.error);
        posts.push(PostedObservation {
            source: text(&row.source),
            poster: if row.same_process == 0 {
                "second-process".to_string()
            } else {
                "same-process".to_string()
            },
            poster_pid: row.poster_pid,
            state_id: row.state_id,
            source_pid: row.source_pid,
            fields_read: row.fields_read != 0,
            tap_saw: row.tap_saw != 0,
            window_received: row.window_received != 0,
            error,
        });
    }
    let mut slow_error = optional_text(&out.slow_error);
    let top = optional_text(&out.error);
    if slow_error.is_none() && out.sleep_ms == 0 {
        slow_error = top.clone();
    }
    ClassifyReport {
        posts,
        slow: SlowObservation {
            disable_event: disable_event_name(out.disable_type).to_string(),
            sleep_ms: out.sleep_ms,
            disable_wait_ms: out.disable_wait_ms,
            key_reached_during: out.key_reached_during != 0,
            key_reached_after: out.key_reached_after != 0,
            reenabled: out.reenabled != 0,
            error: slow_error,
        },
        marker: MarkerObservation {
            dead_ms: out.dead_ms,
            dead_misses: out.dead_misses.max(0) as u32,
            healthy_ms: out.healthy_ms,
            healthy_seen: out.healthy_seen != 0,
            reenabled: out.reenabled != 0,
            error: match optional_text(&out.marker_error) {
                Some(error) => Some(error),
                None if out.dead_ms == 0 => top,
                None => None,
            },
        },
        end_state: fail_closed_end().to_string(),
        tap_status: if out.no_tap != 0 {
            "no-tap".to_string()
        } else {
            "created".to_string()
        },
    }
}

fn optional_text(buf: &[u8]) -> Option<String> {
    let text = text(buf);
    if text.is_empty() { None } else { Some(text) }
}

fn zero_m03() -> M03Out {
    M03Out {
        posts: std::array::from_fn(|_| PostRow {
            state_id: 0,
            source_pid: 0,
            poster_pid: 0,
            window_received: 0,
            tap_saw: 0,
            same_process: 0,
            fields_read: 0,
            source: [0; 64],
            error: [0; 256],
        }),
        post_count: 0,
        pad: 0,
        sleep_ms: 0,
        disable_wait_ms: 0,
        dead_ms: 0,
        healthy_ms: 0,
        disable_type: 0,
        key_reached_during: 0,
        key_reached_after: 0,
        dead_misses: 0,
        reenabled: 0,
        healthy_seen: 0,
        no_tap: 0,
        disable_event: [0; 64],
        slow_error: [0; 256],
        marker_error: [0; 256],
        error: [0; 512],
    }
}

fn zero_active() -> ActiveOut {
    ActiveOut {
        created: 0,
        deleted: -1,
        listen_saw: -1,
        control_saw: -1,
        accessibility_trusted: -1,
        listen_preflight: -1,
        prompt: 0,
        detail: [0; 768],
    }
}
