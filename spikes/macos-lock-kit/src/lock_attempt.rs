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
    StillRunning,
}

pub fn judge(call: CallWait, locked_after_ms: Option<u64>) -> Attempt {
    match call {
        CallWait::TimedOut => Attempt::StillRunning,
        CallWait::Failed(detail) => Attempt::Failed {
            detail,
            returned_ms: None,
        },
        CallWait::Returned(returned_ms) => match locked_after_ms {
            Some(until_locked_ms) => Attempt::Locked {
                returned_ms: Some(returned_ms),
                until_locked_ms,
            },
            None => Attempt::Failed {
                detail: "the call returned and the session was not locked".to_string(),
                returned_ms: Some(returned_ms),
            },
        },
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
        let attempt = judge(CallWait::Returned(12), None);
        assert!(matches!(
            attempt,
            Attempt::Failed {
                returned_ms: Some(12),
                ..
            }
        ));
    }

    #[test]
    fn a_locked_session_records_both_times() {
        let attempt = judge(CallWait::Returned(12), Some(40));
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
        assert_eq!(judge(CallWait::TimedOut, None), Attempt::StillRunning);
    }

    #[test]
    fn an_interrupt_falls_back_until_the_session_is_locked() {
        assert_eq!(interrupt_plan(false), InterruptPlan::FallbackThenExit);
        assert_eq!(interrupt_plan(true), InterruptPlan::ExitWithoutUnlock);
    }
}
