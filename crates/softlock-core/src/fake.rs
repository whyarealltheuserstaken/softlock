//! In-memory platform. It records what the controller asked for and never locks a machine.
//!
//! Fault injection (`fail_step`, `fail_revert`, `accept_secrets`, manual verdicts)
//! lives here. Those knobs are not part of the platform traits.

use std::collections::BTreeMap;

use crate::journal::{Journal, JournalEntry};
use crate::platform::{
    AwakeGuard, AwakeRequest, ConfirmationCheck, Credentials, DiskEncryption, EncryptionStatus,
    GuardError, GuardHealth, HardenError, Hardening, HardeningConfig, HardeningPlan, InputGuard,
    LockScreenLook, OsLockError, Overlay, OverlayError, OverlayHealth, PassthroughTarget,
    PhysicalInputSink, PolicySetting, PolicySettings, Power, PowerCaps, PowerError, PromptView,
    SessionControl, SessionId, SessionInfo, UserId, Verdict,
};
use crate::state::{ArmStep, MethodId, Secret, SessionEvent};

/// What the fake saw, in order, while hardening was applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyEvent {
    JournalDurable {
        entries: Vec<JournalEntry>,
        settings: Vec<(PolicySetting, Option<String>)>,
    },
    SettingApplied {
        target: PolicySetting,
        value: String,
    },
}

/// True only when every setting change is preceded by a durable journal record
/// of that same target. A log that applies first returns false.
pub fn journal_is_durable_before_apply(events: &[ApplyEvent]) -> bool {
    let mut durable = std::collections::BTreeSet::new();
    let mut applied = false;
    for event in events {
        match event {
            ApplyEvent::JournalDurable { entries, .. } => {
                if applied {
                    return false;
                }
                for entry in entries {
                    durable.insert(entry.target.clone());
                }
            }
            ApplyEvent::SettingApplied { target, .. } => {
                applied = true;
                if !durable.contains(target.id()) {
                    return false;
                }
            }
        }
    }
    applied
}

#[derive(Clone, Debug)]
pub struct FakePlatform {
    settings: BTreeMap<PolicySetting, String>,
    apply_log: Vec<ApplyEvent>,
    fail_at: Option<ArmStep>,
    /// A platform flag. It must not change the credential verdict.
    accept_next: bool,
    secret_verdict: Verdict,
    confirmation: Verdict,
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
            apply_log: Vec::new(),
            fail_at: None,
            accept_next: false,
            secret_verdict: Verdict::Rejected,
            confirmation: Verdict::Rejected,
            revert_failed: false,
            guard_armed: false,
            overlay_shown: false,
            power_held: false,
            os_locks: 0,
        }
    }

    pub fn set_setting(&mut self, setting: PolicySetting, value: &str) {
        self.write_policy(setting, value);
    }

    pub fn setting(&self, setting: PolicySetting) -> Option<&str> {
        self.settings.get(&setting).map(String::as_str)
    }

    pub fn fail_step(&mut self, step: ArmStep) {
        self.fail_at = Some(step);
    }

    /// Arms a flag that used to unlock on its own. The verifier ignores it.
    pub fn accept_secrets(&mut self, accept: bool) {
        self.accept_next = accept;
    }

    pub fn set_secret_verdict(&mut self, verdict: Verdict) {
        self.secret_verdict = verdict;
    }

    pub fn set_confirmation(&mut self, verdict: Verdict) {
        self.confirmation = verdict;
    }

    pub fn fail_revert(&mut self, fail: bool) {
        self.revert_failed = fail;
    }

    pub fn apply_events(&self) -> &[ApplyEvent] {
        &self.apply_log
    }

    pub fn os_lock_count(&self) -> u32 {
        self.os_locks
    }

    pub fn power_is_held(&self) -> bool {
        self.power_held
    }

    fn write_policy(&mut self, setting: PolicySetting, value: &str) {
        self.settings.insert(setting, value.to_string());
    }

    fn snapshot(&self) -> Vec<(PolicySetting, Option<String>)> {
        PolicySetting::ALL
            .into_iter()
            .map(|setting| (setting, self.settings.get(&setting).cloned()))
            .collect()
    }

    fn record_apply(&mut self, journal: &Journal, plan: &HardeningPlan) {
        let durable = journal.is_flushed()
            && journal.durable_entries().len() == journal.entries().len()
            && !journal.durable_entries().is_empty();
        if durable {
            self.apply_log.push(ApplyEvent::JournalDurable {
                entries: journal.durable_entries().to_vec(),
                settings: self.snapshot(),
            });
        }
        for change in &plan.changes {
            let Some(setting) = PolicySetting::from_id(&change.target) else {
                continue;
            };
            self.write_policy(setting, &change.new_value);
            self.apply_log.push(ApplyEvent::SettingApplied {
                target: setting,
                value: change.new_value.clone(),
            });
        }
    }
}

impl PolicySettings for FakePlatform {
    fn policy(&self, setting: PolicySetting) -> Option<String> {
        self.settings.get(&setting).cloned()
    }

    fn set_policy(&mut self, setting: PolicySetting, value: &str) {
        self.write_policy(setting, value);
    }

    fn clear_policy(&mut self, setting: PolicySetting) {
        self.settings.remove(&setting);
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
        GuardHealth {
            hook_alive: self.guard_armed,
            last_physical_input_ms: None,
        }
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
        OverlayHealth {
            covered_monitors: u32::from(self.overlay_shown),
            capture_excluded: self.overlay_shown,
            topmost: self.overlay_shown,
        }
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
        self.record_apply(journal, plan);
        Ok(())
    }

    fn revert(&mut self, journal: &Journal) -> Result<(), HardenError> {
        if self.revert_failed {
            return Err(HardenError::Failed);
        }
        for entry in journal.entries() {
            let Some(setting) = PolicySetting::from_id(&entry.target) else {
                continue;
            };
            match &entry.previous {
                Some(value) => self.write_policy(setting, value),
                None => {
                    self.settings.remove(&setting);
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

impl ConfirmationCheck for FakePlatform {
    fn confirm(&mut self, _method: MethodId) -> Verdict {
        self.confirmation
    }
}

impl Credentials for FakePlatform {
    fn verify_password(&self, _user: &UserId, _password: &Secret) -> Verdict {
        self.secret_verdict
    }

    fn verify_secret(&self, _method: &MethodId, _secret: &Secret) -> Verdict {
        let _platform_flag = self.accept_next;
        self.secret_verdict
    }
}

impl EncryptionStatus for FakePlatform {
    fn system_volume(&self) -> DiskEncryption {
        DiskEncryption::Off
    }
}
