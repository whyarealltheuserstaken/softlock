//! A failed lock path must not unlock. It may OS-lock.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailClosed {
    /// Session was never claimed locked.
    Idle,
    /// Hold the OS lock (loginctl lock-session). Never unlock.
    OsLock,
}

pub fn on_failure(armed: bool) -> FailClosed {
    if armed {
        FailClosed::OsLock
    } else {
        FailClosed::Idle
    }
}

pub fn state_label(action: FailClosed, interrupted: bool) -> &'static str {
    match (action, interrupted) {
        (FailClosed::OsLock, _) => "os-locked",
        (FailClosed::Idle, true) => "never-unlocked",
        (FailClosed::Idle, false) => "never-unlocked",
    }
}

/// The suite must never issue this logind verb.
pub const FORBIDDEN_UNLOCK: &str = "unlock-session";

#[cfg(test)]
mod tests {
    use super::{FailClosed, on_failure, state_label};

    #[test]
    fn armed_failure_locks() {
        assert_eq!(on_failure(true), FailClosed::OsLock);
        assert_eq!(state_label(FailClosed::OsLock, false), "os-locked");
    }

    #[test]
    fn idle_stays_never_unlocked() {
        assert_eq!(on_failure(false), FailClosed::Idle);
        assert_eq!(state_label(FailClosed::Idle, true), "never-unlocked");
    }
}
