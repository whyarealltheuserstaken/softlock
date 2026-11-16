//! Which real Windows lock was reached.
//!
//! This dev launch does not clear `DisableLockWorkstation`. Ticket 15 does.
//! `LockWorkStation` is tried first and accepted only when the session flags
//! show a lock. Otherwise `WTSDisconnectSession` is the fallback. If both
//! fail, the result is `Unavailable`. The guard and the overlay stay up, the
//! PIN stays usable, and the OS lock is retried. An `Arming` or `Locked`
//! failure does not end `Unlocked`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum RealLock {
    /// `LockWorkStation` and the session flags agree the session is locked.
    Workstation,
    /// The session was disconnected. The sign-in screen is up.
    Disconnected,
    /// Neither call reached the real lock screen.
    Unavailable,
}

/// `workstation_locked` wins. A later disconnect is not required, and must
/// not be treated as the result, once the workstation lock is confirmed.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn decide(workstation_locked: bool, disconnect_succeeded: bool) -> RealLock {
    if workstation_locked {
        RealLock::Workstation
    } else if disconnect_succeeded {
        RealLock::Disconnected
    } else {
        RealLock::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::{RealLock, decide};

    #[test]
    fn a_confirmed_workstation_lock_does_not_fall_back() {
        assert_eq!(decide(true, false), RealLock::Workstation);
        assert_eq!(decide(true, true), RealLock::Workstation);
    }

    #[test]
    fn disconnect_is_the_fallback_when_the_workstation_stays_unlocked() {
        assert_eq!(decide(false, true), RealLock::Disconnected);
    }

    #[test]
    fn both_paths_failing_is_unavailable() {
        assert_eq!(decide(false, false), RealLock::Unavailable);
    }
}
