//! Windows platform for the dev-launch controller.
//!
//! Lockdown policies are not applied. Ticket 15 writes them, after the journal
//! from ticket 16. A no-op here lets the state machine reach Locked without
//! changing the machine.

use std::sync::Arc;

use softlock_core::{
    AwakeGuard, AwakeRequest, Credentials, DiskEncryption, EncryptionStatus, GuardError,
    GuardHealth, HardenError, Hardening, HardeningPlan, InputGuard, Journal, LockScreenLook,
    OsLockError, Overlay, OverlayError, OverlayHealth, PassthroughTarget, PhysicalInputSink,
    PolicySetting, PolicySettings, Power, PowerCaps, PowerError, PromptView, Secret,
    SessionControl, SessionEvent, SessionId, UserId, Verdict, verify_softlock_pin,
};

use super::failclosed;
use super::guard;
use super::overlay;
use super::session::Shared;

pub(crate) struct WinPlatform {
    shared: Arc<Shared>,
}

impl WinPlatform {
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

impl InputGuard for WinPlatform {
    fn arm(&mut self, _sink: PhysicalInputSink) -> Result<(), GuardError> {
        guard::install(&self.shared)
    }

    fn disarm(&mut self) {
        guard::remove(&self.shared);
        failclosed::clear_execution_state(&self.shared);
    }

    fn set_passthrough(&mut self, _target: Option<PassthroughTarget>) {}

    fn health(&self) -> GuardHealth {
        let hook_alive = self
            .shared
            .hook_thread_id
            .load(std::sync::atomic::Ordering::Acquire)
            != 0
            && !self.shared.fail.load(std::sync::atomic::Ordering::Acquire)
            && !self.shared.stop.load(std::sync::atomic::Ordering::Acquire);
        GuardHealth {
            hook_alive,
            last_physical_input_ms: self.shared.physical_ms(),
        }
    }
}

impl Overlay for WinPlatform {
    fn show(&mut self, _look: &LockScreenLook) -> Result<(), OverlayError> {
        if let Some(callback) = self
            .shared
            .while_arming
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            callback();
        }
        overlay::create(&self.shared).map_err(|_| OverlayError::Failed)
    }

    fn update(&mut self, _view: &PromptView) {
        overlay::invalidate(&self.shared);
    }

    fn hide(&mut self) {
        overlay::destroy(&self.shared);
    }

    fn health(&self) -> OverlayHealth {
        OverlayHealth {
            covered_monitors: self
                .shared
                .monitors
                .load(std::sync::atomic::Ordering::Acquire),
            capture_excluded: self
                .shared
                .capture_excluded
                .load(std::sync::atomic::Ordering::Acquire),
            topmost: self
                .shared
                .topmost
                .load(std::sync::atomic::Ordering::Acquire),
        }
    }
}

impl SessionControl for WinPlatform {
    fn console_session(&self) -> SessionId {
        SessionId(failclosed::console_session())
    }

    fn os_lock(&mut self, session: SessionId) -> Result<(), OsLockError> {
        failclosed::lock_or_disconnect(session.0).map_err(|_| OsLockError::Failed)
    }

    fn session_events(&self) -> std::sync::mpsc::Receiver<SessionEvent> {
        self.shared.take_events()
    }
}

impl Hardening for WinPlatform {
    fn apply(&mut self, _plan: &HardeningPlan, _journal: &mut Journal) -> Result<(), HardenError> {
        Ok(())
    }

    fn revert(&mut self, _journal: &Journal) -> Result<(), HardenError> {
        Ok(())
    }
}

impl Power for WinPlatform {
    fn hold(&mut self, request: AwakeRequest) -> Result<AwakeGuard, PowerError> {
        if request.system_required {
            unsafe {
                windows::Win32::System::Power::SetThreadExecutionState(
                    windows::Win32::System::Power::ES_CONTINUOUS
                        | windows::Win32::System::Power::ES_SYSTEM_REQUIRED,
                );
            }
            self.shared
                .execution
                .store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(AwakeGuard::inactive())
    }

    fn capabilities(&self) -> PowerCaps {
        PowerCaps {
            supported: true,
            modern_standby: false,
        }
    }
}

impl Credentials for WinPlatform {
    fn verify_password(&self, _user: &UserId, _password: &Secret) -> Verdict {
        Verdict::Rejected
    }

    fn verify_secret(&self, method: &softlock_core::MethodId, secret: &Secret) -> Verdict {
        if method.as_str() != softlock_core::MethodId::softlock_pin().as_str() {
            return Verdict::Rejected;
        }
        verify_softlock_pin(secret, &self.shared.pin_hash)
    }
}

impl PolicySettings for WinPlatform {
    fn policy(&self, _setting: PolicySetting) -> Option<String> {
        None
    }

    fn set_policy(&mut self, _setting: PolicySetting, _value: &str) {}

    fn clear_policy(&mut self, _setting: PolicySetting) {}
}

impl EncryptionStatus for WinPlatform {
    fn system_volume(&self) -> DiskEncryption {
        DiskEncryption::Unknown
    }
}
