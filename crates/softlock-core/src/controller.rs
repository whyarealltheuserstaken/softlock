//! Lock state machine. Events go in, decisions come out. Nothing here calls an OS.

use std::collections::VecDeque;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::{ConfigError, ConfigStore};
use crate::heartbeat::{ARM_STEP_TIMEOUT, HEARTBEAT_INTERVAL, HEARTBEAT_MISSES_BEFORE_FAILURE};
use crate::ipc::{RequestKind, ResponseKind};
use crate::journal::Journal;
use crate::platform::{
    AwakeGuard, AwakeRequest, Credentials, DisplayMode, Hardening, InputGuard, LockScreenLook,
    Overlay, PhysicalInputSink, Platform, Power, SessionControl, Verdict, standard_hardening_plan,
};
use crate::state::{
    ArmStep, DisarmStep, FailReason, Fallback, HeartbeatReport, LaunchNonce, LockSource, LockState,
    MethodId, MonotonicInstant, PromptState, Role, Secret, SessionEvent,
};

/// One pump iteration per arming step: journal, guard, overlay, hardening, power.
const ARM_STEP_LIMIT: usize = 5;

/// Trace and rejection logs keep the newest entries only.
const HISTORY_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    pub state: crate::state::StateName,
    pub event: &'static str,
}

#[derive(Debug)]
pub enum Event {
    LockRequested {
        source: LockSource,
    },
    Public {
        role: Role,
        kind: RequestKind,
    },
    Verified {
        nonce: LaunchNonce,
    },
    PromptOpened {
        method: MethodId,
    },
    PassthroughStarted {
        method: MethodId,
    },
    VerificationStarted {
        method: MethodId,
    },
    RequestHello,
    /// Stores a nonce the Service already drew. Does not draw a new one.
    AdoptLaunchNonce {
        nonce: LaunchNonce,
    },
    Session(SessionEvent),
    OsLockConfirmed,
    SubmitSecret {
        method: MethodId,
        secret: Secret,
    },
    AdvanceTime(Duration),
    HostExited,
    /// The guard is down and the host can no longer hold the session.
    GuardDown,
    /// Heartbeats stopped. Same recovery as a dead host.
    HostSilent,
    Heartbeat(HeartbeatReport),
    ServiceStarted {
        journal: Journal,
        session_active: bool,
    },
    ServiceStopRequested,
    /// A watched policy changed under us. Re-assert while locked.
    PolicyChanged,
    /// Administrator repair. Reverts a leftover journal and refuses while locked.
    Repair,
    /// The platform could not put a locked setting back.
    ReassertFailed,
}

pub struct LockController<P> {
    platform: P,
    state: LockState,
    trace: VecDeque<LockState>,
    #[cfg(feature = "test-support")]
    manual: bool,
    journal: Journal,
    rejections: VecDeque<Rejection>,
    failure_events: VecDeque<FailReason>,
    failure_count: u32,
    backoff_until: Option<MonotonicInstant>,
    now: MonotonicInstant,
    last_heartbeat: Option<MonotonicInstant>,
    last_heartbeat_seq: Option<u64>,
    last_reassert: Option<MonotonicInstant>,
    step_started: Option<MonotonicInstant>,
    session_os_locked: bool,
    broker_launched: bool,
    broker_nonce: Option<LaunchNonce>,
    /// The last `os_lock` call failed. `FailingClosed` may then take the
    /// same nonce-checked unlock as `Locked`. A call that succeeded leaves
    /// this false, and `FailingClosed` waits for the OS lock instead.
    os_lock_unavailable: bool,
    os_lock_attempts: u32,
    /// The OS lock did not engage and the host is not holding the session.
    /// The Service respawns the Lock Host. This never means Unlocked.
    recovery_due: bool,
    config: ConfigStore,
    awake: Option<AwakeGuard>,
    last_response: Option<ResponseKind>,
    notifications: VecDeque<Fallback>,
}

impl<P: Platform> LockController<P> {
    pub fn new(platform: P) -> Self {
        Self {
            platform,
            state: LockState::Unlocked,
            trace: VecDeque::from([LockState::Unlocked]),
            #[cfg(feature = "test-support")]
            manual: false,
            journal: Journal::empty(),
            rejections: VecDeque::new(),
            failure_events: VecDeque::new(),
            failure_count: 0,
            backoff_until: None,
            now: MonotonicInstant::zero(),
            last_heartbeat: None,
            last_heartbeat_seq: None,
            last_reassert: None,
            step_started: None,
            session_os_locked: false,
            broker_launched: false,
            broker_nonce: None,
            os_lock_unavailable: false,
            os_lock_attempts: 0,
            recovery_due: false,
            config: ConfigStore::new(),
            awake: None,
            last_response: None,
            notifications: VecDeque::new(),
        }
    }

    pub fn handle(&mut self, event: Event) {
        match event {
            Event::LockRequested { source } => self.on_lock_requested(source),
            Event::Public { role, kind } => {
                self.last_response = Some(self.dispatch(role, kind));
            }
            Event::Verified { nonce } => self.on_verified(nonce),
            Event::PromptOpened { method } => self.on_prompt(PromptState::Open { method }),
            Event::PassthroughStarted { method } => {
                self.on_prompt(PromptState::Passthrough { method });
            }
            Event::VerificationStarted { method } => {
                self.on_prompt(PromptState::Verifying { method });
            }
            Event::RequestHello => self.on_request_hello(),
            Event::AdoptLaunchNonce { nonce } => self.on_adopt_nonce(nonce),
            Event::Session(notification) => self.on_session(notification),
            Event::OsLockConfirmed => self.on_os_lock_confirmed(),
            Event::SubmitSecret { method, secret } => self.on_submit_secret(method, secret),
            Event::AdvanceTime(by) => self.advance(by),
            Event::HostExited => self.on_host_lost(FailReason::HostExited),
            Event::GuardDown => self.on_host_lost(FailReason::GuardUnhealthy),
            Event::HostSilent => self.on_host_lost(FailReason::HeartbeatMissed),
            Event::Heartbeat(report) => self.on_heartbeat(report),
            Event::ServiceStarted {
                journal,
                session_active,
            } => {
                self.on_service_started(journal, session_active);
            }
            Event::ServiceStopRequested => self.on_stop(),
            Event::PolicyChanged => self.on_policy_changed(),
            Event::Repair => self.on_repair(),
            Event::ReassertFailed => {
                if matches!(self.state, LockState::Locked { .. }) {
                    self.fail_closed(FailReason::RevertFailed);
                }
            }
        }
    }

    #[cfg(feature = "test-support")]
    pub fn drive_manually(&mut self) {
        self.manual = true;
    }

    pub fn state(&self) -> &LockState {
        &self.state
    }

    /// True after `os_lock` has failed and until a later call succeeds.
    pub fn os_lock_unavailable(&self) -> bool {
        self.os_lock_unavailable
    }

    /// True after an OS lock failed while the host is not holding the session.
    /// The Service respawns the Lock Host and retries the OS lock.
    pub fn recovery_due(&self) -> bool {
        self.recovery_due
    }

    pub fn acknowledge_recovery(&mut self) {
        self.recovery_due = false;
    }

    pub fn os_lock_attempts(&self) -> u32 {
        self.os_lock_attempts
    }

    pub fn visited(&self) -> &VecDeque<LockState> {
        &self.trace
    }

    pub fn returned_to_unlocked(&self) -> bool {
        let mut left = false;
        for state in &self.trace {
            if matches!(state, LockState::Unlocked) {
                if left {
                    return true;
                }
            } else {
                left = true;
            }
        }
        false
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn platform(&self) -> &P {
        &self.platform
    }

    pub fn platform_mut(&mut self) -> &mut P {
        &mut self.platform
    }

    pub fn rejection_log(&self) -> &VecDeque<Rejection> {
        &self.rejections
    }

    pub fn failure_events(&self) -> &VecDeque<FailReason> {
        &self.failure_events
    }

    pub fn failure_count(&self) -> u32 {
        self.failure_count
    }

    pub fn backoff_delay(&self) -> Duration {
        match self.backoff_until {
            Some(until) if self.now < until => until.saturating_duration_since(self.now),
            _ => Duration::ZERO,
        }
    }

    pub fn session_os_locked(&self) -> bool {
        self.session_os_locked
    }

    pub fn broker_launched(&self) -> bool {
        self.broker_launched
    }

    pub fn broker_nonce(&self) -> Option<LaunchNonce> {
        self.broker_nonce.clone()
    }

    pub fn next_heartbeat_seq(&self) -> u64 {
        self.last_heartbeat_seq
            .map_or(1, |seq| seq.saturating_add(1))
    }

    /// Completes the current arming step. Tests use this to show the timeout
    /// belongs to each step.
    #[cfg(feature = "test-support")]
    pub fn finish_arm_step(&mut self) {
        self.one_arm_step();
    }

    pub fn config_revision(&self) -> u64 {
        self.config.revision()
    }

    /// Install the on-disk document as the current revision. Does not bump.
    pub fn install_startup_config(&mut self, toml: &str) -> Result<(), Vec<String>> {
        self.config.install_startup(toml)
    }

    /// The ONLOGON repair task already reverted the journal. Leave `OsLocked`.
    pub fn release_after_repair(&mut self) {
        if matches!(self.state, LockState::OsLocked { .. }) {
            self.session_os_locked = false;
            self.broker_launched = false;
            self.enter(LockState::Unlocked);
        }
    }

    pub fn config_toml(&self) -> &str {
        self.config.document()
    }

    pub fn config_hotkey(&self) -> &str {
        self.config.hotkey()
    }

    pub fn config_message(&self) -> &str {
        self.config.message()
    }

    pub fn last_response(&self) -> Option<&ResponseKind> {
        self.last_response.as_ref()
    }

    pub fn notifications(&self) -> &VecDeque<Fallback> {
        &self.notifications
    }

    fn automatic(&self) -> bool {
        #[cfg(feature = "test-support")]
        {
            !self.manual
        }
        #[cfg(not(feature = "test-support"))]
        {
            let _ = self;
            true
        }
    }

    fn enter(&mut self, state: LockState) {
        self.state = state.clone();
        push_bounded(&mut self.trace, state);
    }

    fn reject(&mut self, event: &'static str) {
        push_bounded(
            &mut self.rejections,
            Rejection {
                state: self.state.name(),
                event,
            },
        );
    }

    fn on_lock_requested(&mut self, source: LockSource) {
        if !matches!(self.state, LockState::Unlocked) {
            self.reject("LockRequested");
            return;
        }
        self.start_lock(source);
    }

    fn start_lock(&mut self, source: LockSource) {
        self.failure_count = 0;
        self.backoff_until = None;
        self.last_heartbeat_seq = None;
        self.enter_arm(source, ArmStep::Journal);
        if self.automatic() {
            self.pump_arm();
        }
    }

    fn enter_arm(&mut self, source: LockSource, step: ArmStep) {
        self.step_started = Some(self.now);
        self.enter(LockState::Arming { source, step });
    }

    fn pump_arm(&mut self) {
        for _ in 0..ARM_STEP_LIMIT {
            if !matches!(self.state, LockState::Arming { .. }) {
                return;
            }
            self.one_arm_step();
        }
    }

    fn one_arm_step(&mut self) {
        let LockState::Arming { source, step } = self.state.clone() else {
            return;
        };
        match step {
            ArmStep::Journal => {
                self.journal.mark_locked();
                self.journal.flush();
                self.enter_arm(source, ArmStep::Guard);
            }
            ArmStep::Guard => {
                match InputGuard::arm(&mut self.platform, PhysicalInputSink::noop()) {
                    Ok(()) => self.enter_arm(source, ArmStep::Overlay),
                    Err(_) => self.fail_closed(FailReason::ArmStep { step }),
                }
            }
            ArmStep::Overlay => {
                let look = self.lock_look();
                match Overlay::show(&mut self.platform, &look) {
                    Ok(()) => self.enter_arm(source, ArmStep::Hardening),
                    Err(_) => self.fail_closed(FailReason::ArmStep { step }),
                }
            }
            ArmStep::Hardening => match self.journal_and_apply() {
                Ok(()) => self.enter_arm(source, ArmStep::Power),
                Err(_) => self.fail_closed(FailReason::ArmStep { step }),
            },
            ArmStep::Power => match Power::hold(&mut self.platform, AwakeRequest::system()) {
                Ok(guard) => {
                    self.awake = Some(guard);
                    self.journal.set_failures(0);
                    self.journal.flush();
                    self.last_reassert = Some(self.now);
                    self.last_heartbeat = Some(self.now);
                    self.last_heartbeat_seq = None;
                    self.broker_nonce = None;
                    self.broker_launched = false;
                    self.enter(LockState::Locked {
                        source,
                        prompt: PromptState::Idle,
                    });
                }
                Err(_) => self.fail_closed(FailReason::ArmStep { step }),
            },
        }
    }

    fn journal_and_apply(&mut self) -> Result<(), crate::platform::HardenError> {
        let plan = standard_hardening_plan();
        Hardening::apply(&mut self.platform, &plan, &mut self.journal)
    }

    fn on_verified(&mut self, nonce: LaunchNonce) {
        if !self.nonce_unlock_allowed() {
            self.reject("Verified");
            return;
        }
        let genuine = self.broker_launched
            && self
                .broker_nonce
                .as_ref()
                .is_some_and(|expected| expected.eq_ct(&nonce));
        if !genuine {
            self.reject("Verified");
            return;
        }
        // One use. A replay cannot match, including when teardown fails and
        // the machine is still in a locked state.
        self.broker_nonce = None;
        self.broker_launched = false;
        self.begin_unlock();
    }

    /// `Locked`, or `FailingClosed` after the OS lock call failed. A bad or
    /// missing nonce still stays put. `begin_unlock` is the only exit.
    fn nonce_unlock_allowed(&self) -> bool {
        matches!(self.state, LockState::Locked { .. })
            || (self.os_lock_unavailable && matches!(self.state, LockState::FailingClosed { .. }))
    }

    fn on_prompt(&mut self, prompt: PromptState) {
        let LockState::Locked { source, .. } = self.state.clone() else {
            self.reject("Prompt");
            return;
        };
        self.enter(LockState::Locked { source, prompt });
    }

    fn begin_unlock(&mut self) {
        self.enter(LockState::Unlocking {
            step: DisarmStep::RevertHardening,
        });
        if !self.automatic() {
            return;
        }
        if self.teardown() {
            self.broker_launched = false;
            self.enter(LockState::Unlocked);
        } else {
            self.fail_closed(FailReason::RevertFailed);
        }
    }

    fn teardown(&mut self) -> bool {
        if Hardening::revert(&mut self.platform, &self.journal).is_err() {
            return false;
        }
        self.release_guard_and_overlay();
        self.journal.clear();
        self.failure_count = 0;
        self.backoff_until = None;
        true
    }

    fn release_guard_and_overlay(&mut self) {
        self.awake = None;
        Overlay::hide(&mut self.platform);
        InputGuard::disarm(&mut self.platform);
    }

    /// The Windows service calls this after a lock that ran outside the
    /// authority mutex has been confirmed. A guarded `Locked` or `Arming`
    /// session keeps a recorded failure: clearing it there would look like
    /// the OS lock had already succeeded.
    pub fn note_os_lock_held(&mut self) {
        if os_lock_held_clears(&self.state) {
            self.os_lock_unavailable = false;
            self.recovery_due = false;
        }
    }

    /// Test setup. Marks the OS lock down without leaving the current state.
    #[cfg(feature = "test-support")]
    pub fn test_mark_os_lock_down(&mut self) {
        self.os_lock_unavailable = true;
        self.recovery_due = true;
    }

    fn request_os_lock(&mut self) {
        self.os_lock_attempts = self.os_lock_attempts.saturating_add(1);
        let session = SessionControl::console_session(&self.platform);
        if SessionControl::os_lock(&mut self.platform, session).is_err() {
            self.os_lock_unavailable = true;
            self.reject("OsLock");
        } else {
            self.os_lock_unavailable = false;
        }
    }

    fn fail_closed(&mut self, reason: FailReason) {
        push_bounded(&mut self.failure_events, reason);
        self.request_os_lock();
        self.enter(LockState::FailingClosed { reason });
        // A successful OS lock is confirmation, not another guarded state.
        // Sitting in FailingClosed here would arm a replacement host while
        // the workstation is already locked.
        if self.os_lock_unavailable {
            self.recovery_due = true;
        } else {
            self.recovery_due = false;
            self.on_os_lock_confirmed();
        }
    }

    fn on_os_lock_confirmed(&mut self) {
        let LockState::FailingClosed { reason } = self.state.clone() else {
            self.reject("OsLockConfirmed");
            return;
        };
        // Revert the policies, and keep a durable OsLocked marker. A restart
        // before the owner returns must not come back Unlocked.
        self.platform.keep_os_lock_after_revert(true);
        let reverted = self.teardown();
        self.platform.keep_os_lock_after_revert(false);
        if !reverted {
            self.reject("Revert");
        }
        self.enter(LockState::OsLocked {
            after_fail: Some(reason),
        });
    }

    fn on_adopt_nonce(&mut self, nonce: LaunchNonce) {
        if !self.nonce_unlock_allowed() {
            self.reject("AdoptLaunchNonce");
            return;
        }
        self.broker_launched = true;
        self.broker_nonce = Some(nonce);
    }

    fn on_request_hello(&mut self) {
        if !self.nonce_unlock_allowed() {
            self.reject("RequestHello");
            return;
        }
        match LaunchNonce::generate() {
            Ok(nonce) => {
                self.broker_launched = true;
                self.broker_nonce = Some(nonce);
            }
            Err(_) => {
                self.broker_launched = false;
                self.broker_nonce = None;
                self.reject("RequestHello");
            }
        }
    }

    /// Host gone, heartbeats missed, or the guard dropped. A repeat while
    /// already `FailingClosed` retries the OS lock. Success leaves `OsLocked`
    /// and does not ask for another armed host. Failure stays guarded.
    /// Neither path enters `Unlocked`.
    fn on_host_lost(&mut self, reason: FailReason) {
        let label = match reason {
            FailReason::GuardUnhealthy => "GuardDown",
            FailReason::HeartbeatMissed => "HeartbeatMissed",
            _ => "HostExited",
        };
        match self.state.clone() {
            LockState::Locked { .. } | LockState::Arming { .. } => self.fail_closed(reason),
            LockState::FailingClosed { .. } => {
                push_bounded(&mut self.failure_events, reason);
                self.request_os_lock();
                if self.os_lock_unavailable {
                    self.recovery_due = true;
                } else {
                    self.recovery_due = false;
                    self.on_os_lock_confirmed();
                }
            }
            _ => self.reject(label),
        }
    }

    /// Stop is not an unlock. `Locked` and `Arming` fail closed, which confirms
    /// the OS lock when that call works. `FailingClosed` confirms it too, and
    /// stays guarded when the call still fails.
    fn on_stop(&mut self) {
        match self.state.clone() {
            LockState::Locked { .. } | LockState::Arming { .. } => {
                self.fail_closed(FailReason::ServiceStop);
            }
            LockState::FailingClosed { .. } => {
                self.request_os_lock();
                if self.os_lock_unavailable {
                    self.recovery_due = true;
                    self.reject("ServiceStopRequested");
                } else {
                    self.recovery_due = false;
                    self.on_os_lock_confirmed();
                }
            }
            _ => self.reject("ServiceStopRequested"),
        }
    }

    fn on_submit_secret(&mut self, method: MethodId, secret: Secret) {
        // `FailingClosed` is still a lock. A wrong PIN has to count, and a
        // PIN during backoff has to be refused, or the OS-lock retry would
        // allow unlimited guessing.
        if !matches!(
            self.state,
            LockState::Locked { .. } | LockState::FailingClosed { .. }
        ) {
            self.reject("SubmitSecret");
            drop(secret);
            return;
        }
        if !self.backoff_delay().is_zero() {
            drop(secret);
            self.reject("SubmitSecret");
            return;
        }
        let verdict = Credentials::verify_secret(&self.platform, &method, &secret);
        drop(secret);
        if verdict == Verdict::Verified && matches!(self.state, LockState::Locked { .. }) {
            self.begin_unlock();
        } else if verdict == Verdict::Verified {
            // A credential verdict carries no nonce. `FailingClosed` leaves
            // only through `RequestHello` and `Verified`, same as `Locked`.
            // Counting this as a failure would punish a correct PIN.
            self.reject("SubmitSecret");
        } else {
            self.note_failure();
        }
    }

    fn note_failure(&mut self) {
        self.failure_count = self.failure_count.saturating_add(1);
        self.journal.set_failures(self.failure_count);
        self.journal.flush();
        let delay = crate::backoff::delay_after_failure(self.failure_count);
        let prompt = if delay.is_zero() {
            self.backoff_until = None;
            PromptState::Idle
        } else {
            let until = self.now.saturating_add(delay);
            self.backoff_until = Some(until);
            PromptState::Backoff {
                until,
                failures: self.failure_count,
            }
        };
        // The delay is recorded for both states. Only `Locked` has a prompt
        // to show it. `FailingClosed` stays `FailingClosed`.
        let LockState::Locked { source, .. } = self.state.clone() else {
            return;
        };
        self.enter(LockState::Locked { source, prompt });
    }

    fn advance(&mut self, by: Duration) {
        self.now = self.now.saturating_add(by);
        self.watch();
    }

    fn watch(&mut self) {
        if matches!(self.state, LockState::Locked { .. })
            && let Some(last) = self.last_heartbeat
        {
            let elapsed = self.now.saturating_duration_since(last);
            let interval = HEARTBEAT_INTERVAL.as_millis().max(1);
            let missed = elapsed.as_millis() / interval;
            if missed >= u128::from(HEARTBEAT_MISSES_BEFORE_FAILURE) {
                self.fail_closed(FailReason::HeartbeatMissed);
                return;
            }
        }
        if matches!(self.state, LockState::Arming { .. })
            && let Some(started) = self.step_started
            && self.now.saturating_duration_since(started) >= ARM_STEP_TIMEOUT
        {
            self.fail_closed(FailReason::ArmTimeout);
            return;
        }
        self.reassert_locked();
    }

    fn reassert_locked(&mut self) {
        if !matches!(self.state, LockState::Locked { .. }) {
            return;
        }
        let Some(last) = self.last_reassert else {
            return;
        };
        if self.now.saturating_duration_since(last) < Duration::from_secs(5) {
            return;
        }
        self.force_reassert();
    }

    fn force_reassert(&mut self) {
        if Hardening::reassert(&mut self.platform, &self.journal).is_err() {
            self.fail_closed(FailReason::RevertFailed);
            return;
        }
        self.last_reassert = Some(self.now);
    }

    fn on_policy_changed(&mut self) {
        if !matches!(self.state, LockState::Locked { .. }) {
            self.reject("PolicyChanged");
            return;
        }
        self.force_reassert();
    }

    fn on_repair(&mut self) {
        if !matches!(self.state, LockState::Unlocked) {
            self.reject("Repair");
            return;
        }
        if self.journal.is_empty() {
            return;
        }
        if !self.teardown() {
            self.fail_closed(FailReason::RevertFailed);
        }
    }

    fn on_heartbeat(&mut self, report: HeartbeatReport) {
        if !matches!(self.state, LockState::Locked { .. }) {
            return;
        }
        let fresh = match self.last_heartbeat_seq {
            Some(previous) => report.seq > previous,
            None => true,
        };
        if !report.guard.hook_alive {
            self.fail_closed(FailReason::GuardUnhealthy);
            return;
        }
        if !report.overlay.capture_excluded
            || !report.overlay.topmost
            || report.overlay.covered_monitors == 0
        {
            self.fail_closed(FailReason::OverlayUnhealthy);
            return;
        }
        if !fresh {
            return;
        }
        self.last_heartbeat_seq = Some(report.seq);
        self.last_heartbeat = Some(self.now);
    }

    fn on_session(&mut self, notification: SessionEvent) {
        match notification {
            SessionEvent::OsLocked => {
                self.session_os_locked = true;
                if matches!(self.state, LockState::Locked { .. }) {
                    self.enter(LockState::OsLocked { after_fail: None });
                }
            }
            SessionEvent::OsUnlocked => {
                self.session_os_locked = false;
                match self.state.clone() {
                    LockState::OsLocked { after_fail } => {
                        if !self.journal.is_empty() && !self.teardown() {
                            self.release_guard_and_overlay();
                            push_bounded(
                                &mut self.notifications,
                                Fallback {
                                    reason: FailReason::Repair,
                                    at_ms: self.now.as_millis(),
                                    at_utc_ms: wall_unix_ms(),
                                },
                            );
                            self.enter(LockState::OsLocked { after_fail });
                            return;
                        }
                        // The in-memory journal is clear. Drop the durable
                        // OsLocked marker too, so a restart does not lock again
                        // after the owner has signed back in. A failed clear
                        // stays OsLocked.
                        if Hardening::revert(&mut self.platform, &self.journal).is_err() {
                            self.release_guard_and_overlay();
                            push_bounded(
                                &mut self.notifications,
                                Fallback {
                                    reason: FailReason::Repair,
                                    at_ms: self.now.as_millis(),
                                    at_utc_ms: wall_unix_ms(),
                                },
                            );
                            self.enter(LockState::OsLocked { after_fail });
                            return;
                        }
                        if let Some(reason) = after_fail {
                            push_bounded(
                                &mut self.notifications,
                                Fallback {
                                    reason,
                                    at_ms: self.now.as_millis(),
                                    at_utc_ms: wall_unix_ms(),
                                },
                            );
                        }
                        self.broker_launched = false;
                        self.enter(LockState::Unlocked);
                    }
                    LockState::Locked { .. } => self.reject("OsUnlocked"),
                    _ => {}
                }
            }
            SessionEvent::Logon => {
                if !matches!(self.state, LockState::Unlocked) {
                    self.reject("Logon");
                }
            }
            SessionEvent::Logoff => {
                if !matches!(self.state, LockState::Unlocked) {
                    self.reject("Logoff");
                }
            }
        }
    }

    fn on_service_started(&mut self, journal: Journal, session_active: bool) {
        if journal.os_held() {
            // Policies are already restored. The console is not active.
            // SPEC section 7: stay OsLocked until the owner signs back in.
            self.enter(LockState::OsLocked {
                after_fail: Some(FailReason::SessionActive),
            });
            return;
        }
        if journal.is_empty() {
            return;
        }
        let says_locked = journal.says_locked();
        self.failure_count = journal.failures();
        self.journal = journal;
        self.enter(LockState::Recovering);
        if !self.automatic() {
            return;
        }
        if session_active && says_locked {
            self.fail_closed(FailReason::SessionActive);
        } else if self.teardown() {
            self.enter(LockState::Unlocked);
        } else {
            self.fail_closed(FailReason::RevertFailed);
        }
    }

    fn dispatch(&mut self, role: Role, kind: RequestKind) -> ResponseKind {
        if let Some(denied) =
            crate::dispatch::denied(role, &kind, &self.state, self.session_os_locked)
        {
            self.reject(request_name(&kind));
            return denied;
        }
        let name = request_name(&kind);
        match kind {
            RequestKind::Hello { .. } => ResponseKind::Hello {
                service_version: env!("CARGO_PKG_VERSION").to_string(),
                state: self.state.name(),
                capabilities: vec!["lock".to_string()],
            },
            RequestKind::GetStatus => ResponseKind::Status {
                state: self.state.name(),
                lock_source: self.state.source(),
                guard: InputGuard::health(&self.platform),
                overlay: Overlay::health(&self.platform),
                last_fallback: self.notifications.back().copied(),
            },
            RequestKind::Lock { source, .. } => {
                self.start_lock(source);
                ResponseKind::Accepted
            }
            RequestKind::HardLock => {
                // Dispatch already rejected every caller except the App while
                // Unlocked and the Lock Host while Locked.
                if matches!(self.state, LockState::Locked { .. }) {
                    self.fail_closed(FailReason::HardLock);
                } else {
                    self.request_os_lock();
                }
                ResponseKind::Accepted
            }
            RequestKind::GetConfig => ResponseKind::Config {
                toml: self.config.document().to_string(),
                revision: self.config.revision(),
            },
            RequestKind::ValidateConfig { toml } => match crate::config::Config::parse(&toml) {
                Ok(_) => ResponseKind::Valid,
                Err(errors) => ResponseKind::Invalid { errors },
            },
            RequestKind::PutConfig {
                toml,
                base_revision,
            } => {
                let fields = sensitive_tables(&toml);
                if !fields.is_empty() {
                    self.reject(name);
                    return ResponseKind::NeedsConfirmation { fields };
                }
                match self.config.apply(toml, base_revision) {
                    Ok(()) => ResponseKind::Applied,
                    Err(ConfigError::Invalid(errors)) => {
                        self.reject(name);
                        ResponseKind::Invalid { errors }
                    }
                    Err(ConfigError::Conflict) => {
                        self.reject(name);
                        ResponseKind::Conflict
                    }
                }
            }
            RequestKind::ConfirmSensitiveChange { .. }
            | RequestKind::SetPin { .. }
            | RequestKind::ClearPin
            | RequestKind::CheckUpdate
            | RequestKind::InstallUpdate { .. } => {
                self.reject(name);
                ResponseKind::Unsupported
            }
            RequestKind::Subscribe => ResponseKind::Accepted,
        }
    }
}

const SENSITIVE_TABLES: [&str; 3] = ["unlock", "hardening", "takeover"];

/// Top-level `unlock`, `hardening`, and `takeover` keys. A document that does
/// not parse fails closed: every sensitive name is reported so the write is
/// not applied.
fn sensitive_tables(document: &str) -> Vec<String> {
    let Ok(value) = toml::from_str::<toml::Value>(document) else {
        return SENSITIVE_TABLES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
    };
    let Some(table) = value.as_table() else {
        return SENSITIVE_TABLES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
    };
    SENSITIVE_TABLES
        .into_iter()
        .filter(|name| table.contains_key(*name))
        .map(str::to_string)
        .collect()
}

fn request_name(kind: &RequestKind) -> &'static str {
    match kind {
        RequestKind::Hello { .. } => "Hello",
        RequestKind::GetStatus => "GetStatus",
        RequestKind::Lock { .. } => "Lock",
        RequestKind::HardLock => "HardLock",
        RequestKind::GetConfig => "GetConfig",
        RequestKind::ValidateConfig { .. } => "ValidateConfig",
        RequestKind::PutConfig { .. } => "PutConfig",
        RequestKind::ConfirmSensitiveChange { .. } => "ConfirmSensitiveChange",
        RequestKind::SetPin { .. } => "SetPin",
        RequestKind::ClearPin => "ClearPin",
        RequestKind::Subscribe => "Subscribe",
        RequestKind::CheckUpdate => "CheckUpdate",
        RequestKind::InstallUpdate { .. } => "InstallUpdate",
    }
}

impl<P: Platform> LockController<P> {
    fn lock_look(&self) -> LockScreenLook {
        let message = self.config.message();
        LockScreenLook {
            display: DisplayMode::Opaque,
            message: if message.is_empty() {
                None
            } else {
                Some(message.to_string())
            },
            show_clock: true,
        }
    }
}

fn os_lock_held_clears(state: &LockState) -> bool {
    matches!(
        state,
        LockState::FailingClosed { .. } | LockState::OsLocked { .. } | LockState::Unlocked
    )
}

fn wall_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn push_bounded<T>(items: &mut VecDeque<T>, item: T) {
    if items.len() == HISTORY_LIMIT {
        items.pop_front();
    }
    items.push_back(item);
}
