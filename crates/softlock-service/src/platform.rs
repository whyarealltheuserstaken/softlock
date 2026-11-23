//! Platform the Service's controller drives.
//!
//! Arming the real Lock Host is optional. Unit tests leave it unset, so the
//! state machine reaches Locked without a desktop. The Windows server installs
//! a wait that returns only after the Lock Host reports ready.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use softlock_core::{
    AwakeGuard, AwakeRequest, Credentials, DiskEncryption, EncryptionStatus, GuardError,
    GuardHealth, HardenError, Hardening, HardeningPlan, InputGuard, Journal, JournalEntry,
    LockScreenLook, OsLockError, Overlay, OverlayError, OverlayHealth, PassthroughTarget,
    PhysicalInputSink, PolicySetting, PolicySettings, Power, PowerCaps, PowerError, PromptView,
    Secret, SessionControl, SessionEvent, SessionId, UserId, Verdict,
};

/// How [`SessionControl::os_lock`] behaves.
///
/// `Succeeds` reports the lock as held and does not call the operating
/// system. Seam 2 uses that. `System` asks the Lock Host to call
/// `LockWorkStation` and, if that is not confirmed, disconnects the console
/// session. `Fails` is the stub and every test where the OS lock is down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsLock {
    Fails,
    Succeeds,
    System,
}

pub struct ServicePlatform {
    os_lock: OsLock,
    guard: GuardHealth,
    overlay: OverlayHealth,
    arm: Option<ArmWait>,
    #[cfg(windows)]
    settings: Option<crate::settings::SettingsEngine>,
    #[cfg(windows)]
    settings_required: bool,
}

impl ServicePlatform {
    pub fn new(os_lock: OsLock) -> Self {
        Self {
            os_lock,
            guard: GuardHealth {
                hook_alive: false,
                last_physical_input_ms: None,
            },
            overlay: OverlayHealth {
                covered_monitors: 0,
                capture_excluded: false,
                topmost: false,
            },
            arm: None,
            #[cfg(windows)]
            settings: None,
            #[cfg(windows)]
            settings_required: false,
        }
    }

    #[cfg(windows)]
    pub fn install_engine(&mut self, engine: crate::settings::SettingsEngine) {
        self.settings = Some(engine);
    }

    #[cfg(windows)]
    pub fn require_settings(&mut self) {
        self.settings_required = true;
    }

    #[cfg(all(windows, test))]
    pub(crate) fn settings_are_required(&self) -> bool {
        self.settings_required && self.settings.is_none()
    }

    #[cfg(windows)]
    pub fn poll_settings(&mut self) -> Result<(), &'static str> {
        let Some(engine) = self.settings.as_mut() else {
            return Ok(());
        };
        engine.reassert_if_due()
    }

    pub fn set_arm_wait(&mut self, arm: ArmWait) {
        self.arm = Some(arm);
    }

    pub fn arm_wait(&self) -> Option<ArmWait> {
        self.arm.clone()
    }

    /// True when locking has to wait for a Lock Host. The Windows service
    /// installs the slot. Unit tests leave it unset and report `Locked`
    /// with no desktop guard.
    pub fn awaits_host(&self) -> bool {
        self.arm.is_some()
    }

    pub fn set_report(&mut self, guard: GuardHealth, overlay: OverlayHealth) {
        self.guard = guard;
        self.overlay = overlay;
    }
}

/// How long a desktop lock may wait for `HostReady`.
///
/// Fail-closed detection is one second: three missed 250 ms heartbeats, and
/// once that decision is made the OS lock has to confirm inside the same
/// second. A healthy Lock Host reports ready inside that second. Hosted
/// runs arm in tens of milliseconds, and the slow samples stayed under
/// 600 ms, so a shorter cutoff would fail a lock that did arm. One second
/// is that budget. It replaces the old 8 s wait, which left the desktop
/// unguarded for the whole timeout.
pub const ARM_READY_LIMIT: Duration = Duration::from_secs(1);

/// The request thread waits here while another thread watches the Lock Host.
#[derive(Clone)]
pub struct ArmWait {
    inner: Arc<ArmInner>,
}

struct ArmInner {
    slot: Mutex<ArmSlot>,
    cv: Condvar,
}

enum ArmSlot {
    Idle,
    Waiting,
    Ready,
    Failed,
}

impl ArmWait {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ArmInner {
                slot: Mutex::new(ArmSlot::Idle),
                cv: Condvar::new(),
            }),
        }
    }

    /// Marks the slot waiting. Does not block: the caller has to drop the
    /// authority mutex before [`Self::wait_ready`], or the reader cannot
    /// accept `HostReady` and the wait never ends.
    pub fn mark(&self) {
        let mut slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *slot = ArmSlot::Waiting;
        self.inner.cv.notify_all();
    }

    /// Blocks until [`Self::ready`] or the arm deadline. Call this only after
    /// releasing the authority mutex.
    pub fn wait_ready(&self) -> bool {
        let mut slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let deadline = Instant::now() + ARM_READY_LIMIT;
        while matches!(*slot, ArmSlot::Waiting) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            let (next, timeout) = self
                .inner
                .cv
                .wait_timeout(slot, left)
                .unwrap_or_else(|poison| poison.into_inner());
            slot = next;
            if timeout.timed_out() && matches!(*slot, ArmSlot::Waiting) {
                return false;
            }
        }
        matches!(*slot, ArmSlot::Ready)
    }

    /// The host supervisor calls this when it should send Arm.
    pub fn take_request(&self) -> bool {
        let mut slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if matches!(*slot, ArmSlot::Waiting) {
            true
        } else {
            let (next, _) = self
                .inner
                .cv
                .wait_timeout(slot, Duration::from_millis(20))
                .unwrap_or_else(|poison| poison.into_inner());
            slot = next;
            matches!(*slot, ArmSlot::Waiting)
        }
    }

    pub fn is_waiting(&self) -> bool {
        let slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        matches!(*slot, ArmSlot::Waiting)
    }

    pub fn is_ready(&self) -> bool {
        let slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        matches!(*slot, ArmSlot::Ready)
    }

    pub fn ready(&self) {
        let mut slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *slot = ArmSlot::Ready;
        self.inner.cv.notify_all();
    }

    pub fn failed(&self) {
        let mut slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *slot = ArmSlot::Failed;
        self.inner.cv.notify_all();
    }
}

impl Default for ArmWait {
    fn default() -> Self {
        Self::new()
    }
}

impl InputGuard for ServicePlatform {
    fn arm(&mut self, _sink: PhysicalInputSink) -> Result<(), GuardError> {
        if let Some(wait) = &self.arm {
            // The request thread still holds the authority mutex. Waiting
            // here deadlocks the reader, which has to accept HostReady
            // before it may wake this slot.
            wait.mark();
        }
        self.guard.hook_alive = true;
        Ok(())
    }

    fn disarm(&mut self) {
        self.guard.hook_alive = false;
    }

    fn set_passthrough(&mut self, _target: Option<PassthroughTarget>) {}

    fn health(&self) -> GuardHealth {
        self.guard
    }
}

impl Overlay for ServicePlatform {
    fn show(&mut self, _look: &LockScreenLook) -> Result<(), OverlayError> {
        if self.overlay.covered_monitors == 0 {
            self.overlay.covered_monitors = 1;
        }
        self.overlay.capture_excluded = true;
        self.overlay.topmost = true;
        Ok(())
    }

    fn update(&mut self, _view: &PromptView) {}

    fn hide(&mut self) {
        self.overlay.covered_monitors = 0;
        self.overlay.capture_excluded = false;
        self.overlay.topmost = false;
    }

    fn health(&self) -> OverlayHealth {
        self.overlay
    }
}

impl SessionControl for ServicePlatform {
    fn console_session(&self) -> SessionId {
        #[cfg(windows)]
        {
            SessionId::from_raw(crate::windows::console_session())
        }
        #[cfg(not(windows))]
        {
            SessionId::from_raw(1)
        }
    }

    fn os_lock(&mut self, session: SessionId) -> Result<(), OsLockError> {
        match self.os_lock {
            OsLock::Fails => Err(OsLockError::Failed),
            OsLock::Succeeds => {
                let _ = session;
                Ok(())
            }
            OsLock::System => {
                #[cfg(windows)]
                {
                    if crate::stub_os_lock() {
                        return crate::windows::lock_session(session.0);
                    }
                    // The chain runs after this mutex is released. Returning
                    // an error keeps the state fail-closed until `drain_os_lock`
                    // confirms it.
                    crate::windows::schedule_os_lock(session.0);
                    Err(OsLockError::Failed)
                }
                #[cfg(not(windows))]
                {
                    let _ = session;
                    Ok(())
                }
            }
        }
    }

    fn session_events(&self) -> std::sync::mpsc::Receiver<SessionEvent> {
        let (_sender, receiver) = std::sync::mpsc::channel();
        receiver
    }
}

impl Hardening for ServicePlatform {
    fn apply(&mut self, plan: &HardeningPlan, journal: &mut Journal) -> Result<(), HardenError> {
        #[cfg(windows)]
        if self.settings_required && self.settings.is_none() {
            crate::audit("settings", &[("result", "failed"), ("reason", "journal")]);
            return Err(HardenError::Failed);
        }
        #[cfg(windows)]
        if let Some(engine) = self.settings.as_mut() {
            engine.stage(journal)?;
        }
        for change in &plan.changes {
            #[cfg(windows)]
            if self.settings.is_some() && softlock_core::lockdown_target(&change.target) {
                continue;
            }
            journal.record(JournalEntry {
                target: change.target.clone(),
                previous: None,
                new_value: change.new_value.clone(),
            });
        }
        journal.flush();
        #[cfg(windows)]
        if let Some(engine) = self.settings.as_mut() {
            engine.persist_and_apply()?;
        }
        Ok(())
    }

    fn keep_os_lock_after_revert(&mut self, keep: bool) {
        #[cfg(windows)]
        if let Some(engine) = self.settings.as_mut() {
            engine.keep_os_lock_after_revert(keep);
        }
        #[cfg(not(windows))]
        {
            let _ = keep;
        }
    }

    fn revert(&mut self, journal: &Journal) -> Result<(), HardenError> {
        #[cfg(windows)]
        if let Some(engine) = self.settings.as_mut() {
            return engine.revert_memory(journal).map_err(|reason| {
                crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
                HardenError::Failed
            });
        }
        #[cfg(not(windows))]
        {
            let _ = journal;
        }
        Ok(())
    }

    fn reassert(&mut self, _journal: &Journal) -> Result<(), HardenError> {
        #[cfg(windows)]
        if let Some(engine) = self.settings.as_mut() {
            engine.reassert_now().map_err(|reason| {
                crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
                HardenError::Failed
            })?;
        }
        Ok(())
    }
}

impl Power for ServicePlatform {
    fn hold(&mut self, _request: AwakeRequest) -> Result<AwakeGuard, PowerError> {
        Ok(AwakeGuard::inactive())
    }

    fn capabilities(&self) -> PowerCaps {
        PowerCaps {
            supported: true,
            modern_standby: false,
        }
    }
}

impl Credentials for ServicePlatform {
    fn verify_password(&self, _user: &UserId, _password: &Secret) -> Verdict {
        Verdict::Rejected
    }

    fn verify_secret(&self, _method: &softlock_core::MethodId, _secret: &Secret) -> Verdict {
        // A credential verdict carries no nonce. The Service unlocks only
        // through `AdoptLaunchNonce` and `Verified`.
        Verdict::Rejected
    }
}

impl PolicySettings for ServicePlatform {
    fn policy(&self, _setting: PolicySetting) -> Option<String> {
        None
    }

    fn set_policy(&mut self, _setting: PolicySetting, _value: &str) {}

    fn clear_policy(&mut self, _setting: PolicySetting) {}
}

impl EncryptionStatus for ServicePlatform {
    fn system_volume(&self) -> DiskEncryption {
        DiskEncryption::Unknown
    }
}
