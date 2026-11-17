//! Whether a lock path counts as a success.
//!
//! A call that returns, or a screen-saver process that starts, is not enough.
//! The session has to report locked. A call that is still running when the
//! wait ends is `still-running`, and that is a failure for the chooser.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallWait {
    Returned(u64),
    Failed(String),
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attempt {
    Locked {
        returned_ms: Option<u64>,
        until_locked_ms: u64,
    },
    Failed {
        detail: String,
        returned_ms: Option<u64>,
    },
    /// The call returned, and the session-lock check could not be read.
    /// This is not a failed call. The exercise still tries the next path.
    Unobserved {
        returned_ms: Option<u64>,
    },
    StillRunning,
}

/// What the session-lock check reported after a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockSeen {
    Yes(u64),
    No,
    Unavailable,
}

pub fn judge(call: CallWait, seen: LockSeen) -> Attempt {
    match call {
        CallWait::TimedOut => Attempt::StillRunning,
        CallWait::Failed(detail) => Attempt::Failed {
            detail,
            returned_ms: None,
        },
        CallWait::Returned(returned_ms) => match seen {
            LockSeen::Yes(until_locked_ms) => Attempt::Locked {
                returned_ms: Some(returned_ms),
                until_locked_ms,
            },
            LockSeen::No => Attempt::Failed {
                detail: "the call returned and the session was not locked".to_string(),
                returned_ms: Some(returned_ms),
            },
            LockSeen::Unavailable => Attempt::Unobserved {
                returned_ms: Some(returned_ms),
            },
        },
    }
}

/// How a signalled parent records the helper. `Unread` is not `never-unlocked`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalEnd {
    AlreadyLocked,
    HelperLocked,
    HelperNeverUnlocked,
    HelperOther,
    Unread,
}

pub fn signal_end(already_locked: bool, helper_end: Option<&str>) -> SignalEnd {
    if already_locked {
        return SignalEnd::AlreadyLocked;
    }
    match helper_end {
        Some("lock-private" | "lock-documented") => SignalEnd::HelperLocked,
        Some("never-unlocked") => SignalEnd::HelperNeverUnlocked,
        Some(_) => SignalEnd::HelperOther,
        None => SignalEnd::Unread,
    }
}

/// A signal during an exercise never unlocks. If the session is not locked
/// yet, the parent runs the fail-closed fallback and then exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterruptPlan {
    FallbackThenExit,
    ExitWithoutUnlock,
}

pub fn interrupt_plan(session_locked: bool) -> InterruptPlan {
    if session_locked {
        InterruptPlan::ExitWithoutUnlock
    } else {
        InterruptPlan::FallbackThenExit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_return_without_a_locked_session_is_a_failed_call() {
        let attempt = judge(CallWait::Returned(12), LockSeen::No);
        assert!(matches!(
            attempt,
            Attempt::Failed {
                returned_ms: Some(12),
                ..
            }
        ));
    }

    #[test]
    fn an_unavailable_check_is_not_a_failed_call() {
        let attempt = judge(CallWait::Returned(12), LockSeen::Unavailable);
        assert_eq!(
            attempt,
            Attempt::Unobserved {
                returned_ms: Some(12)
            }
        );
    }

    #[test]
    fn a_locked_session_records_both_times() {
        let attempt = judge(CallWait::Returned(12), LockSeen::Yes(40));
        assert_eq!(
            attempt,
            Attempt::Locked {
                returned_ms: Some(12),
                until_locked_ms: 40,
            }
        );
    }

    #[test]
    fn a_hung_call_is_still_running() {
        assert_eq!(
            judge(CallWait::TimedOut, LockSeen::No),
            Attempt::StillRunning
        );
    }

    #[test]
    fn an_unread_helper_is_not_never_unlocked() {
        assert_eq!(signal_end(false, None), SignalEnd::Unread);
        assert_eq!(
            signal_end(false, Some("never-unlocked")),
            SignalEnd::HelperNeverUnlocked
        );
        assert_eq!(
            signal_end(false, Some("lock-private")),
            SignalEnd::HelperLocked
        );
        assert_eq!(
            signal_end(true, Some("never-unlocked")),
            SignalEnd::AlreadyLocked
        );
    }

    #[test]
    fn an_interrupt_falls_back_until_the_session_is_locked() {
        assert_eq!(interrupt_plan(false), InterruptPlan::FallbackThenExit);
        assert_eq!(interrupt_plan(true), InterruptPlan::ExitWithoutUnlock);
    }
}
