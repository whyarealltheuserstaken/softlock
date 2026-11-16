//! Platform interfaces. Definitions only: this module does not call an OS.
//! macOS and Linux stubs report [`Support::Unsupported`] for every capability.

use std::sync::mpsc::Receiver;

use serde::{Deserialize, Serialize};

use crate::journal::Journal;
use crate::state::{ArmStep, MethodId, Secret, SessionEvent};

pub const UNSUPPORTED_PLATFORM_MESSAGE: &str = "This platform isn't supported yet";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardHealth {
    pub hook_alive: bool,
    pub last_physical_input_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayHealth {
    pub covered_monitors: u32,
    pub capture_excluded: bool,
    pub topmost: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicalInputSink;

impl PhysicalInputSink {
    pub fn noop() -> Self {
        Self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassthroughTarget {
    pub path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayMode {
    Opaque,
    Dimmed { percent: u8 },
    Transparent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockScreenLook {
    pub display: DisplayMode,
    pub message: Option<String>,
    pub show_clock: bool,
}

impl Default for LockScreenLook {
    fn default() -> Self {
        Self {
            display: DisplayMode::Opaque,
            message: None,
            show_clock: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptView {
    pub title: String,
}

impl PromptView {
    pub fn idle() -> Self {
        Self {
            title: "idle".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: SessionId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserId(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HardeningConfig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedChange {
    pub target: String,
    pub new_value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HardeningPlan {
    pub changes: Vec<PlannedChange>,
}

impl HardeningPlan {
    pub fn empty() -> Self {
        Self {
            changes: Vec::new(),
        }
    }
}

/// Fixed security settings applied while locked. Not a user allowlist.
pub fn standard_hardening_plan() -> HardeningPlan {
    let changes = [
        ("DisableTaskMgr", "1"),
        ("DisableLockWorkstation", "1"),
        ("DisableChangePassword", "1"),
        ("NoLogoff", "1"),
        ("HideFastUserSwitching", "1"),
        ("NoClose", "1"),
        ("ConsentPromptBehaviorAdmin", "1"),
        ("ConsentPromptBehaviorUser", "0"),
        ("DisableTouchPen", "1"),
        ("DisableGameControllers", "1"),
        ("StopAccessibilityInput", "1"),
        ("SuppressNotifications", "1"),
        ("PowerButtonAction", "do-nothing"),
    ]
    .into_iter()
    .map(|(target, new_value)| PlannedChange {
        target: target.to_string(),
        new_value: new_value.to_string(),
    })
    .collect();
    HardeningPlan { changes }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AwakeRequest {
    pub system_required: bool,
    pub display_required: bool,
}

impl AwakeRequest {
    pub fn system() -> Self {
        Self {
            system_required: true,
            display_required: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AwakeGuard {
    active: bool,
}

impl AwakeGuard {
    pub fn inactive() -> Self {
        Self { active: false }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerCaps {
    pub supported: bool,
    pub modern_standby: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskEncryption {
    On,
    Off,
    Unknown,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Verified,
    Rejected,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardError {
    Unsupported,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayError {
    Unsupported,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsLockError {
    Unsupported,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardenError {
    Unsupported,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerError {
    Unsupported,
    Failed,
}

pub trait InputGuard {
    fn arm(&mut self, sink: PhysicalInputSink) -> Result<(), GuardError>;
    fn disarm(&mut self);
    fn set_passthrough(&mut self, target: Option<PassthroughTarget>);
    fn health(&self) -> GuardHealth;
}

pub trait Overlay {
    fn show(&mut self, look: &LockScreenLook) -> Result<(), OverlayError>;
    fn update(&mut self, view: &PromptView);
    fn hide(&mut self);
    fn health(&self) -> OverlayHealth;
}

pub trait SessionControl {
    fn os_lock(&mut self, session: SessionId) -> Result<(), OsLockError>;
    fn session_events(&self) -> Receiver<SessionEvent>;
}

pub trait Hardening {
    fn plan(&self, cfg: &HardeningConfig, session: &SessionInfo) -> HardeningPlan;
    fn apply(&mut self, plan: &HardeningPlan, journal: &mut Journal) -> Result<(), HardenError>;
    fn revert(&mut self, journal: &Journal) -> Result<(), HardenError>;
}

pub trait Power {
    fn hold(&mut self, request: AwakeRequest) -> Result<AwakeGuard, PowerError>;
    fn capabilities(&self) -> PowerCaps;
}

pub trait Credentials {
    fn verify_password(&self, user: &UserId, password: &Secret) -> Verdict;
}

/// Hello or the account password, checked by the Broker the service launched.
pub trait ConfirmationCheck {
    fn confirm(&mut self, method: MethodId) -> Verdict;
}

pub trait EncryptionStatus {
    fn system_volume(&self) -> DiskEncryption;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Unsupported,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityReport {
    pub input_guard: Support,
    pub overlay: Support,
    pub session_control: Support,
    pub hardening: Support,
    pub power: Support,
    pub credentials: Support,
    pub encryption_status: Support,
    pub message: &'static str,
}

impl CapabilityReport {
    pub const fn unsupported() -> Self {
        Self {
            input_guard: Support::Unsupported,
            overlay: Support::Unsupported,
            session_control: Support::Unsupported,
            hardening: Support::Unsupported,
            power: Support::Unsupported,
            credentials: Support::Unsupported,
            encryption_status: Support::Unsupported,
            message: UNSUPPORTED_PLATFORM_MESSAGE,
        }
    }

    pub fn is_supported(&self) -> bool {
        self.input_guard == Support::Ready
            && self.overlay == Support::Ready
            && self.session_control == Support::Ready
            && self.hardening == Support::Ready
            && self.power == Support::Ready
            && self.credentials == Support::Ready
            && self.encryption_status == Support::Ready
    }
}

/// Shared body of the macOS and Linux stubs. Every capability is unsupported.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnsupportedPlatform;

impl UnsupportedPlatform {
    pub fn capability_report(&self) -> CapabilityReport {
        CapabilityReport::unsupported()
    }
}

impl InputGuard for UnsupportedPlatform {
    fn arm(&mut self, _sink: PhysicalInputSink) -> Result<(), GuardError> {
        Err(GuardError::Unsupported)
    }

    fn disarm(&mut self) {}

    fn set_passthrough(&mut self, _target: Option<PassthroughTarget>) {}

    fn health(&self) -> GuardHealth {
        GuardHealth {
            hook_alive: false,
            last_physical_input_ms: None,
        }
    }
}

impl Overlay for UnsupportedPlatform {
    fn show(&mut self, _look: &LockScreenLook) -> Result<(), OverlayError> {
        Err(OverlayError::Unsupported)
    }

    fn update(&mut self, _view: &PromptView) {}

    fn hide(&mut self) {}

    fn health(&self) -> OverlayHealth {
        OverlayHealth {
            covered_monitors: 0,
            capture_excluded: false,
            topmost: false,
        }
    }
}

impl SessionControl for UnsupportedPlatform {
    fn os_lock(&mut self, _session: SessionId) -> Result<(), OsLockError> {
        Err(OsLockError::Unsupported)
    }

    fn session_events(&self) -> Receiver<SessionEvent> {
        let (_sender, receiver) = std::sync::mpsc::channel();
        receiver
    }
}

impl Hardening for UnsupportedPlatform {
    fn plan(&self, _cfg: &HardeningConfig, _session: &SessionInfo) -> HardeningPlan {
        HardeningPlan::empty()
    }

    fn apply(&mut self, _plan: &HardeningPlan, _journal: &mut Journal) -> Result<(), HardenError> {
        Err(HardenError::Unsupported)
    }

    fn revert(&mut self, _journal: &Journal) -> Result<(), HardenError> {
        Err(HardenError::Unsupported)
    }
}

impl Power for UnsupportedPlatform {
    fn hold(&mut self, _request: AwakeRequest) -> Result<AwakeGuard, PowerError> {
        Err(PowerError::Unsupported)
    }

    fn capabilities(&self) -> PowerCaps {
        PowerCaps {
            supported: false,
            modern_standby: false,
        }
    }
}

impl Credentials for UnsupportedPlatform {
    fn verify_password(&self, _user: &UserId, _password: &Secret) -> Verdict {
        Verdict::Rejected
    }
}

impl ConfirmationCheck for UnsupportedPlatform {
    fn confirm(&mut self, _method: MethodId) -> Verdict {
        Verdict::Rejected
    }
}

impl EncryptionStatus for UnsupportedPlatform {
    fn system_volume(&self) -> DiskEncryption {
        DiskEncryption::Unsupported
    }
}

/// What the fake recorded at the start of `Hardening::apply`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyObservation {
    pub durable_entries: Vec<crate::journal::JournalEntry>,
    pub settings_before: std::collections::BTreeMap<String, String>,
}

/// Operations the lock controller needs from an in-memory platform.
pub trait DrivenPlatform: ConfirmationCheck {
    fn fail_at(&self) -> Option<ArmStep>;
    fn settings(&self) -> &std::collections::BTreeMap<String, String>;
    fn settings_mut(&mut self) -> &mut std::collections::BTreeMap<String, String>;
    fn note_apply(&mut self, observation: ApplyObservation);
    fn set_guard_armed(&mut self, armed: bool);
    fn set_overlay_shown(&mut self, shown: bool);
    fn set_power_held(&mut self, held: bool);
    fn record_os_lock(&mut self);
    fn apply_hardened(&mut self, journal: &Journal, plan: &HardeningPlan);
    fn take_accept(&mut self) -> bool;
    fn revert_failed(&self) -> bool;
    fn guard_health(&self) -> GuardHealth;
    fn overlay_health(&self) -> OverlayHealth;
}
