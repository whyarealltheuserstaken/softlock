//! Which OS-lock call the proof accepts. No operating-system calls.

#![cfg_attr(not(windows), allow(dead_code))]

pub const ONE_SECOND_US: u128 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick {
    pub primary: &'static str,
    pub fallback: &'static str,
    pub lock_workstation: bool,
    pub service_disconnect: bool,
    pub own_session: bool,
}

pub fn under_one_second(elapsed_us: u128) -> bool {
    elapsed_us < ONE_SECOND_US
}

/// The policy blocks the lock when no lock notification arrived and the
/// session is not reported locked. `LockWorkStation` can still return success
/// in that case, so the API result is not part of the proof. An unreadable
/// flag does not count as locked.
pub fn blocked_proven(_api_ok: bool, saw_lock: bool, session_locked: Option<bool>) -> bool {
    !saw_lock && session_locked != Some(true)
}

/// A notification inside the deadline. The API's success flag is not evidence.
#[cfg_attr(not(test), allow(dead_code))]
pub fn confirmed(notified: bool, elapsed_us: u128) -> bool {
    notified && under_one_second(elapsed_us)
}

/// The lock is real when a lock notification arrived, or the session state
/// says locked, inside one second. A successful return by itself is not a lock.
pub fn confirmed_lock(
    kind: &str,
    notified: bool,
    session_locked: Option<bool>,
    elapsed_us: u128,
) -> bool {
    under_one_second(elapsed_us) && ((notified && kind == "lock") || session_locked == Some(true))
}

pub fn confirmed_disconnect(
    kind: &str,
    notified: bool,
    disconnected: bool,
    elapsed_us: u128,
) -> bool {
    under_one_second(elapsed_us) && ((notified && kind == "console-disconnect") || disconnected)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    Locked,
    Failed,
}

/// Try the lock. Only if that is not confirmed, try the disconnect. If neither
/// is confirmed the outcome is [`CloseOutcome::Failed`] and the caller must
/// not report the machine as locked.
pub fn run_close_chain(
    try_lock: impl FnOnce() -> bool,
    try_disconnect: impl FnOnce() -> bool,
) -> CloseOutcome {
    if try_lock() {
        return CloseOutcome::Locked;
    }
    if try_disconnect() {
        return CloseOutcome::Locked;
    }
    CloseOutcome::Failed
}

/// `LockWorkStation` is primary when it confirms. `WTSDisconnectSession` is
/// the fall-back when either disconnect caller confirms. Ticket 15 implements
/// that pair. Ticket 17 clears `DisableLockWorkstation` before the primary.
pub fn pick(lock: bool, service: bool, own: bool) -> Pick {
    let primary = if lock {
        "LockWorkStation"
    } else if service || own {
        "WTSDisconnectSession"
    } else {
        "none"
    };
    let fallback = if service || own {
        "WTSDisconnectSession"
    } else if lock {
        "unproven"
    } else {
        "none"
    };
    Pick {
        primary,
        fallback,
        lock_workstation: lock,
        service_disconnect: service,
        own_session: own,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn judge(
    mode: &str,
    api_ok: bool,
    notified: bool,
    elapsed_us: u128,
    session_locked: Option<bool>,
) -> bool {
    match mode {
        "blocked" => blocked_proven(api_ok, notified, session_locked),
        "confirm" => confirmed(notified, elapsed_us),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_second_is_not_under_one_second() {
        assert!(under_one_second(999_999));
        assert!(!under_one_second(1_000_000));
    }

    #[test]
    fn the_policy_blocks_the_lock_when_no_notification_arrives_and_the_session_stays_up() {
        assert!(blocked_proven(false, false, Some(false)));
        assert!(blocked_proven(false, false, None));
        assert!(blocked_proven(true, false, Some(false)));
        assert!(!blocked_proven(true, false, Some(true)));
        assert!(!blocked_proven(false, true, Some(false)));
        assert!(!blocked_proven(false, false, Some(true)));
        assert!(!blocked_proven(true, true, Some(false)));
    }

    #[test]
    fn a_confirmed_path_needs_the_notification_and_the_deadline() {
        assert!(confirmed(true, 999_999));
        assert!(!confirmed(true, 1_000_000));
        assert!(!confirmed(false, 10));
    }

    #[test]
    fn the_api_return_does_not_confirm_a_lock() {
        assert!(confirmed_lock("lock", true, Some(false), 40));
        assert!(confirmed_lock("none", false, Some(true), 40));
        assert!(!confirmed_lock("none", false, Some(false), 40));
        assert!(!confirmed_lock("lock", true, Some(true), 1_000_000));
        assert!(!confirmed_lock("console-disconnect", true, Some(false), 40));
    }

    #[test]
    fn a_disconnect_is_confirmed_by_the_notification_or_the_session_state() {
        assert!(confirmed_disconnect("console-disconnect", true, false, 80));
        assert!(confirmed_disconnect("none", false, true, 80));
        assert!(!confirmed_disconnect("none", false, false, 80));
        assert!(!confirmed_disconnect("lock", true, false, 80));
        assert!(!confirmed_disconnect(
            "console-disconnect",
            true,
            true,
            1_000_000
        ));
    }

    #[test]
    fn a_confirmed_lock_does_not_call_the_disconnect() {
        let mut disconnects = 0;
        let outcome = run_close_chain(
            || true,
            || {
                disconnects += 1;
                true
            },
        );
        assert_eq!(outcome, CloseOutcome::Locked);
        assert_eq!(disconnects, 0);
    }

    #[test]
    fn an_unconfirmed_lock_uses_a_confirmed_disconnect() {
        let outcome = run_close_chain(|| false, || true);
        assert_eq!(outcome, CloseOutcome::Locked);
    }

    #[test]
    fn both_unconfirmed_steps_fail_closed() {
        let mut locks = 0;
        let mut disconnects = 0;
        let outcome = run_close_chain(
            || {
                locks += 1;
                false
            },
            || {
                disconnects += 1;
                false
            },
        );
        assert_eq!(outcome, CloseOutcome::Failed);
        assert_eq!(locks, 1);
        assert_eq!(disconnects, 1);
    }

    #[test]
    fn all_three_paths_pick_the_workstation_lock_and_the_disconnect_fall_back() {
        let pick = pick(true, true, true);
        assert_eq!(pick.primary, "LockWorkStation");
        assert_eq!(pick.fallback, "WTSDisconnectSession");
        assert!(pick.lock_workstation);
        assert!(pick.service_disconnect);
        assert!(pick.own_session);
    }

    #[test]
    fn a_failed_workstation_lock_makes_disconnect_the_primary() {
        let pick = pick(false, true, true);
        assert_eq!(pick.primary, "WTSDisconnectSession");
        assert_eq!(pick.fallback, "WTSDisconnectSession");
        assert!(!pick.lock_workstation);
    }

    #[test]
    fn one_disconnect_caller_is_enough_for_the_fall_back() {
        let from_service = pick(true, true, false);
        assert_eq!(from_service.primary, "LockWorkStation");
        assert_eq!(from_service.fallback, "WTSDisconnectSession");
        assert!(from_service.service_disconnect);
        assert!(!from_service.own_session);

        let from_self = pick(true, false, true);
        assert_eq!(from_self.fallback, "WTSDisconnectSession");
        assert!(from_self.own_session);
        assert!(!from_self.service_disconnect);
    }

    #[test]
    fn a_lock_without_a_disconnect_leaves_the_fall_back_unproven() {
        let pick = pick(true, false, false);
        assert_eq!(pick.primary, "LockWorkStation");
        assert_eq!(pick.fallback, "unproven");
    }

    #[test]
    fn nothing_confirmed_picks_neither() {
        let pick = pick(false, false, false);
        assert_eq!(pick.primary, "none");
        assert_eq!(pick.fallback, "none");
    }

    #[test]
    fn judge_uses_the_blocked_and_confirm_rules() {
        assert!(judge("blocked", false, false, 1_000_000, Some(false)));
        assert!(judge("blocked", true, false, 1_000_000, Some(false)));
        assert!(!judge("blocked", true, true, 10, Some(false)));
        assert!(judge("confirm", false, true, 40, Some(false)));
        assert!(!judge("confirm", true, false, 40, Some(false)));
        assert!(!judge("confirm", true, true, 1_000_000, Some(true)));
        assert!(!judge("other", true, true, 10, Some(true)));
    }
}
