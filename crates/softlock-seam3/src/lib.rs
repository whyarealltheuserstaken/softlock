//! Seam 3 probe harness.
//!
//! On Windows this records input that reached a probe window, that window's
//! focus, and captures through GDI, DXGI desktop duplication, and
//! Windows.Graphics.Capture. It also snapshots registry values, device enable
//! states, and active power requests. The diff helpers are pure and run on
//! every platform.

mod model;
mod pixels;
mod powercfg;

#[cfg(windows)]
mod win;

pub use model::{
    DeviceChange, DeviceDiff, DeviceSnapshot, PowerDiff, PowerRequest, PowerRequestKind,
    PowerSnapshot, RegistryChange, RegistryData, RegistryDiff, RegistryPath, RegistryRoot,
    RegistrySnapshot, diff_devices, diff_power, diff_registry,
};
pub use pixels::{MARKER, Rgb, count_near};
pub use powercfg::parse_powercfg_requests;

/// The desktop probe, captures, and live snapshots run on Windows.
pub fn desktop_supported() -> bool {
    cfg!(windows)
}

#[cfg(all(windows, feature = "simulated-physical"))]
pub use win::classify_observed;
#[cfg(windows)]
pub use win::{
    CaptureMethod, DeviceInfo, FocusState, HarnessError, HookObservation, MethodCapture, PowerHold,
    Probe, RecordedInput, capture_marker, delete_tree, observe_injected_key, put_dword,
    send_virtual_key, snapshot_devices, snapshot_key, snapshot_power_requests, snapshot_values,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_count_matches_a_solid_image() {
        let pixel = [MARKER.b, MARKER.g, MARKER.r, 255];
        let mut pixels = Vec::new();
        pixels.extend(std::iter::repeat_n(pixel, 4).flatten());
        assert_eq!(count_near(&pixels, MARKER, 2), 4);
        assert_eq!(count_near(&pixels, Rgb { r: 0, g: 0, b: 0 }, 2), 0);
    }

    #[test]
    fn registry_diff_reports_added_changed_and_removed_values() {
        let path = RegistryPath {
            root: RegistryRoot::CurrentUser,
            key: "Software\\seam3-probe".to_string(),
            value: "Token".to_string(),
        };
        let before = RegistrySnapshot {
            values: [(path.clone(), RegistryData::Dword(1))].into(),
        };
        let after = RegistrySnapshot {
            values: [(path.clone(), RegistryData::Dword(2))].into(),
        };
        let diff = diff_registry(&before, &after);
        assert_eq!(diff.changes.len(), 1);
        assert_eq!(diff.changes[0].before, Some(RegistryData::Dword(1)));
        assert_eq!(diff.changes[0].after, Some(RegistryData::Dword(2)));

        let removed = diff_registry(&before, &RegistrySnapshot::empty());
        assert_eq!(removed.changes[0].after, None);
        let added = diff_registry(&RegistrySnapshot::empty(), &before);
        assert_eq!(added.changes[0].before, None);
        assert!(diff_registry(&before, &before).is_empty());
    }

    #[test]
    fn device_diff_reports_an_enable_state_change() {
        let before = DeviceSnapshot {
            devices: [("HID\\VID_1&0".to_string(), true)].into(),
        };
        let mut after = before.clone();
        after.devices.insert("HID\\VID_1&0".to_string(), false);
        let diff = diff_devices(&before, &after);
        assert_eq!(
            diff.changes,
            vec![DeviceChange {
                instance_id: "HID\\VID_1&0".to_string(),
                before: Some(true),
                after: Some(false),
            }]
        );
        assert!(diff_devices(&before, &before).is_empty());
    }

    #[test]
    fn power_diff_reports_an_added_request() {
        let request = PowerRequest {
            kind: PowerRequestKind::System,
            requester: "[PROCESS] probe.exe".to_string(),
            reason: "seam3-power".to_string(),
        };
        let before = PowerSnapshot { requests: vec![] };
        let after = PowerSnapshot {
            requests: vec![request.clone()],
        };
        let diff = diff_power(&before, &after);
        assert_eq!(diff.added, vec![request]);
        assert!(diff.removed.is_empty());
        assert!(diff_power(&after, &after).is_empty());
    }

    #[test]
    fn powercfg_text_keeps_the_reason_and_the_section() {
        let text = "\
DISPLAY:\r\n\
None.\r\n\
\r\n\
SYSTEM:\r\n\
[PROCESS] \\Device\\HarddiskVolume1\\probe.exe\r\n\
seam3-power-token\r\n\
\r\n\
AWAYMODE:\r\n\
None.\r\n";
        let requests = parse_powercfg_requests(text);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].kind, PowerRequestKind::System);
        assert!(requests[0].requester.contains("probe.exe"));
        assert_eq!(requests[0].reason, "seam3-power-token");
    }

    #[test]
    fn desktop_probe_is_unavailable_off_windows() {
        if !cfg!(windows) {
            assert!(!desktop_supported());
        }
    }
}
