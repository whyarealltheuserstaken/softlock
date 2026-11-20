//! Shared lock logic. No operating-system calls.
//!
//! The lock controller decides state from events. Platform traits and the
//! in-memory fake stand in for an operating system, so this crate never locks
//! a machine.

mod backoff;
mod config;
mod controller;
mod decision;
mod dispatch;
#[cfg(feature = "test-support")]
mod fake;
mod heartbeat;
mod hostmsg;
mod ipc;
mod journal;
mod name;
mod oslock;
mod pin;
mod platform;
mod settings_journal;
mod state;

pub use backoff::delay_after_failure;
pub use config::Config;
pub use controller::Event;
pub use controller::{LockController, Rejection};
pub use decision::{PinAuthority, PinDecision};
#[cfg(feature = "test-support")]
pub use fake::{ApplyEvent, FakePlatform, journal_is_durable_before_apply};
pub use heartbeat::{
    ARM_STEP_TIMEOUT, HEARTBEAT_INTERVAL, HEARTBEAT_MISSES_BEFORE_FAILURE, SERVICE_SILENCE_LIMIT,
    lock_host_fails_closed_when_service_is_silent,
};
pub use hostmsg::{
    HostToService, NonceBytes, ServiceToHost, decode_host_frame, decode_service_frame,
    encode_service, host_guard_down_frame, host_hard_lock_frame, host_heartbeat_frame,
    host_hello_frame, host_ready_frame, host_secret_frame,
};
pub use ipc::{
    DecodedRequest, IPC_VERSION, IpcError, MAX_FRAME, RequestKind, ResponseKind, checked_body_len,
    decode_request_frame, encode_frame, message_json, read_prefixed_frame, write_prefixed_frame,
};
pub use journal::{Journal, JournalEntry};
pub use name::{
    APP_BUNDLE_ID, APP_PACKAGE_NAME, PRODUCT_DISPLAY_NAME, SERVICE_DISPLAY_NAME, SERVICE_ID,
    SERVICE_NAME, pipe_name, registry_key,
};
pub use oslock::{
    Attempt, CONFIRM_BUDGET, Caller, Expect, Method, NoticeKind, NoticeLog, Observation, OsLockOps,
    Outcome, StepFault, caller_for_service, confirmed, console_notice, fail_closed,
    failed_closed_log, policy_seam, session_for_expect, stays_guarded, tick_is_on_or_after,
    within_budget,
};
pub use pin::{
    MIN_PIN_LEN, PIN_MEMORY_KIB, PIN_PARALLELISM, PIN_SALT_LEN, PIN_TIME_COST, PinHashError,
    hash_softlock_pin, verify_softlock_pin,
};
pub use platform::{
    AwakeGuard, AwakeRequest, CapabilityReport, Credentials, DiskEncryption, EncryptionStatus,
    GuardError, GuardHealth, HardenError, Hardening, HardeningConfig, HardeningPlan, InputGuard,
    LockScreenLook, OsLockError, Overlay, OverlayError, OverlayHealth, PassthroughTarget,
    PhysicalInputSink, PlannedChange, Platform, PolicySetting, PolicySettings, Power, PowerCaps,
    PowerError, PromptView, SessionControl, SessionId, SessionInfo, Support,
    UNSUPPORTED_PLATFORM_MESSAGE, UnsupportedPlatform, UserId, Verdict,
};
pub use settings_journal::{
    CommitFault, DirFacts, DirFault, DirOwner, HostPresence, JournalFault, ProductState,
    REASSERT_AFTER, RepairAction, SettingEntry, SettingValue, SettingsJournal, StartAction,
    TASK_MANAGER, admit_directory, allowed_previous, commit_then_change, dir_fault_log,
    encode_document, executable_may_run_as_system, expand_profile_image, fault_log,
    hive_target_allowed, journal_dacl_exact, parent_dacl_trusted, parse_document, reassert_due,
    repair_action, repair_entry_name, restore_action, restore_sid, same_exe_path,
    service_start_action, sid_is_safe, task_manager_document,
};
pub use state::{
    ArmStep, DisarmStep, FailReason, Fallback, HeartbeatReport, LaunchNonce, LockSource, LockState,
    MethodId, PromptState, Role, Secret, SessionEvent, StateName,
};
