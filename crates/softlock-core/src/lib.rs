//! Shared lock logic. No operating-system calls.
//!
//! The lock controller decides state from events. Platform traits and the
//! in-memory fake stand in for an operating system, so this crate never locks
//! a machine.

mod backoff;
mod controller;
mod fake;
mod heartbeat;
mod ipc;
mod journal;
mod name;
mod platform;
mod state;

pub use backoff::delay_after_failure;
pub use controller::Event;
pub use controller::{FallbackNotice, LockController, Rejection};
pub use fake::FakePlatform;
pub use heartbeat::{
    ARM_STEP_TIMEOUT, HEARTBEAT_INTERVAL, HEARTBEAT_MISSES_BEFORE_FAILURE, SERVICE_SILENCE_LIMIT,
    lock_host_fails_closed_when_service_is_silent,
};
pub use ipc::{
    DecodedRequest, IPC_VERSION, IpcError, RequestKind, ResponseKind, decode_request_frame,
    encode_frame, request_json, response_json,
};
pub use journal::{Journal, JournalEntry};
pub use name::{
    APP_BUNDLE_ID, APP_PACKAGE_NAME, PRODUCT_DISPLAY_NAME, SERVICE_DISPLAY_NAME, SERVICE_ID,
    SERVICE_NAME, pipe_name, registry_key,
};
pub use platform::{
    ApplyObservation, AwakeGuard, AwakeRequest, CapabilityReport, Credentials, DiskEncryption,
    EncryptionStatus, GuardError, GuardHealth, HardenError, Hardening, HardeningConfig,
    HardeningPlan, InputGuard, LockScreenLook, OsLockError, Overlay, OverlayError, OverlayHealth,
    PassthroughTarget, PhysicalInputSink, PlannedChange, Power, PowerCaps, PowerError, PromptView,
    SessionControl, SessionId, SessionInfo, Support, UNSUPPORTED_PLATFORM_MESSAGE,
    UnsupportedPlatform, UserId, Verdict,
};
pub use state::{
    ArmStep, DisarmStep, FailReason, FailureEvent, HeartbeatReport, LockSource, LockState,
    MethodId, PromptState, Role, Secret, SessionEvent, StateName, VerdictChannel,
};
