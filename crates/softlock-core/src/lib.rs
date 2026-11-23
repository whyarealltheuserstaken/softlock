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

pub use backoff::{
    BackoffRecord, BackoffRestore, ClockSample, LockoutPolicy, PasswordGate, PasswordStamp,
    delay_after_failure, password_attempt_limit, password_escalation, password_gate,
    restore_backoff, unreadable_lockout_policy,
};
pub use config::{
    CONFIG_ERRORS_NAME, CONFIG_FILE_NAME, Config, HotkeyChord, MAX_CONFIG_BYTES, UserConfig,
    hotkey_in_document, parse_hotkey, published_schema_json, schema_accepts_document,
};
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
    Outcome, StepFault, caller_for_service, clears_disable_lock_workstation, confirmed,
    console_notice, fail_closed, failed_closed_log, policy_seam, session_for_expect, stays_guarded,
    tick_is_on_or_after, within_budget,
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
    CONFIG_DIR, CONFIG_FILE, CommitFault, DirFacts, DirFault, DirOwner, HeldStart, HostPresence,
    JournalFault, LOCKDOWN, LockIdentity, LockdownPolicy, PolicyHive, ProductState, REASSERT_AFTER,
    RepairAction, SessionFact, SettingEntry, SettingValue, SettingsJournal, StartAction,
    StartupStep, TASK_MANAGER, admit_directory, allowed_previous, allowed_previous_for,
    blocks_owner_sign_in, commit_then_change, config_acl_entry, config_dir_allowed,
    config_dir_entry, config_lock_held, config_path_allowed, confirmed_during_wait,
    console_user_from, dacl_without_owner_write, dir_fault_log, encode_document,
    executable_may_run_as_system, expand_profile_image, fact_after_boot, fact_for_session,
    fault_log, fresh_log, held_start, hive_target_allowed, host_identity, journal_dacl_exact,
    lockdown_document, lockdown_policy, lockdown_target, mark_workstation_clear, original_previous,
    owner_reconnected, parent_dacl_trusted, parse_document, reassert_due, reassert_write,
    recovery_action, release_os_hold, repair_action, repair_entry_name, restore_action,
    restore_action_for, restore_sid, same_exe_path, sddl_allowed, service_start_action,
    sid_is_safe, sign_in_release, startup_steps, task_manager_document, with_config_acl,
    workstation_value_after_attempt,
};
pub use state::{
    ArmStep, DisarmStep, FailReason, Fallback, HeartbeatReport, LaunchNonce, LockSource, LockState,
    MethodId, PromptState, Role, Secret, SessionEvent, StateName,
};
