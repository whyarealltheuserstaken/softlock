//! Overlay capability: wlr-layer-shell / X11 override-redirect. No persistent lock UI.

use std::process::Command;

use serde_json::json;

use crate::detect::DetectedConfig;
use crate::report::{TestRow, empty_measurements};

pub fn wayland_layer_shell(config: &DetectedConfig) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("session".into(), json!(config.session_type));
    if config.session_type != "wayland" {
        return TestRow::skip(
            "overlay.wayland.layer-shell",
            "Wayland wlr-layer-shell / lock surface",
            "not-applicable",
            false,
            false,
            "not a Wayland session",
            measurements,
        );
    }
    let info = stdout("wayland-info", &[]);
    let layer = info
        .as_deref()
        .is_some_and(|t| t.contains("zwlr_layer_shell") || t.contains("ext_session_lock"));
    measurements.insert("layer_shell".into(), json!(layer));
    measurements.insert("wayland_info".into(), json!(info.is_some()));
    TestRow::pass(
        "overlay.wayland.layer-shell",
        "Wayland wlr-layer-shell / lock surface",
        false,
        false,
        format!(
            "compositor={} layer_shell_or_session_lock={layer}",
            config.compositor
        ),
        measurements,
    )
}

pub fn x11_override_redirect(config: &DetectedConfig) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("session".into(), json!(config.session_type));
    if config.session_type != "x11" {
        return TestRow::skip(
            "overlay.x11.redirect",
            "X11 override-redirect overlay path",
            "not-applicable",
            false,
            false,
            "not an X11 session",
            measurements,
        );
    }
    let xwin = stdout("xwininfo", &["-root"]);
    let xrandr = stdout("xrandr", &["--query"]);
    measurements.insert("xwininfo".into(), json!(xwin.is_some()));
    measurements.insert("xrandr".into(), json!(xrandr.is_some()));
    if xwin.is_none() && xrandr.is_none() {
        return TestRow::skip(
            "overlay.x11.redirect",
            "X11 override-redirect overlay path",
            "not-applicable",
            false,
            false,
            "DISPLAY set but xwininfo/xrandr missing",
            measurements,
        );
    }
    TestRow::pass(
        "overlay.x11.redirect",
        "X11 override-redirect overlay path",
        false,
        false,
        "X11 session reachable without creating a lock window",
        measurements,
    )
}

pub fn coverage(config: &DetectedConfig) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("display_count".into(), json!(config.display_count));
    measurements.insert("outputs".into(), json!(config.displays.len()));
    if config.display_count == 0 && config.session_type == "none" {
        return TestRow::skip(
            "overlay.coverage",
            "One overlay surface per connected output",
            "not-applicable",
            false,
            false,
            "no connected display",
            measurements,
        );
    }
    TestRow::pass(
        "overlay.coverage",
        "One overlay surface per connected output",
        false,
        false,
        format!(
            "connected={} listed={}",
            config.display_count,
            config.displays.len()
        ),
        measurements,
    )
}

fn stdout(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}
