//! Measurement calls live in `probe.m`. This module copies the result across.

use std::os::raw::c_char;

use crate::claim::{SCREEN_CAPTURE_ENTRY, SCSTREAM_ENTRY};
use crate::report::{CaptureSample, ClickFocus, OverlayWindow};
use crate::text::one_line;

#[repr(C)]
struct OverlayOut {
    ordered_front: i32,
    is_key: i32,
    is_main: i32,
    ignores_mouse_events: i32,
    shielding_called: i32,
    display_count: i32,
    covered_count: i32,
    window_area: i32,
    visible_after_none: i32,
    level: [u8; 64],
    collection: [u8; 128],
    sharing: [u8; 64],
    display_image_api: [u8; 128],
    error: [u8; 256],
}

#[repr(C)]
struct CaptureOut {
    opened: i32,
    width: i32,
    height: i32,
    marker: i32,
    control: i32,
    detail: [u8; 384],
}

#[repr(C)]
struct ClickOut {
    overlay_is_key: i32,
    target_is_key: i32,
    click_count: i32,
    key_count: i32,
    accessibility: i32,
    listen: i32,
    error: [u8; 256],
}

#[repr(C)]
struct PresentOut {
    app_active: i32,
    process_switching: i32,
    force_quit: i32,
    session_termination: i32,
    error: [u8; 256],
}

unsafe extern "C" {
    fn m06_overlay_open(out: *mut OverlayOut) -> i32;
    fn m06_overlay_set_none(out: *mut OverlayOut) -> i32;
    fn m06_overlay_order_out() -> i32;
    fn m06_overlay_close() -> i32;
    fn m06_overlay_sync(out: *mut OverlayOut) -> i32;
    fn m06_overlay_escape_seen() -> i32;
    fn m06_overlay_pump(millis: u32);
    fn m06_overlay_wait_visible(timeout_ms: u32) -> i32;
    fn m06_overlay_wait_hidden(timeout_ms: u32) -> i32;
    fn m06_overlay_set_banner(utf8: *const c_char) -> i32;
    fn m06_screen_capture_access() -> i32;
    fn m06_control_open() -> i32;
    fn m06_target_open(out: *mut ClickOut) -> i32;
    fn m06_target_close() -> i32;
    fn m06_posted_click_focus(out: *mut ClickOut) -> i32;
    fn m06_presentation_apply(out: *mut PresentOut) -> i32;
    fn m06_presentation_clear() -> i32;
    fn m06_capture(api: *const c_char, out: *mut CaptureOut) -> i32;
    fn m06_hw_model(dest: *mut c_char, cap: usize) -> i32;
    fn m06_display_count() -> i32;
}

fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

fn optional_text(bytes: &[u8]) -> Option<String> {
    let value = text(bytes);
    if value.is_empty() { None } else { Some(value) }
}

fn empty_overlay() -> OverlayOut {
    OverlayOut {
        ordered_front: 0,
        is_key: 0,
        is_main: 0,
        ignores_mouse_events: 0,
        shielding_called: 0,
        display_count: 0,
        covered_count: 0,
        window_area: 0,
        visible_after_none: 0,
        level: [0; 64],
        collection: [0; 128],
        sharing: [0; 64],
        display_image_api: [0; 128],
        error: [0; 256],
    }
}

fn overlay_from(out: &OverlayOut, sharing_default: String, sharing_none: String) -> OverlayWindow {
    OverlayWindow {
        ordered_front: out.ordered_front != 0,
        level: text(&out.level),
        collection: text(&out.collection),
        ignores_mouse_events: out.ignores_mouse_events != 0,
        is_key: out.is_key != 0,
        is_main: out.is_main != 0,
        shielding_called: out.shielding_called != 0,
        sharing_default,
        sharing_none,
        window_area: out.window_area,
        display_count: out.display_count,
        covered_count: out.covered_count,
        visible_after_none: out.visible_after_none != 0,
        display_image_api: text(&out.display_image_api),
        screen_capture_entry: SCREEN_CAPTURE_ENTRY.to_string(),
        scstream_entry: SCSTREAM_ENTRY.to_string(),
        error: optional_text(&out.error),
    }
}

pub struct OverlayGuard {
    pub window: OverlayWindow,
}

impl OverlayGuard {
    pub fn open() -> Self {
        let mut out = empty_overlay();
        unsafe {
            m06_overlay_open(&mut out);
        }
        OverlayGuard {
            window: overlay_from(&out, text(&out.sharing), String::new()),
        }
    }

    pub fn set_none(&mut self) {
        let mut out = empty_overlay();
        unsafe {
            m06_overlay_set_none(&mut out);
        }
        let sharing_default = self.window.sharing_default.clone();
        self.window = overlay_from(&out, sharing_default, text(&out.sharing));
        self.window.visible_after_none = out.visible_after_none != 0;
    }

    pub fn order_out() {
        unsafe {
            m06_overlay_order_out();
        }
    }

    pub fn sync(&mut self) {
        let mut out = empty_overlay();
        unsafe {
            m06_overlay_sync(&mut out);
        }
        let sharing_default = self.window.sharing_default.clone();
        let sharing_none = self.window.sharing_none.clone();
        self.window = overlay_from(&out, sharing_default, sharing_none);
    }

    pub fn pump(millis: u32) {
        unsafe {
            m06_overlay_pump(millis);
        }
    }

    pub fn wait_visible(timeout_ms: u32) -> bool {
        unsafe { m06_overlay_wait_visible(timeout_ms) != 0 }
    }

    pub fn wait_hidden(timeout_ms: u32) -> bool {
        unsafe { m06_overlay_wait_hidden(timeout_ms) != 0 }
    }

    pub fn set_banner(text: &str) {
        let c_text = std::ffi::CString::new(text).unwrap_or_default();
        unsafe {
            let _ = m06_overlay_set_banner(c_text.as_ptr());
        }
    }

    pub fn escape_seen() -> bool {
        unsafe { m06_overlay_escape_seen() != 0 }
    }

    pub fn close() {
        unsafe {
            m06_overlay_close();
        }
    }
}

impl Drop for OverlayGuard {
    fn drop(&mut self) {
        OverlayGuard::close();
        let _ = unsafe { m06_presentation_clear() };
        TargetWindow::close();
    }
}

pub fn control_open() {
    unsafe {
        let _ = m06_control_open();
    }
}

pub fn capture(api: &str) -> CaptureSample {
    let c_api = std::ffi::CString::new(api).unwrap_or_default();
    let mut out = CaptureOut {
        opened: 0,
        width: 0,
        height: 0,
        marker: -1,
        control: -1,
        detail: [0; 384],
    };
    unsafe {
        m06_capture(c_api.as_ptr(), &mut out);
    }
    CaptureSample {
        sharing: String::new(),
        api: api.to_string(),
        opened: out.opened != 0,
        width: out.width,
        height: out.height,
        marker_pixels: out.marker,
        control_pixels: out.control,
        error: optional_text(&out.detail).map(|value| one_line(&value)),
    }
}

pub struct TargetWindow {
    pub accessibility: bool,
}

impl TargetWindow {
    pub fn open() -> Self {
        let mut out = ClickOut {
            overlay_is_key: 0,
            target_is_key: 0,
            click_count: 0,
            key_count: 0,
            accessibility: 0,
            listen: 0,
            error: [0; 256],
        };
        unsafe {
            m06_target_open(&mut out);
        }
        TargetWindow {
            accessibility: out.accessibility != 0,
        }
    }

    pub fn posted_click_focus() -> ClickFocus {
        let mut out = ClickOut {
            overlay_is_key: 0,
            target_is_key: 0,
            click_count: 0,
            key_count: 0,
            accessibility: 0,
            listen: 0,
            error: [0; 256],
        };
        unsafe {
            m06_posted_click_focus(&mut out);
        }
        let granted = out.accessibility != 0;
        let error = optional_text(&out.error);
        let untrusted = error
            .as_deref()
            .is_some_and(|value| value.contains("target never became key"));
        let click = if !granted {
            "not-applicable"
        } else if out.click_count > 0 {
            "fell-through"
        } else if untrusted {
            "measurement-unreliable"
        } else {
            "blocked"
        };
        let focus = if out.overlay_is_key != 0 {
            "moved"
        } else if out.target_is_key != 0 {
            "held"
        } else if untrusted {
            "measurement-unreliable"
        } else {
            "moved"
        };
        ClickFocus {
            ignores_mouse_events: true,
            overlay_became_key: out.overlay_is_key != 0,
            target_stayed_key: out.target_is_key != 0,
            posted_click_count: out.click_count,
            posted_key_count: out.key_count,
            accessibility: granted,
            measured_click: click.to_string(),
            measured_focus: focus.to_string(),
            typed_click: String::new(),
            typed_focus: String::new(),
            click_outcome: click.to_string(),
            focus_outcome: focus.to_string(),
            note: error.unwrap_or_default(),
        }
    }

    pub fn close() {
        unsafe {
            m06_target_close();
        }
    }
}

impl Drop for TargetWindow {
    fn drop(&mut self) {
        TargetWindow::close();
    }
}

#[derive(Clone, Debug)]
pub struct PresentationBits {
    pub app_active: bool,
    pub process_switching: bool,
    pub force_quit: bool,
    pub session_termination: bool,
}

pub fn presentation_apply() -> PresentationBits {
    let mut out = PresentOut {
        app_active: 0,
        process_switching: 0,
        force_quit: 0,
        session_termination: 0,
        error: [0; 256],
    };
    unsafe {
        m06_presentation_apply(&mut out);
    }
    PresentationBits {
        app_active: out.app_active != 0,
        process_switching: out.process_switching != 0,
        force_quit: out.force_quit != 0,
        session_termination: out.session_termination != 0,
    }
}

pub fn presentation_clear() {
    unsafe {
        let _ = m06_presentation_clear();
    }
}

pub fn display_count() -> i32 {
    unsafe { m06_display_count() }
}

pub fn screen_capture_access() -> bool {
    unsafe { m06_screen_capture_access() != 0 }
}

pub fn hw_model() -> String {
    let mut buf = [0u8; 128];
    unsafe {
        m06_hw_model(buf.as_mut_ptr() as *mut c_char, buf.len());
    }
    text(&buf)
}
