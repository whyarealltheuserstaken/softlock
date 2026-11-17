//! xrandr, wlr-randr, DRM connector status. No EDID.

use std::fs;
use std::path::Path;

use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DisplayInfo {
    pub name: String,
    pub connected: bool,
    pub kind: String,
    pub current_mode: Option<String>,
    pub current_rate: Option<String>,
}

pub fn parse_xrandr(text: &str) -> Vec<DisplayInfo> {
    let mut out: Vec<DisplayInfo> = Vec::new();
    let mut current: Option<usize> = None;
    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(idx) = current
                && line.contains('*')
            {
                if let Some(mode) = line.split_whitespace().next()
                    && !mode.is_empty()
                    && out[idx].current_mode.is_none()
                {
                    out[idx].current_mode = Some(mode.to_string());
                }
                for token in line.split_whitespace().skip(1) {
                    if token.contains('*') {
                        let rate = token.trim_end_matches(['*', '+']);
                        if !rate.is_empty() {
                            out[idx].current_rate = Some(rate.to_string());
                        }
                    }
                }
            }
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else { continue };
        if name == "Screen" {
            continue;
        }
        let Some(state) = parts.next() else { continue };
        if state != "connected" && state != "disconnected" {
            continue;
        }
        out.push(DisplayInfo {
            name: name.to_string(),
            connected: state == "connected",
            kind: "xrandr".to_string(),
            current_mode: None,
            current_rate: None,
        });
        current = Some(out.len() - 1);
    }
    out
}

pub fn parse_wlr_randr(text: &str) -> Vec<DisplayInfo> {
    let mut out: Vec<DisplayInfo> = Vec::new();
    let mut current: Option<usize> = None;
    for line in text.lines() {
        if !line.starts_with(' ') && !line.starts_with('\t') && !line.is_empty() {
            let name = line.split_whitespace().next().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            out.push(DisplayInfo {
                name,
                connected: true,
                kind: "wlr-randr".to_string(),
                current_mode: None,
                current_rate: None,
            });
            current = Some(out.len() - 1);
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("Enabled:")
            && let Some(idx) = current
        {
            out[idx].connected = trimmed.ends_with("yes");
        }
        if (trimmed.contains("(preferred, current)") || trimmed.contains("(current)"))
            && let Some(idx) = current
        {
            let mode = trimmed.split_whitespace().next().unwrap_or("");
            if !mode.is_empty() {
                out[idx].current_mode = Some(mode.to_string());
            }
        }
    }
    out
}

pub fn parse_drm_status_map(pairs: &[(String, String)]) -> Vec<DisplayInfo> {
    let mut out = Vec::new();
    for (name, status) in pairs {
        let connected = status.trim() == "connected";
        out.push(DisplayInfo {
            name: connector_name(name),
            connected,
            kind: "drm".to_string(),
            current_mode: None,
            current_rate: None,
        });
    }
    out
}

fn connector_name(raw: &str) -> String {
    raw.rsplit('/').next().unwrap_or(raw).to_string()
}

pub fn parse_drm_dir(dir: impl AsRef<Path>) -> Vec<DisplayInfo> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.contains('-') {
            continue;
        }
        let status_path = entry.path().join("status");
        let Ok(status) = fs::read_to_string(status_path) else {
            continue;
        };
        pairs.push((name.into_owned(), status));
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    parse_drm_status_map(&pairs)
}

/// Snapshot of connected names, for hot-plug comparison.
pub fn connected_names(displays: &[DisplayInfo]) -> Vec<String> {
    displays
        .iter()
        .filter(|d| d.connected)
        .map(|d| d.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_drm_status_map, parse_wlr_randr, parse_xrandr};

    #[test]
    fn xrandr_connected_and_mode() {
        let text = "\
Screen 0: minimum 320 x 200, current 1920 x 1080
eDP-1 connected primary 1920x1080+0+0 (normal left inverted right x axis y axis)
   1920x1080     60.00*+  59.97
   1600x900      60.00
HDMI-1 disconnected (normal left inverted right x axis y axis)
";
        let rows = parse_xrandr(text);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].connected);
        assert_eq!(rows[0].name, "eDP-1");
        assert_eq!(rows[0].current_mode.as_deref(), Some("1920x1080"));
        assert_eq!(rows[0].current_rate.as_deref(), Some("60.00"));
        assert!(!rows[1].connected);
    }

    #[test]
    fn wlr_randr_enabled() {
        let text = "\
eDP-1
  Physical size: 309x174 mm
  Enabled: yes
  Modes:
    1920x1080 px, 60.001999 Hz (preferred, current)
HDMI-A-1
  Enabled: no
";
        let rows = parse_wlr_randr(text);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].connected);
        assert_eq!(rows[0].current_mode.as_deref(), Some("1920x1080"));
        assert!(!rows[1].connected);
    }

    #[test]
    fn drm_status() {
        let rows = parse_drm_status_map(&[
            ("card0-eDP-1".into(), "connected\n".into()),
            ("card0-HDMI-A-1".into(), "disconnected\n".into()),
        ]);
        assert_eq!(rows[0].name, "card0-eDP-1");
        assert!(rows[0].connected);
        assert!(!rows[1].connected);
    }
}
