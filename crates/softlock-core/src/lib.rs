//! Shared lock logic. No operating-system calls.
//!
//! The lock controller decides state from events. Platform traits and the
//! in-memory fake stand in for an operating system, so this crate never locks
//! a machine.

mod backoff;
mod config;
mod controller;
mod dispatch;
#[cfg(feature = "test-support")]
mod fake;
mod heartbeat;
mod ipc;
mod journal;
mod name;
mod pin;
mod platform;
mod state;

pub use backoff::delay_after_failure;
pub use config::Config;
pub use controller::Event;
pub use controller::{LockController, Rejection};
#[cfg(feature = "test-support")]
pub use fake::{ApplyEvent, FakePlatform, journal_is_durable_before_apply};
pub use heartbeat::{
    ARM_STEP_TIMEOUT, HEARTBEAT_INTERVAL, HEARTBEAT_MISSES_BEFORE_FAILURE, SERVICE_SILENCE_LIMIT,
    lock_host_fails_closed_when_service_is_silent,
};
pub use ipc::{
    DecodedRequest, IPC_VERSION, IpcError, MAX_FRAME, RequestKind, ResponseKind, checked_body_len,
    decode_request_frame, encode_frame, message_json,
};
pub use journal::{Journal, JournalEntry};
pub use name::{
    APP_BUNDLE_ID, APP_PACKAGE_NAME, PRODUCT_DISPLAY_NAME, SERVICE_DISPLAY_NAME, SERVICE_ID,
    SERVICE_NAME, pipe_name, registry_key,
};
pub use pin::{
    PIN_MEMORY_KIB, PIN_PARALLELISM, PIN_SALT_LEN, PIN_TIME_COST, PinHashError, hash_softlock_pin,
    verify_softlock_pin,
};
pub use platform::{
    AwakeGuard, AwakeRequest, CapabilityReport, Credentials, DiskEncryption, EncryptionStatus,
    GuardError, GuardHealth, HardenError, Hardening, HardeningConfig, HardeningPlan, InputGuard,
    LockScreenLook, OsLockError, Overlay, OverlayError, OverlayHealth, PassthroughTarget,
    PhysicalInputSink, PlannedChange, Platform, PolicySetting, PolicySettings, Power, PowerCaps,
    PowerError, PromptView, SessionControl, SessionId, SessionInfo, Support,
    UNSUPPORTED_PLATFORM_MESSAGE, UnsupportedPlatform, UserId, Verdict,
};
pub use state::{
    ArmStep, DisarmStep, FailReason, Fallback, HeartbeatReport, LaunchNonce, LockSource, LockState,
    MethodId, PromptState, Role, Secret, SessionEvent, StateName,
};
