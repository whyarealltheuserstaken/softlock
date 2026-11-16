//! Lock state machine. Events go in, decisions come out. Nothing here calls an OS.

use std::time::Duration;

use crate::heartbeat::{ARM_STEP_TIMEOUT, HEARTBEAT_INTERVAL, HEARTBEAT_MISSES_BEFORE_FAILURE};
use crate::ipc::{RequestKind, ResponseKind, StatusFallback};
use crate::journal::{Journal, JournalEntry};
use crate::platform::{ApplyObservation, DrivenPlatform, standard_hardening_plan};
use crate::state::{
    ArmStep, DisarmStep, FailReason, HeartbeatReport, LockSource, LockState, MethodId,
    MonotonicInstant, PromptState, Role, Secret, SessionEvent, VerdictChannel,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    pub state: crate::state::StateName,
    pub event: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FallbackNotice {
    pub reason: FailReason,
    pub at_ms: u64,
}

#[derive(Debug)]
pub enum Event {
    LockRequested {
        source: LockSource,
    },
    Public {
        role: Role,
        id: u64,
        kind: RequestKind,
    },
    Verified {
        channel: VerdictChannel,
        method: MethodId,
    },
    RequestHello,
    Session(SessionEvent),
    OsLockConfirmed,
    SubmitSecret {
        method: MethodId,
        secret: Secret,
    },
    AdvanceTime(Duration),
    HostExited,
    Heartbeat(HeartbeatReport),
    ServiceStarted {
        journal: Journal,
        session_active: bool,
    },
    ServiceStopRequested,
}

pub struct LockController<P> {
    platform: P,
    state: LockState,
    trace: Vec<LockState>,
    manual: bool,
    journal: Journal,
    rejections: Vec<Rejection>,
    failure_events: Vec<FailReason>,
    failure_count: u32,
    now: MonotonicInstant,
    last_heartbeat: Option<MonotonicInstant>,
    arm_started: Option<MonotonicInstant>,
    session_os_locked: bool,
    broker_launched: bool,
    config_toml: String,
    config_revision: u64,
    last_response: Option<ResponseKind>,
    notifications: Vec<FallbackNotice>,
}

impl<P: DrivenPlatform> LockController<P> {
    pub fn new(platform: P) -> Self {
        Self {
            platform,
            state: LockState::Unlocked,
            trace: vec![LockState::Unlocked],
            manual: false,
            journal: Journal::empty(),
            rejections: Vec::new(),
            failure_events: Vec::new(),
            failure_count: 0,
            now: MonotonicInstant::zero(),
            last_heartbeat: None,
            arm_started: None,
            session_os_locked: false,
            broker_launched: false,
            config_toml: "schema_version = 1\n".to_string(),
            config_revision: 1,
            last_response: None,
            notifications: Vec::new(),
        }
    }

    pub fn handle(&mut self, event: Event) {
        match event {
            Event::LockRequested { source } => self.on_lock_requested(source),
            Event::Public { role, kind, .. } => {
                self.last_response = Some(self.dispatch(role, kind));
            }
            Event::Verified { channel, .. } => self.on_verified(channel),
            Event::RequestHello => self.on_request_hello(),
            Event::Session(notification) => self.on_session(notification),
            Event::OsLockConfirmed => self.on_os_lock_confirmed(),
            Event::SubmitSecret { secret, .. } => self.on_submit_secret(secret),
            Event::AdvanceTime(by) => self.advance(by),
            Event::HostExited => self.on_host_exited(),
            Event::Heartbeat(report) => self.on_heartbeat(report),
            Event::ServiceStarted {
                journal,
                session_active,
            } => {
                self.on_service_started(journal, session_active);
            }
            Event::ServiceStopRequested => self.on_stop(),
        }
    }

    pub fn drive_manually(&mut self) {
        self.manual = true;
    }

    pub fn state(&self) -> &LockState {
        &self.state
    }

    pub fn visited(&self) -> &[LockState] {
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

    pub fn rejection_log(&self) -> &[Rejection] {
        &self.rejections
    }

    pub fn failure_events(&self) -> &[FailReason] {
        &self.failure_events
    }

    pub fn failure_count(&self) -> u32 {
        self.failure_count
    }

    pub fn backoff_delay(&self) -> Duration {
        match &self.state {
            LockState::Locked {
                prompt: PromptState::Backoff { until, .. },
                ..
            } => until.saturating_duration_since(self.now),
            _ => Duration::ZERO,
        }
    }

    pub fn session_os_locked(&self) -> bool {
        self.session_os_locked
    }

    pub fn broker_launched(&self) -> bool {
        self.broker_launched
    }

    pub fn config_revision(&self) -> u64 {
        self.config_revision
    }

    pub fn config_toml(&self) -> &str {
        &self.config_toml
    }

    pub fn last_response(&self) -> Option<&ResponseKind> {
        self.last_response.as_ref()
    }

    pub fn notifications(&self) -> &[FallbackNotice] {
        &self.notifications
    }

    fn enter(&mut self, state: LockState) {
        self.state = state.clone();
        self.trace.push(state);
    }

    fn reject(&mut self, event: &'static str) {
        self.rejections.push(Rejection {
            state: self.state.name(),
            event,
        });
    }

    fn on_lock_requested(&mut self, source: LockSource) {
        if !matches!(self.state, LockState::Unlocked) {
            self.reject("LockRequested");
            return;
        }
        self.start_lock(source);
    }

    fn start_lock(&mut self, source: LockSource) {
        self.arm_started = Some(self.now);
        self.failure_count = 0;
        self.enter(LockState::Arming {
            source,
            step: ArmStep::Journal,
        });
        if !self.manual {
            self.pump_arm();
        }
    }

    fn pump_arm(&mut self) {
        for _ in 0..8 {
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
        if self.platform.fail_at() == Some(step) {
            self.fail_closed(FailReason::ArmStep { step });
            return;
        }
        match step {
            ArmStep::Journal => {
                self.journal.mark_locked();
                self.journal.flush();
                self.enter(LockState::Arming {
                    source,
                    step: ArmStep::Guard,
                });
            }
            ArmStep::Guard => {
                self.platform.set_guard_armed(true);
                self.enter(LockState::Arming {
                    source,
                    step: ArmStep::Overlay,
                });
            }
            ArmStep::Overlay => {
                self.platform.set_overlay_shown(true);
                self.enter(LockState::Arming {
                    source,
                    step: ArmStep::Hardening,
                });
            }
            ArmStep::Hardening => {
                self.journal_and_apply();
                self.enter(LockState::Arming {
                    source,
                    step: ArmStep::Power,
                });
            }
            ArmStep::Power => {
                self.platform.set_power_held(true);
                self.journal.set_failures(0);
                self.journal.flush();
                self.last_heartbeat = Some(self.now);
                self.enter(LockState::Locked {
                    source,
                    prompt: PromptState::Idle,
                });
            }
        }
    }

    fn journal_and_apply(&mut self) {
        let plan = standard_hardening_plan();
        let current = self.platform.settings().clone();
        for change in &plan.changes {
            self.journal.record(JournalEntry {
                target: change.target.clone(),
                previous: current.get(&change.target).cloned(),
                new_value: change.new_value.clone(),
            });
        }
        self.journal.flush();
        self.platform.note_apply(ApplyObservation {
            durable_entries: self.journal.durable_entries().to_vec(),
            settings_before: current,
        });
        for change in &plan.changes {
            self.platform
                .settings_mut()
                .insert(change.target.clone(), change.new_value.clone());
        }
    }

    fn on_verified(&mut self, channel: VerdictChannel) {
        if !matches!(self.state, LockState::Locked { .. }) {
            self.reject("Verified");
            return;
        }
        if channel == VerdictChannel::Broker && !self.broker_launched {
            self.reject("Verified");
            return;
        }
        self.begin_unlock();
    }

    fn begin_unlock(&mut self) {
        self.enter(LockState::Unlocking {
            step: DisarmStep::RevertHardening,
        });
        if self.manual {
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
        if self.platform.revert_failed() {
            return false;
        }
        let entries = self.journal.entries().to_vec();
        for entry in entries {
            match entry.previous {
                Some(value) => {
                    self.platform.settings_mut().insert(entry.target, value);
                }
                None => {
                    self.platform.settings_mut().remove(&entry.target);
                }
            }
        }
        self.platform.set_power_held(false);
        self.platform.set_overlay_shown(false);
        self.platform.set_guard_armed(false);
        self.journal.clear();
        self.failure_count = 0;
        true
    }

    fn fail_closed(&mut self, reason: FailReason) {
        self.failure_events.push(reason);
        self.platform.record_os_lock();
        self.enter(LockState::FailingClosed { reason });
    }

    fn on_os_lock_confirmed(&mut self) {
        let LockState::FailingClosed { reason } = self.state.clone() else {
            self.reject("OsLockConfirmed");
            return;
        };
        let _ = self.teardown();
        self.enter(LockState::OsLocked {
            after_fail: Some(reason),
        });
    }

    fn on_request_hello(&mut self) {
        if matches!(self.state, LockState::Locked { .. }) {
            self.broker_launched = true;
        } else {
            self.reject("RequestHello");
        }
    }

    fn on_host_exited(&mut self) {
        if matches!(
            self.state,
            LockState::Locked { .. } | LockState::Arming { .. }
        ) {
            self.fail_closed(FailReason::HostExited);
        } else {
            self.reject("HostExited");
        }
    }

    fn on_stop(&mut self) {
        if matches!(self.state, LockState::Locked { .. }) {
            self.fail_closed(FailReason::ServiceStop);
        }
    }

    fn on_submit_secret(&mut self, secret: Secret) {
        if !matches!(self.state, LockState::Locked { .. }) {
            self.reject("SubmitSecret");
            drop(secret);
            return;
        }
        if !self.backoff_delay().is_zero() {
            drop(secret);
            return;
        }
        drop(secret);
        if self.platform.take_accept() {
            self.begin_unlock();
        } else {
            self.note_failure();
        }
    }

    fn note_failure(&mut self) {
        self.failure_count = self.failure_count.saturating_add(1);
        self.journal.set_failures(self.failure_count);
        self.journal.flush();
        let delay = crate::backoff::delay_after_failure(self.failure_count);
        let LockState::Locked { source, .. } = self.state.clone() else {
            return;
        };
        let prompt = if delay.is_zero() {
            PromptState::Idle
        } else {
            PromptState::Backoff {
                until: self.now.saturating_add(delay),
                failures: self.failure_count,
            }
        };
        self.state = LockState::Locked { source, prompt };
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
            && let Some(started) = self.arm_started
            && self.now.saturating_duration_since(started) >= ARM_STEP_TIMEOUT
        {
            self.fail_closed(FailReason::ArmTimeout);
        }
    }

    fn on_heartbeat(&mut self, report: HeartbeatReport) {
        if !matches!(self.state, LockState::Locked { .. }) {
            return;
        }
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
                        if !self.journal.is_empty() {
                            let _ = self.teardown();
                        }
                        if let Some(reason) = after_fail {
                            self.notifications.push(FallbackNotice {
                                reason,
                                at_ms: self.now.as_millis(),
                            });
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
        if journal.is_empty() {
            return;
        }
        let says_locked = journal.says_locked();
        self.journal = journal;
        self.enter(LockState::Recovering);
        if self.manual {
            return;
        }
        if session_active && says_locked {
            self.fail_closed(FailReason::ServiceStop);
        } else if self.teardown() {
            self.enter(LockState::Unlocked);
        }
    }

    fn dispatch(&mut self, role: Role, kind: RequestKind) -> ResponseKind {
        match kind {
            RequestKind::Hello { .. } => ResponseKind::Hello {
                service_version: env!("CARGO_PKG_VERSION").to_string(),
                state: self.state.name(),
                capabilities: vec!["lock".to_string()],
            },
            RequestKind::GetStatus => ResponseKind::Status {
                state: self.state.name(),
                lock_source: self.state.source(),
                guard: self.platform.guard_health(),
                overlay: self.platform.overlay_health(),
                last_fallback: self.notifications.last().map(|notice| StatusFallback {
                    reason: notice.reason,
                    at_ms: notice.at_ms,
                }),
            },
            RequestKind::Lock { source, .. } => self.dispatch_lock(role, source),
            RequestKind::HardLock => self.dispatch_hard_lock(role),
            RequestKind::GetConfig => {
                if self.public_reader(role) {
                    ResponseKind::Config {
                        toml: self.config_toml.clone(),
                        revision: self.config_revision,
                    }
                } else {
                    ResponseKind::Unauthorized
                }
            }
            RequestKind::ValidateConfig { toml } => {
                if !self.public_reader(role) {
                    ResponseKind::Unauthorized
                } else if toml.trim().is_empty() {
                    invalid_config()
                } else {
                    ResponseKind::Valid
                }
            }
            RequestKind::PutConfig {
                toml,
                base_revision,
            } => self.dispatch_put(role, toml, base_revision),
            RequestKind::ConfirmSensitiveChange { .. }
            | RequestKind::SetPin { .. }
            | RequestKind::ClearPin => self.dispatch_sensitive(role),
            RequestKind::Subscribe => {
                if role == Role::App {
                    ResponseKind::Accepted
                } else {
                    ResponseKind::Unauthorized
                }
            }
            RequestKind::CheckUpdate => {
                if role != Role::App {
                    ResponseKind::Unauthorized
                } else if self.config_writable() {
                    ResponseKind::UpToDate
                } else {
                    ResponseKind::Locked
                }
            }
            RequestKind::InstallUpdate { .. } => {
                if role != Role::App {
                    ResponseKind::Unauthorized
                } else if self.config_writable() {
                    ResponseKind::Installing
                } else {
                    ResponseKind::Locked
                }
            }
        }
    }

    fn dispatch_lock(&mut self, role: Role, source: LockSource) -> ResponseKind {
        if !matches!(role, Role::App | Role::LockClient) {
            return ResponseKind::Unauthorized;
        }
        if !matches!(self.state, LockState::Unlocked) {
            return ResponseKind::AlreadyLocked;
        }
        self.start_lock(source);
        ResponseKind::Accepted
    }

    fn dispatch_hard_lock(&mut self, role: Role) -> ResponseKind {
        if role == Role::App && matches!(self.state, LockState::Unlocked) {
            self.platform.record_os_lock();
            return ResponseKind::Accepted;
        }
        if role == Role::LockHost && matches!(self.state, LockState::Locked { .. }) {
            self.fail_closed(FailReason::ServiceStop);
            return ResponseKind::Accepted;
        }
        ResponseKind::Unauthorized
    }

    fn dispatch_put(&mut self, role: Role, toml: String, base_revision: u64) -> ResponseKind {
        if role != Role::App {
            return ResponseKind::Unauthorized;
        }
        if !self.config_writable() {
            return ResponseKind::Locked;
        }
        if toml.trim().is_empty() {
            return invalid_config();
        }
        if base_revision != self.config_revision {
            return ResponseKind::Conflict;
        }
        self.config_toml = toml;
        self.config_revision = self.config_revision.saturating_add(1);
        ResponseKind::Applied
    }

    fn dispatch_sensitive(&mut self, role: Role) -> ResponseKind {
        if role != Role::App {
            ResponseKind::Unauthorized
        } else if self.config_writable() {
            ResponseKind::Ok
        } else {
            ResponseKind::Locked
        }
    }

    fn public_reader(&self, role: Role) -> bool {
        matches!(role, Role::App | Role::LockClient)
    }

    fn config_writable(&self) -> bool {
        matches!(self.state, LockState::Unlocked) && !self.session_os_locked
    }
}

fn invalid_config() -> ResponseKind {
    ResponseKind::Invalid {
        errors: vec!["empty config".to_string()],
    }
}
