//! macOS platform backend.
//!
//! Phase 2 is not implemented. Every capability reports Unsupported.
//! The decisions and open risks are in `docs/platforms/macos.md`.
//!
//! The lock-path chooser, the FileVault text parser, and the screen-saver
//! password check are pure. They do not call `SACLockScreenImmediate`, start
//! the screen saver, or change FileVault. The kit's run gate lives in the
//! spike, not in this crate.

mod filevault;
mod lock_choice;
mod screensaver;

pub use filevault::{
    CommandText, FileVaultRead, console_filevault, disk_label, interpret_isactive, interpret_status,
};
pub use lock_choice::{
    FailClosedEnd, LOGIN_FRAMEWORK_IMAGE, LockArm, PRIVATE_LOCK_SYMBOL,
    SCREEN_SAVER_ENGINE_CANDIDATES, choose, fail_closed_after_error, refuse_to_arm_message,
};
pub use screensaver::{
    LockSetting, LockSettingReport, PasswordAfterScreensaver, ScreenLockRead, combine_lock_setting,
    interpret_screen_lock_status, parse_defaults_value, password_after_screensaver,
    preference_label, resolve_preference,
};
pub use softlock_core::CapabilityReport;
pub use softlock_core::UnsupportedPlatform as MacosBackend;

pub fn capability_report() -> CapabilityReport {
    MacosBackend.capability_report()
}

#[cfg(test)]
mod tests {
    use super::capability_report;
    use softlock_core::{
        AwakeRequest, CapabilityReport, Credentials, DiskEncryption, EncryptionStatus, GuardError,
        HardenError, Hardening, HardeningPlan, InputGuard, LockScreenLook, OsLockError, Overlay,
        OverlayError, PhysicalInputSink, Power, PowerError, PromptView, Secret, SessionControl,
        SessionId, Support, UNSUPPORTED_PLATFORM_MESSAGE, UserId, Verdict,
    };

    use super::MacosBackend;

    #[test]
    fn every_capability_is_unsupported() {
        let report = capability_report();
        assert_unsupported(&report);
        let mut backend = MacosBackend;
        assert_eq!(
            InputGuard::arm(&mut backend, PhysicalInputSink::noop()),
            Err(GuardError::Unsupported)
        );
        InputGuard::disarm(&mut backend);
        InputGuard::set_passthrough(&mut backend, None);
        assert!(!InputGuard::health(&backend).hook_alive);

        assert_eq!(
            Overlay::show(&mut backend, &LockScreenLook::default()),
            Err(OverlayError::Unsupported)
        );
        Overlay::update(&mut backend, &PromptView::idle());
        Overlay::hide(&mut backend);
        assert!(!Overlay::health(&backend).capture_excluded);

        assert_eq!(
            SessionControl::os_lock(&mut backend, SessionId(1)),
            Err(OsLockError::Unsupported)
        );
        let _events = SessionControl::session_events(&backend);

        let plan = HardeningPlan::empty();
        let mut journal = softlock_core::Journal::empty();
        assert_eq!(
            Hardening::apply(&mut backend, &plan, &mut journal),
            Err(HardenError::Unsupported)
        );
        assert_eq!(
            Hardening::revert(&mut backend, &journal),
            Err(HardenError::Unsupported)
        );

        assert_eq!(
            Power::hold(&mut backend, AwakeRequest::system()),
            Err(PowerError::Unsupported)
        );
        assert!(!Power::capabilities(&backend).supported);

        assert_eq!(
            Credentials::verify_password(
                &backend,
                &UserId("owner".to_string()),
                &Secret::new("pw")
            ),
            Verdict::Rejected
        );
        assert_eq!(
            EncryptionStatus::system_volume(&backend),
            DiskEncryption::Unsupported
        );
    }

    fn assert_unsupported(report: &CapabilityReport) {
        assert_eq!(report.message, UNSUPPORTED_PLATFORM_MESSAGE);
        assert!(!report.is_supported());
        assert_eq!(report.input_guard, Support::Unsupported);
        assert_eq!(report.overlay, Support::Unsupported);
        assert_eq!(report.session_control, Support::Unsupported);
        assert_eq!(report.hardening, Support::Unsupported);
        assert_eq!(report.power, Support::Unsupported);
        assert_eq!(report.credentials, Support::Unsupported);
        assert_eq!(report.encryption_status, Support::Unsupported);
    }

    #[test]
    fn the_backend_does_not_call_the_live_lock_or_filevault() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let entries = std::fs::read_dir(&root).expect("macos src");
        let mut files = 0;
        for entry in entries {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            files += 1;
            let text = std::fs::read_to_string(&path).expect("read");
            let dlopen_call = ["dlo", "pen("].concat();
            let dlsym_call = ["dls", "ym("].concat();
            assert!(
                !text.contains(&dlopen_call) && !text.contains(&dlsym_call),
                "{} calls the live symbol",
                path.display()
            );
            let command_new = ["Command", "::new"].concat();
            let link_attr = ["#[", "link"].concat();
            assert!(
                !text.contains(&command_new) && !text.contains(&link_attr),
                "{} links or spawns a command",
                path.display()
            );
        }
        assert!(files >= 4);
    }
}
