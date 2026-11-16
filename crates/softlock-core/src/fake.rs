//! In-memory platform. It records what the controller asked for and never locks a machine.

use std::collections::BTreeMap;

use crate::journal::Journal;
use crate::platform::{
    ApplyObservation, AwakeGuard, AwakeRequest, Credentials, DiskEncryption, DrivenPlatform,
    EncryptionStatus, GuardError, GuardHealth, HardenError, Hardening, HardeningConfig,
    HardeningPlan, InputGuard, LockScreenLook, OsLockError, Overlay, OverlayError, OverlayHealth,
    PassthroughTarget, PhysicalInputSink, Power, PowerCaps, PowerError, PromptView, SessionControl,
    SessionId, SessionInfo, UserId, Verdict,
};
use crate::state::{ArmStep, Secret, SessionEvent};

#[derive(Clone, Debug)]
pub struct FakePlatform {
    settings: BTreeMap<String, String>,
    observations: Vec<ApplyObservation>,
    fail_at: Option<ArmStep>,
    accept_next: bool,
    revert_failed: bool,
    guard_armed: bool,
    overlay_shown: bool,
    power_held: bool,
    os_locks: u32,
}

impl Default for FakePlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl FakePlatform {
    pub fn new() -> Self {
        Self {
            settings: BTreeMap::new(),
            observations: Vec::new(),
            fail_at: None,
            accept_next: false,
            revert_failed: false,
            guard_armed: false,
            overlay_shown: false,
            power_held: false,
            os_locks: 0,
        }
    }

    pub fn set_setting(&mut self, key: &str, value: &str) {
        self.settings.insert(key.to_string(), value.to_string());
    }

    pub fn settings(&self) -> &BTreeMap<String, String> {
        &self.settings
    }

    pub fn fail_step(&mut self, step: ArmStep) {
        self.fail_at = Some(step);
    }

    pub fn accept_secrets(&mut self, accept: bool) {
        self.accept_next = accept;
    }

    pub fn apply_observations(&self) -> &[ApplyObservation] {
        &self.observations
    }

    pub fn os_lock_count(&self) -> u32 {
        self.os_locks
    }
}

impl DrivenPlatform for FakePlatform {
    fn fail_at(&self) -> Option<ArmStep> {
        self.fail_at
    }

    fn settings(&self) -> &BTreeMap<String, String> {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut BTreeMap<String, String> {
        &mut self.settings
    }

    fn note_apply(&mut self, observation: ApplyObservation) {
        self.observations.push(observation);
    }

    fn set_guard_armed(&mut self, armed: bool) {
        self.guard_armed = armed;
    }

    fn set_overlay_shown(&mut self, shown: bool) {
        self.overlay_shown = shown;
    }

    fn set_power_held(&mut self, held: bool) {
        self.power_held = held;
    }

    fn record_os_lock(&mut self) {
        self.os_locks = self.os_locks.saturating_add(1);
    }

    fn take_accept(&mut self) -> bool {
        let accept = self.accept_next;
        self.accept_next = false;
        accept
    }

    fn revert_failed(&self) -> bool {
        self.revert_failed
    }

    fn guard_health(&self) -> GuardHealth {
        GuardHealth {
            hook_alive: self.guard_armed,
            last_physical_input_ms: None,
        }
    }

    fn overlay_health(&self) -> OverlayHealth {
        OverlayHealth {
            covered_monitors: u32::from(self.overlay_shown),
            capture_excluded: self.overlay_shown,
            topmost: self.overlay_shown,
        }
    }
}

impl InputGuard for FakePlatform {
    fn arm(&mut self, _sink: PhysicalInputSink) -> Result<(), GuardError> {
        if self.fail_at == Some(ArmStep::Guard) {
            return Err(GuardError::Failed);
        }
        self.guard_armed = true;
        Ok(())
    }

    fn disarm(&mut self) {
        self.guard_armed = false;
    }

    fn set_passthrough(&mut self, _target: Option<PassthroughTarget>) {}

    fn health(&self) -> GuardHealth {
        DrivenPlatform::guard_health(self)
    }
}

impl Overlay for FakePlatform {
    fn show(&mut self, _look: &LockScreenLook) -> Result<(), OverlayError> {
        if self.fail_at == Some(ArmStep::Overlay) {
            return Err(OverlayError::Failed);
        }
        self.overlay_shown = true;
        Ok(())
    }

    fn update(&mut self, _view: &PromptView) {}

    fn hide(&mut self) {
        self.overlay_shown = false;
    }

    fn health(&self) -> OverlayHealth {
        DrivenPlatform::overlay_health(self)
    }
}

impl SessionControl for FakePlatform {
    fn os_lock(&mut self, _session: SessionId) -> Result<(), OsLockError> {
        self.os_locks = self.os_locks.saturating_add(1);
        Ok(())
    }

    fn session_events(&self) -> std::sync::mpsc::Receiver<SessionEvent> {
        let (_sender, receiver) = std::sync::mpsc::channel();
        receiver
    }
}

impl Hardening for FakePlatform {
    fn plan(&self, _cfg: &HardeningConfig, _session: &SessionInfo) -> HardeningPlan {
        crate::platform::standard_hardening_plan()
    }

    fn apply(&mut self, plan: &HardeningPlan, journal: &mut Journal) -> Result<(), HardenError> {
        if self.fail_at == Some(ArmStep::Hardening) {
            return Err(HardenError::Failed);
        }
        self.observations.push(ApplyObservation {
            durable_entries: journal.durable_entries().to_vec(),
            settings_before: self.settings.clone(),
        });
        for change in &plan.changes {
            self.settings
                .insert(change.target.clone(), change.new_value.clone());
        }
        Ok(())
    }

    fn revert(&mut self, journal: &Journal) -> Result<(), HardenError> {
        if self.revert_failed {
            return Err(HardenError::Failed);
        }
        for entry in journal.entries() {
            match &entry.previous {
                Some(value) => {
                    self.settings.insert(entry.target.clone(), value.clone());
                }
                None => {
                    self.settings.remove(&entry.target);
                }
            }
        }
        Ok(())
    }
}

impl Power for FakePlatform {
    fn hold(&mut self, _request: AwakeRequest) -> Result<AwakeGuard, PowerError> {
        if self.fail_at == Some(ArmStep::Power) {
            return Err(PowerError::Failed);
        }
        self.power_held = true;
        Ok(AwakeGuard::inactive())
    }

    fn capabilities(&self) -> PowerCaps {
        PowerCaps {
            supported: true,
            modern_standby: false,
        }
    }
}

impl Credentials for FakePlatform {
    fn verify_password(&self, _user: &UserId, _password: &Secret) -> Verdict {
        if self.accept_next {
            Verdict::Verified
        } else {
            Verdict::Rejected
        }
    }
}

impl EncryptionStatus for FakePlatform {
    fn system_volume(&self) -> DiskEncryption {
        DiskEncryption::Off
    }
}
