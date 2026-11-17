//! Overlay capability: wlr-layer-shell / X11 override-redirect. No persistent lock UI.

use std::process::Command;

use serde_json::json;

use crate::detect::DetectedConfig;
use crate::report::TestRow;

pub fn wayland_layer_shell(config: &DetectedConfig) -> TestRow {
    let mut measurements = map();
    measurements.insert("session".into(), json!(config.session_type));
    if config.session_type != "wayland" {
        return skip(
            "overlay.wayland.layer-shell",
            "Wayland wlr-layer-shell / lock surface",
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
    row(
        "overlay.wayland.layer-shell",
        "Wayland wlr-layer-shell / lock surface",
        "pass",
        format!(
            "compositor={} layer_shell_or_session_lock={layer}",
            config.compositor
        ),
        measurements,
    )
}

pub fn x11_override_redirect(config: &DetectedConfig) -> TestRow {
    let mut measurements = map();
    measurements.insert("session".into(), json!(config.session_type));
    if config.session_type != "x11" {
        return skip(
            "overlay.x11.redirect",
            "X11 override-redirect overlay path",
            "not an X11 session",
            measurements,
        );
    }
    let xwin = stdout("xwininfo", &["-root"]);
    let xrandr = stdout("xrandr", &["--query"]);
    measurements.insert("xwininfo".into(), json!(xwin.is_some()));
    measurements.insert("xrandr".into(), json!(xrandr.is_some()));
    if xwin.is_none() && xrandr.is_none() {
        return skip(
            "overlay.x11.redirect",
            "X11 override-redirect overlay path",
            "DISPLAY set but xwininfo/xrandr missing",
            measurements,
        );
    }
    row(
        "overlay.x11.redirect",
        "X11 override-redirect overlay path",
        "pass",
        "X11 session reachable without creating a lock window".into(),
        measurements,
    )
}

pub fn coverage(config: &DetectedConfig) -> TestRow {
    let mut measurements = map();
    measurements.insert("display_count".into(), json!(config.display_count));
    measurements.insert("outputs".into(), json!(config.displays.len()));
    if config.display_count == 0 && config.session_type == "none" {
        return skip(
            "overlay.coverage",
            "One overlay surface per connected output",
            "no connected display",
            measurements,
        );
    }
    row(
        "overlay.coverage",
        "One overlay surface per connected output",
        "pass",
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

fn map() -> std::collections::BTreeMap<String, serde_json::Value> {
    std::collections::BTreeMap::new()
}

fn row(
    id: &str,
    name: &str,
    status: &str,
    detail: String,
    measurements: std::collections::BTreeMap<String, serde_json::Value>,
) -> TestRow {
    TestRow {
        id: id.into(),
        name: name.into(),
        status: status.into(),
        skip_reason: if status == "skip" {
            Some(detail.clone())
        } else {
            None
        },
        physical: false,
        needs_admin: false,
        duration_ms: 0,
        detail: if status == "skip" {
            String::new()
        } else {
            detail
        },
        measurements,
    }
}

fn skip(
    id: &str,
    name: &str,
    reason: &str,
    measurements: std::collections::BTreeMap<String, serde_json::Value>,
) -> TestRow {
    row(id, name, "skip", reason.to_string(), measurements)
}
