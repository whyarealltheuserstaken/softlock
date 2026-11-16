//! Seam 3 self-tests. The desktop cases run on a Windows runner.

#[cfg(not(windows))]
#[test]
fn desktop_probe_runs_on_windows() {
    assert!(!softlock_seam3::desktop_supported());
}

#[cfg(windows)]
mod desktop {
    use std::sync::{Mutex, MutexGuard};
    use std::time::Duration;

    use softlock_seam3::{
        CaptureMethod, PowerRequestKind, Probe, RegistryData, RegistryRoot, capture_marker,
        delete_tree, diff_devices, diff_power, diff_registry, put_dword, send_virtual_key,
        snapshot_devices, snapshot_key, snapshot_power_requests,
    };

    const F13: u16 = 0x7C;
    const MARKER_PIXELS: u64 = 8_000;

    fn lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    #[test]
    fn probe_window_receives_sendinput_key_and_capture_contains_marker() {
        let _guard = lock();
        let probe = Probe::open().expect("probe window");
        let focus_before = probe.focus();
        let mut input = None;
        for _ in 0..5 {
            probe.claim_focus();
            send_virtual_key(F13, 0).expect("SendInput");
            let _ = probe.wait_for_key(F13, Duration::from_millis(400));
            if let Some(recorded) = probe
                .inputs()
                .into_iter()
                .rev()
                .find(|input| input.virtual_key == F13 && input.probe_was_foreground)
            {
                input = Some(recorded);
                break;
            }
        }
        let input = input.unwrap_or_else(|| {
            panic!(
                "probe did not receive virtual key {F13}; focus={:?}; inputs={:?}",
                probe.focus(),
                probe.inputs()
            )
        });
        assert!(input.reached_probe);
        assert_eq!(input.virtual_key, F13);
        assert!(
            input.probe_was_foreground,
            "focus when the key arrived: {focus_before:?}; now {:?}",
            probe.focus()
        );
        let focus = probe.focus();
        assert!(
            focus.probe_is_foreground,
            "probe lost focus: {}",
            focus.foreground_title
        );

        probe.repaint();
        std::thread::sleep(Duration::from_millis(200));
        let captures = capture_marker(probe.hwnd()).expect("captures");
        for method in [
            CaptureMethod::Gdi,
            CaptureMethod::DxgiDuplication,
            CaptureMethod::WindowsGraphicsCapture,
        ] {
            let shot = captures
                .iter()
                .find(|shot| shot.method == method)
                .unwrap_or_else(|| panic!("missing {method:?}"));
            assert!(
                shot.marker_pixels >= MARKER_PIXELS,
                "{method:?} saw {} marker pixels in {}x{}",
                shot.marker_pixels,
                shot.width,
                shot.height
            );
        }
    }

    #[test]
    fn registry_device_and_power_snapshots_diff() {
        let _guard = lock();
        let key = format!("Software\\seam3-probe-{}", std::process::id());
        delete_tree(RegistryRoot::CurrentUser, &key).expect("clear leftover key");

        let before = snapshot_key(RegistryRoot::CurrentUser, &key).expect("registry before");
        assert!(before.values.is_empty());
        put_dword(RegistryRoot::CurrentUser, &key, "Token", 1).expect("write dword");
        let written = snapshot_key(RegistryRoot::CurrentUser, &key).expect("registry written");
        let added = diff_registry(&before, &written);
        assert!(added.changes.iter().any(|change| {
            change.path.value == "Token" && change.after == Some(RegistryData::Dword(1))
        }));
        put_dword(RegistryRoot::CurrentUser, &key, "Token", 2).expect("rewrite dword");
        let changed = snapshot_key(RegistryRoot::CurrentUser, &key).expect("registry changed");
        let edit = diff_registry(&written, &changed);
        assert!(edit.changes.iter().any(|change| {
            change.before == Some(RegistryData::Dword(1))
                && change.after == Some(RegistryData::Dword(2))
        }));
        delete_tree(RegistryRoot::CurrentUser, &key).expect("delete key");
        let cleared = snapshot_key(RegistryRoot::CurrentUser, &key).expect("registry cleared");
        let removed = diff_registry(&changed, &cleared);
        assert!(removed.changes.iter().any(|change| {
            change.path.value == "Token"
                && change.before == Some(RegistryData::Dword(2))
                && change.after.is_none()
        }));

        let devices = snapshot_devices().expect("devices");
        assert!(!devices.devices.is_empty());
        assert!(diff_devices(&devices, &devices).is_empty());
        let mut flipped = devices.clone();
        let (instance_id, enabled) = flipped
            .devices
            .iter()
            .next()
            .map(|(id, enabled)| (id.clone(), *enabled))
            .expect("a device");
        flipped.devices.insert(instance_id.clone(), !enabled);
        let device_diff = diff_devices(&devices, &flipped);
        assert!(device_diff.changes.iter().any(|change| {
            change.instance_id == instance_id
                && change.before == Some(enabled)
                && change.after == Some(!enabled)
        }));

        let reason = format!("seam3-power-{}", std::process::id());
        let power_before = snapshot_power_requests().expect("power before");
        let hold = softlock_seam3::PowerHold::system(&reason).expect("power request");
        let mut power_during = power_before.clone();
        let mut saw_request = false;
        for _ in 0..20 {
            power_during = snapshot_power_requests().expect("power during");
            if power_during.requests.iter().any(|request| {
                request.kind == PowerRequestKind::System && request.reason.contains(&reason)
            }) {
                saw_request = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        assert!(
            saw_request,
            "active power requests did not include {reason}: {:?}",
            power_during.requests
        );
        let power_diff = diff_power(&power_before, &power_during);
        assert!(power_diff.added.iter().any(|request| {
            request.kind == PowerRequestKind::System && request.reason.contains(&reason)
        }));
        drop(hold);
        let mut gone = false;
        let mut power_after = power_during.clone();
        for _ in 0..20 {
            power_after = snapshot_power_requests().expect("power after");
            if power_after
                .requests
                .iter()
                .all(|request| !request.reason.contains(&reason))
            {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        assert!(
            gone,
            "power request {reason} stayed active: {:?}",
            power_after.requests
        );
        let restored = diff_power(&power_before, &power_after);
        assert!(
            restored
                .added
                .iter()
                .all(|request| !request.reason.contains(&reason))
        );
        assert!(
            restored
                .removed
                .iter()
                .all(|request| !request.reason.contains(&reason))
        );
    }

    #[cfg(feature = "simulated-physical")]
    #[test]
    fn hook_sees_nonce_tagged_sendinput_as_simulated_physical() {
        let _guard = lock();
        use softlock_lockhost::{
            HookInput, InputClass, RunNonce, classify, clear_run_nonce, set_run_nonce,
        };

        let nonce = RunNonce::random().expect("nonce");
        set_run_nonce(nonce);
        let tagged = softlock_seam3::observe_injected_key(F13, nonce.as_extra_info())
            .expect("tagged hook event");
        assert!(tagged.injected, "SendInput was not marked injected");
        assert_eq!(tagged.extra_info, nonce.as_extra_info());
        assert_eq!(
            softlock_seam3::classify_observed(tagged),
            InputClass::SimulatedPhysical
        );
        assert_eq!(
            classify(HookInput {
                injected: tagged.injected,
                extra_info: tagged.extra_info,
            }),
            InputClass::SimulatedPhysical
        );

        let plain = softlock_seam3::observe_injected_key(F13, 0).expect("plain hook event");
        assert!(plain.injected);
        assert_eq!(plain.extra_info, 0);
        assert_eq!(
            softlock_seam3::classify_observed(plain),
            InputClass::Injected
        );
        clear_run_nonce();
    }
}
