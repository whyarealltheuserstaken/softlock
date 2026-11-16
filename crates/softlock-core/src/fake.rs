//! In-memory platform. It records what the controller asked for and never locks a machine.
//!
//! Fault injection (`fail_step`, `fail_revert`, `accept_secrets`, manual verdicts)
//! lives here. Those knobs are not part of the platform traits.

use std::collections::BTreeMap;

use crate::journal::{Journal, JournalEntry};
use crate::platform::{
    AwakeGuard, AwakeRequest, ConfirmationCheck, Credentials, DiskEncryption, EncryptionStatus,
    GuardError, GuardHealth, HardenError, Hardening, HardeningPlan, InputGuard, LockScreenLook,
    OsLockError, Overlay, OverlayError, OverlayHealth, PassthroughTarget, PhysicalInputSink,
    PolicySetting, PolicySettings, Power, PowerCaps, PowerError, PromptView, SessionControl,
    SessionId, UserId, Verdict,
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
    /// A platform flag. Verification does not read it.
    accept_next: bool,
    extras: BTreeMap<String, String>,
    console: SessionId,
    os_lock_fails: bool,
    last_os_lock: Option<SessionId>,
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
            extras: BTreeMap::new(),
            console: SessionId::from_raw(1),
            os_lock_fails: false,
            last_os_lock: None,
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

    /// Arms a flag that must not unlock on its own.
    pub fn accept_secrets(&mut self, accept: bool) {
        self.accept_next = accept;
    }

    pub fn secrets_flag(&self) -> bool {
        self.accept_next
    }

    /// Instance id a backend journals itself. The controller does not know it.
    pub const DEVICE_INSTANCE: &'static str = "HID\\TOUCH&COL01";

    pub fn extra(&self, target: &str) -> Option<&str> {
        self.extras.get(target).map(String::as_str)
    }

    pub fn set_console_session(&mut self, session: SessionId) {
        self.console = session;
    }

    pub fn last_os_lock_session(&self) -> Option<SessionId> {
        self.last_os_lock
    }

    pub fn fail_os_lock(&mut self, fail: bool) {
        self.os_lock_fails = fail;
    }

    /// Changes settings without a durable journal. The order check must fail.
    pub fn apply_settings_before_journal(&mut self, journal: &Journal, plan: &HardeningPlan) {
        self.record_apply(journal, plan);
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

    pub fn guard_is_armed(&self) -> bool {
        self.guard_armed
    }

    pub fn overlay_is_shown(&self) -> bool {
        self.overlay_shown
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
    fn console_session(&self) -> SessionId {
        self.console
    }

    fn os_lock(&mut self, session: SessionId) -> Result<(), OsLockError> {
        self.last_os_lock = Some(session);
        if self.os_lock_fails {
            return Err(OsLockError::Failed);
        }
        self.os_locks = self.os_locks.saturating_add(1);
        Ok(())
    }

    fn session_events(&self) -> std::sync::mpsc::Receiver<SessionEvent> {
        let (_sender, receiver) = std::sync::mpsc::channel();
        receiver
    }
}

impl Hardening for FakePlatform {
    fn apply(&mut self, plan: &HardeningPlan, journal: &mut Journal) -> Result<(), HardenError> {
        if self.fail_at == Some(ArmStep::Hardening) {
            return Err(HardenError::Failed);
        }
        for change in &plan.changes {
            let previous = PolicySetting::from_id(&change.target)
                .and_then(|setting| PolicySettings::policy(self, setting));
            journal.record(JournalEntry {
                target: change.target.clone(),
                previous,
                new_value: change.new_value.clone(),
            });
        }
        journal.record(JournalEntry {
            target: Self::DEVICE_INSTANCE.to_string(),
            previous: self.extras.get(Self::DEVICE_INSTANCE).cloned(),
            new_value: "blocked".to_string(),
        });
        journal.flush();
        self.record_apply(journal, plan);
        self.extras
            .insert(Self::DEVICE_INSTANCE.to_string(), "blocked".to_string());
        Ok(())
    }

    fn revert(&mut self, journal: &Journal) -> Result<(), HardenError> {
        if self.revert_failed {
            return Err(HardenError::Failed);
        }
        for entry in journal.entries() {
            if let Some(setting) = PolicySetting::from_id(&entry.target) {
                match &entry.previous {
                    Some(value) => self.write_policy(setting, value),
                    None => {
                        self.settings.remove(&setting);
                    }
                }
            } else {
                match &entry.previous {
                    Some(value) => {
                        self.extras.insert(entry.target.clone(), value.clone());
                    }
                    None => {
                        self.extras.remove(&entry.target);
                    }
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
        self.secret_verdict
    }
}

impl EncryptionStatus for FakePlatform {
    fn system_volume(&self) -> DiskEncryption {
        DiskEncryption::Off
    }
}
