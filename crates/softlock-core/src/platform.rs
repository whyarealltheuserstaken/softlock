//! Platform interfaces. Definitions only: this module does not call an OS.
//! macOS and Linux stubs report [`Support::Unsupported`] for every capability.

use std::sync::mpsc::Receiver;

use serde::{Deserialize, Serialize};

use crate::journal::Journal;
use crate::state::{MethodId, Secret, SessionEvent};

pub const UNSUPPORTED_PLATFORM_MESSAGE: &str = "This platform isn't supported yet";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardHealth {
    pub hook_alive: bool,
    pub last_physical_input_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

impl SessionId {
    pub const UNKNOWN: Self = Self(u32::MAX);

    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

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
    let changes = PolicySetting::ALL
        .into_iter()
        .map(|setting| PlannedChange {
            target: setting.id().to_string(),
            new_value: setting.locked_value().to_string(),
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
    /// The active console session (`WTSGetActiveConsoleSessionId` on Windows).
    fn console_session(&self) -> SessionId;
    fn os_lock(&mut self, session: SessionId) -> Result<(), OsLockError>;
    fn session_events(&self) -> Receiver<SessionEvent>;
}

pub trait Hardening {
    fn apply(&mut self, plan: &HardeningPlan, journal: &mut Journal) -> Result<(), HardenError>;
    fn revert(&mut self, journal: &Journal) -> Result<(), HardenError>;
}

pub trait Power {
    fn hold(&mut self, request: AwakeRequest) -> Result<AwakeGuard, PowerError>;
    fn capabilities(&self) -> PowerCaps;
}

pub trait Credentials {
    fn verify_password(&self, user: &UserId, password: &Secret) -> Verdict;

    /// Forwards one submitted secret. Only [`Verdict::Verified`] may unlock.
    fn verify_secret(&self, method: &MethodId, secret: &Secret) -> Verdict {
        let _ = (method, secret);
        Verdict::Rejected
    }
}

/// Typed policy values. The journal stores these ids, not a string map the
/// controller edits itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PolicySetting {
    DisableTaskMgr,
    DisableLockWorkstation,
    DisableChangePassword,
    NoLogoff,
    HideFastUserSwitching,
    NoClose,
    ConsentPromptBehaviorAdmin,
    ConsentPromptBehaviorUser,
    DisableTouchPen,
    DisableGameControllers,
    StopAccessibilityInput,
    SuppressNotifications,
    PowerButtonAction,
}

impl PolicySetting {
    pub const ALL: [Self; 13] = [
        Self::DisableTaskMgr,
        Self::DisableLockWorkstation,
        Self::DisableChangePassword,
        Self::NoLogoff,
        Self::HideFastUserSwitching,
        Self::NoClose,
        Self::ConsentPromptBehaviorAdmin,
        Self::ConsentPromptBehaviorUser,
        Self::DisableTouchPen,
        Self::DisableGameControllers,
        Self::StopAccessibilityInput,
        Self::SuppressNotifications,
        Self::PowerButtonAction,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::DisableTaskMgr => "DisableTaskMgr",
            Self::DisableLockWorkstation => "DisableLockWorkstation",
            Self::DisableChangePassword => "DisableChangePassword",
            Self::NoLogoff => "NoLogoff",
            Self::HideFastUserSwitching => "HideFastUserSwitching",
            Self::NoClose => "NoClose",
            Self::ConsentPromptBehaviorAdmin => "ConsentPromptBehaviorAdmin",
            Self::ConsentPromptBehaviorUser => "ConsentPromptBehaviorUser",
            Self::DisableTouchPen => "DisableTouchPen",
            Self::DisableGameControllers => "DisableGameControllers",
            Self::StopAccessibilityInput => "StopAccessibilityInput",
            Self::SuppressNotifications => "SuppressNotifications",
            Self::PowerButtonAction => "PowerButtonAction",
        }
    }

    pub fn locked_value(self) -> &'static str {
        match self {
            Self::ConsentPromptBehaviorUser => "0",
            Self::PowerButtonAction => "do-nothing",
            _ => "1",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|setting| setting.id() == id)
    }
}

pub trait PolicySettings {
    fn policy(&self, setting: PolicySetting) -> Option<String>;
    fn set_policy(&mut self, setting: PolicySetting, value: &str);
    fn clear_policy(&mut self, setting: PolicySetting);
}

/// The platform surface from the spec, plus typed policy settings and the
/// broker confirmation check. Fault injection is not part of this trait.
pub trait Platform:
    InputGuard
    + Overlay
    + SessionControl
    + Hardening
    + Power
    + Credentials
    + EncryptionStatus
    + PolicySettings
    + ConfirmationCheck
{
}

impl<T> Platform for T where
    T: InputGuard
        + Overlay
        + SessionControl
        + Hardening
        + Power
        + Credentials
        + EncryptionStatus
        + PolicySettings
        + ConfirmationCheck
{
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
    fn console_session(&self) -> SessionId {
        SessionId::UNKNOWN
    }

    fn os_lock(&mut self, _session: SessionId) -> Result<(), OsLockError> {
        Err(OsLockError::Unsupported)
    }

    fn session_events(&self) -> Receiver<SessionEvent> {
        let (_sender, receiver) = std::sync::mpsc::channel();
        receiver
    }
}

impl Hardening for UnsupportedPlatform {
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

impl PolicySettings for UnsupportedPlatform {
    fn policy(&self, _setting: PolicySetting) -> Option<String> {
        None
    }

    fn set_policy(&mut self, _setting: PolicySetting, _value: &str) {}

    fn clear_policy(&mut self, _setting: PolicySetting) {}
}

impl EncryptionStatus for UnsupportedPlatform {
    fn system_volume(&self) -> DiskEncryption {
        DiskEncryption::Unsupported
    }
}
