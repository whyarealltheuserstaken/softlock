//! OS-lock decision. No operating-system calls.
//!
//! `LockWorkStation`'s return value is not evidence. A step counts only when
//! [`confirmed`] says a console-session notification or the session state
//! arrived inside [`CONFIRM_BUDGET`]. Ticket 17 replaces
//! [`OsLockOps::before_lock_workstation`]. This module does not clear
//! `DisableLockWorkstation`. Nothing here unlocks a session.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

/// A lock or disconnect that arrives at this instant or later is not evidence.
pub const CONFIRM_BUDGET: Duration = Duration::from_secs(1);

/// Who is asking. The signed-in Lock Host is the only caller of
/// `LockWorkStation`. The session-0 Service disconnects the console session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    /// `LockWorkStation`, then `WTSDisconnectSession` on this process's session.
    LockHost,
    /// `LockWorkStation` only. The Service confirms and falls back itself.
    LockHostPrimary,
    /// `WTSDisconnectSession` on the console session. No workstation call.
    Service,
    /// Ask the Lock Host to lock, then disconnect the console if that is
    /// not confirmed. A missing host skips the workstation call.
    ServiceCoordinated,
}

impl Caller {
    pub fn name(self) -> &'static str {
        match self {
            Self::LockHost => "lock-host",
            Self::LockHostPrimary => "lock-host-primary",
            Self::Service => "service",
            Self::ServiceCoordinated => "service-coordinated",
        }
    }

    fn steps(self) -> &'static [Method] {
        match self {
            Self::LockHost => &[Method::LockWorkStation, Method::DisconnectOwnSession],
            Self::LockHostPrimary => &[Method::LockWorkStation],
            Self::Service => &[Method::DisconnectConsole],
            Self::ServiceCoordinated => &[Method::LockWorkStation, Method::DisconnectConsole],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    LockWorkStation,
    DisconnectOwnSession,
    DisconnectConsole,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Self::LockWorkStation => "LockWorkStation",
            Self::DisconnectOwnSession => "WTSDisconnectSession-own",
            Self::DisconnectConsole => "WTSDisconnectSession-console",
        }
    }

    fn expect(self) -> Expect {
        match self {
            Self::LockWorkStation => Expect::Lock,
            Self::DisconnectOwnSession => Expect::DisconnectOwn,
            Self::DisconnectConsole => Expect::DisconnectConsole,
        }
    }
}

/// What the one-second wait is looking for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    Lock,
    DisconnectOwn,
    DisconnectConsole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Lock,
    ConsoleDisconnect,
    Other,
}

/// A WTS notification is evidence only for the console session.
pub fn console_notice(kind: NoticeKind, session: u32, console: u32) -> Option<NoticeKind> {
    if session != console {
        return None;
    }
    match kind {
        NoticeKind::Lock | NoticeKind::ConsoleDisconnect => Some(kind),
        NoticeKind::Other => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub notice: Option<NoticeKind>,
    pub session_locked: Option<bool>,
    pub session_disconnected: Option<bool>,
    pub elapsed: Duration,
}

pub fn within_budget(elapsed: Duration) -> bool {
    elapsed < CONFIRM_BUDGET
}

/// The API return is not a parameter. An unreadable session flag is not a lock.
pub fn confirmed(expect: Expect, observation: &Observation) -> bool {
    if !within_budget(observation.elapsed) {
        return false;
    }
    match expect {
        Expect::Lock => {
            observation.notice == Some(NoticeKind::Lock) || observation.session_locked == Some(true)
        }
        Expect::DisconnectOwn | Expect::DisconnectConsole => {
            observation.notice == Some(NoticeKind::ConsoleDisconnect)
                || observation.session_disconnected == Some(true)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepFault {
    Failed,
    /// The call was not made. The chain must not wait out the budget.
    Skipped,
    Panic,
}

impl StepFault {
    fn name(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Panic => "panic",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub method: Method,
    pub confirmed: bool,
    pub fault: Option<StepFault>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Confirmed {
        method: Method,
    },
    /// Distinct from unlocked. The overlay and the input block stay up.
    FailedClosed {
        attempts: Vec<Attempt>,
    },
}

pub fn stays_guarded(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::FailedClosed { .. })
}

/// Ticket 17 replaces the workstation preparation. This ticket leaves the
/// value alone, including for the disconnect fall-back.
pub fn policy_seam() -> &'static str {
    "leave-disable-lock-workstation"
}

pub fn failed_closed_log(caller: Caller, attempts: &[Attempt]) -> String {
    let mut line = format!("failed-closed caller={}", caller.name());
    for attempt in attempts {
        line.push(' ');
        line.push_str(attempt.method.name());
        line.push('=');
        if attempt.confirmed {
            line.push_str("confirmed");
        } else {
            line.push_str("unconfirmed");
        }
        if let Some(fault) = attempt.fault {
            line.push(':');
            line.push_str(fault.name());
        }
    }
    line
}

/// Operations the chain calls. A `Skipped` workstation call is how the
/// Service records that the Lock Host is not there. Any other error, including
/// a panic, still falls through to the next method.
pub trait OsLockOps {
    /// Called only before `LockWorkStation`. The Windows caller does not
    /// clear `DisableLockWorkstation` here.
    fn before_lock_workstation(&mut self) -> Result<(), StepFault> {
        Ok(())
    }

    fn call(&mut self, method: Method) -> Result<(), StepFault>;

    fn observe(&mut self, expect: Expect, started: Instant) -> Result<Observation, StepFault>;
}

pub fn fail_closed(caller: Caller, ops: &mut impl OsLockOps) -> Outcome {
    let mut attempts = Vec::new();
    for method in caller.steps() {
        if *method == Method::LockWorkStation {
            let _ = catch_step(StepFault::Panic, || ops.before_lock_workstation());
        }
        let started = Instant::now();
        let call = catch_step(StepFault::Panic, || ops.call(*method));
        if matches!(call, Err(StepFault::Skipped)) {
            attempts.push(Attempt {
                method: *method,
                confirmed: false,
                fault: Some(StepFault::Skipped),
            });
            continue;
        }
        let observed = catch_step(StepFault::Panic, || ops.observe(method.expect(), started));
        let confirmed = match &observed {
            Ok(observation) => confirmed(method.expect(), observation),
            Err(_) => false,
        };
        let fault = if confirmed {
            None
        } else {
            match (&call, &observed) {
                (_, Err(fault)) => Some(*fault),
                (Err(fault), _) => Some(*fault),
                _ => None,
            }
        };
        attempts.push(Attempt {
            method: *method,
            confirmed,
            fault,
        });
        if confirmed {
            return Outcome::Confirmed { method: *method };
        }
    }
    Outcome::FailedClosed { attempts }
}

fn catch_step<T>(
    panic_fault: StepFault,
    step: impl FnOnce() -> Result<T, StepFault>,
) -> Result<T, StepFault> {
    match catch_unwind(AssertUnwindSafe(step)) {
        Ok(result) => result,
        Err(_) => Err(panic_fault),
    }
}
