//! Lock states, events, and the ids the state machine talks about.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Monotonic clock reading owned by the controller. Tests advance it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MonotonicInstant {
    millis: u64,
}

impl MonotonicInstant {
    pub const fn zero() -> Self {
        Self { millis: 0 }
    }

    pub const fn as_millis(self) -> u64 {
        self.millis
    }

    pub fn saturating_add(self, duration: Duration) -> Self {
        Self {
            millis: self.millis.saturating_add(duration_millis(duration)),
        }
    }

    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration::from_millis(self.millis.saturating_sub(earlier.millis))
    }
}

pub(crate) fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockSource {
    Hotkey,
    Tray,
    App,
    LocalApi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmStep {
    Journal,
    Guard,
    Overlay,
    Hardening,
    Power,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisarmStep {
    RevertHardening,
    ReleasePower,
    HideOverlay,
    DisarmGuard,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MethodId(String);

impl MethodId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn windows_hello() -> Self {
        Self::new("windows_hello")
    }

    pub fn os_password() -> Self {
        Self::new("os_password")
    }

    pub fn softlock_pin() -> Self {
        Self::new("softlock_pin")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A secret the controller drops after the verdict. Debug output is redacted.
pub struct Secret {
    bytes: Vec<u8>,
}

impl Secret {
    pub fn new(value: impl AsRef<[u8]>) -> Self {
        Self {
            bytes: value.as_ref().to_vec(),
        }
    }

    pub fn expose(&self) -> &[u8] {
        &self.bytes
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Secret([redacted])")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateName {
    Unlocked,
    Arming,
    Locked,
    Unlocking,
    FailingClosed,
    OsLocked,
    Recovering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    App,
    LockClient,
    LockHost,
    Broker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerdictChannel {
    LockHost,
    Broker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionEvent {
    OsLocked,
    OsUnlocked,
    Logon,
    Logoff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailReason {
    ArmStep {
        step: ArmStep,
    },
    HeartbeatMissed,
    HostExited,
    GuardUnhealthy,
    OverlayUnhealthy,
    ArmTimeout,
    RevertFailed,
    ServiceStop,
    /// The Lock Host asked for a hard lock while the session was locked.
    HardLock,
    /// Recovery found a live session that still says it is locked.
    SessionActive,
}

/// Why the machine fell back to the OS lock, and when.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fallback {
    pub reason: FailReason,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptState {
    Idle,
    Open {
        method: MethodId,
    },
    Passthrough {
        method: MethodId,
    },
    Verifying {
        method: MethodId,
    },
    Backoff {
        until: MonotonicInstant,
        failures: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockState {
    Unlocked,
    Arming {
        source: LockSource,
        step: ArmStep,
    },
    Locked {
        source: LockSource,
        prompt: PromptState,
    },
    Unlocking {
        step: DisarmStep,
    },
    FailingClosed {
        reason: FailReason,
    },
    OsLocked {
        after_fail: Option<FailReason>,
    },
    Recovering,
}

impl LockState {
    pub fn name(&self) -> StateName {
        match self {
            Self::Unlocked => StateName::Unlocked,
            Self::Arming { .. } => StateName::Arming,
            Self::Locked { .. } => StateName::Locked,
            Self::Unlocking { .. } => StateName::Unlocking,
            Self::FailingClosed { .. } => StateName::FailingClosed,
            Self::OsLocked { .. } => StateName::OsLocked,
            Self::Recovering => StateName::Recovering,
        }
    }

    pub fn source(&self) -> Option<LockSource> {
        match self {
            Self::Arming { source, .. } | Self::Locked { source, .. } => Some(*source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeartbeatReport {
    pub seq: u64,
    pub guard: crate::platform::GuardHealth,
    pub overlay: crate::platform::OverlayHealth,
}

// ArmStep is serialized inside FailReason. It needs serde.
impl Serialize for ArmStep {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Journal => serializer.serialize_str("Journal"),
            Self::Guard => serializer.serialize_str("Guard"),
            Self::Overlay => serializer.serialize_str("Overlay"),
            Self::Hardening => serializer.serialize_str("Hardening"),
            Self::Power => serializer.serialize_str("Power"),
        }
    }
}

impl<'de> Deserialize<'de> for ArmStep {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        match text.as_str() {
            "Journal" => Ok(Self::Journal),
            "Guard" => Ok(Self::Guard),
            "Overlay" => Ok(Self::Overlay),
            "Hardening" => Ok(Self::Hardening),
            "Power" => Ok(Self::Power),
            _ => Err(serde::de::Error::custom("unknown arm step")),
        }
    }
}
