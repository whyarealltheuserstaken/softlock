//! macOS platform backend.
//!
//! Phase 2 is not implemented. Every capability reports Unsupported.
//! The decisions and open risks are in `docs/platforms/macos.md`.

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
}
