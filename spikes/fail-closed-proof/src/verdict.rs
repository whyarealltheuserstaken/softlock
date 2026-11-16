//! Timing and pass/fail rules for the three OS-lock paths. No operating-system calls.

pub const ONE_SECOND_US: u128 = 1_000_000;
pub const HEARTBEAT_GAP_LIMIT_MS: u128 = 2_000;

pub const WTS_CONSOLE_CONNECT: u32 = 0x1;
pub const WTS_CONSOLE_DISCONNECT: u32 = 0x2;
pub const WTS_REMOTE_CONNECT: u32 = 0x3;
pub const WTS_REMOTE_DISCONNECT: u32 = 0x4;
pub const WTS_SESSION_LOGON: u32 = 0x5;
pub const WTS_SESSION_LOGOFF: u32 = 0x6;
pub const WTS_SESSION_LOCK: u32 = 0x7;
pub const WTS_SESSION_UNLOCK: u32 = 0x8;

pub fn notification_name(kind: u32) -> &'static str {
    match kind {
        WTS_CONSOLE_CONNECT => "WTS_CONSOLE_CONNECT",
        WTS_CONSOLE_DISCONNECT => "WTS_CONSOLE_DISCONNECT",
        WTS_REMOTE_CONNECT => "WTS_REMOTE_CONNECT",
        WTS_REMOTE_DISCONNECT => "WTS_REMOTE_DISCONNECT",
        WTS_SESSION_LOGON => "WTS_SESSION_LOGON",
        WTS_SESSION_LOGOFF => "WTS_SESSION_LOGOFF",
        WTS_SESSION_LOCK => "WTS_SESSION_LOCK",
        WTS_SESSION_UNLOCK => "WTS_SESSION_UNLOCK",
        _ => "WTS_OTHER",
    }
}

pub fn elapsed_us(t0: u64, t1: u64, freq: u64) -> Option<u128> {
    if freq == 0 || t1 < t0 {
        return None;
    }
    Some(u128::from(t1 - t0).saturating_mul(1_000_000) / u128::from(freq))
}

pub fn within_one_second(elapsed_us: u128) -> bool {
    elapsed_us < ONE_SECOND_US
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedPath {
    pub api_ok: bool,
    pub notification: Option<u32>,
    pub elapsed_us: Option<u128>,
    pub heartbeat_gap_ms: Option<u128>,
}

pub fn disconnect_path_ok(path: &ObservedPath) -> bool {
    path.api_ok
        && path.notification == Some(WTS_CONSOLE_DISCONNECT)
        && path.elapsed_us.is_some_and(within_one_second)
        && path
            .heartbeat_gap_ms
            .is_some_and(|gap| gap < HEARTBEAT_GAP_LIMIT_MS)
}

pub fn lock_path_ok(path: &ObservedPath) -> bool {
    path.api_ok
        && path.notification == Some(WTS_SESSION_LOCK)
        && path.elapsed_us.is_some_and(within_one_second)
        && path
            .heartbeat_gap_ms
            .is_some_and(|gap| gap < HEARTBEAT_GAP_LIMIT_MS)
}

pub fn lock_blocked(api_ok: bool, saw_lock: bool) -> bool {
    !api_ok && !saw_lock
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub runner_image: String,
    pub session_id: u32,
    pub service_session_id: Option<u32>,
    pub policies: Vec<String>,
    pub lock_blocked: bool,
    pub lock_blocked_win32: Option<u32>,
    pub lock_saw_notification: bool,
    pub service_disconnect: Option<ObservedPath>,
    pub session_disconnect: Option<ObservedPath>,
    pub lock_cleared: bool,
    pub user_lock: Option<ObservedPath>,
    pub policies_restored: bool,
    pub error: Option<String>,
}

impl Outcome {
    pub fn failed(runner_image: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            runner_image: runner_image.into(),
            session_id: u32::MAX,
            service_session_id: None,
            policies: Vec::new(),
            lock_blocked: false,
            lock_blocked_win32: None,
            lock_saw_notification: false,
            service_disconnect: None,
            session_disconnect: None,
            lock_cleared: false,
            user_lock: None,
            policies_restored: false,
            error: Some(error.into()),
        }
    }
}

pub fn passed(outcome: &Outcome) -> bool {
    outcome.error.is_none()
        && outcome.lock_blocked
        && !outcome.lock_saw_notification
        && outcome.lock_cleared
        && outcome.policies_restored
        && outcome.service_session_id == Some(0)
        && outcome.policies.len() == crate::journal::LOCKDOWN.len()
        && outcome
            .service_disconnect
            .as_ref()
            .is_some_and(disconnect_path_ok)
        && outcome
            .session_disconnect
            .as_ref()
            .is_some_and(disconnect_path_ok)
        && outcome.user_lock.as_ref().is_some_and(lock_path_ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast(notification: u32) -> ObservedPath {
        ObservedPath {
            api_ok: true,
            notification: Some(notification),
            elapsed_us: Some(12_000),
            heartbeat_gap_ms: Some(50),
        }
    }

    fn passing() -> Outcome {
        Outcome {
            runner_image: "windows-2022".to_string(),
            session_id: 2,
            service_session_id: Some(0),
            policies: crate::journal::LOCKDOWN
                .iter()
                .map(|policy| policy.name.to_string())
                .collect(),
            lock_blocked: true,
            lock_blocked_win32: Some(5),
            lock_saw_notification: false,
            service_disconnect: Some(fast(WTS_CONSOLE_DISCONNECT)),
            session_disconnect: Some(fast(WTS_CONSOLE_DISCONNECT)),
            lock_cleared: true,
            user_lock: Some(fast(WTS_SESSION_LOCK)),
            policies_restored: true,
            error: None,
        }
    }

    #[test]
    fn a_full_second_is_not_under_one_second() {
        assert!(within_one_second(999_999));
        assert!(!within_one_second(1_000_000));
        assert_eq!(elapsed_us(0, 10_000, 10_000_000), Some(1_000));
        assert_eq!(elapsed_us(5, 4, 10), None);
    }

    #[test]
    fn lock_while_the_policy_is_set_must_fail_without_a_notification() {
        assert!(lock_blocked(false, false));
        assert!(!lock_blocked(true, false));
        assert!(!lock_blocked(false, true));
    }

    #[test]
    fn all_three_paths_must_be_confirmed_in_under_one_second() {
        assert!(passed(&passing()));
        let mut slow = passing();
        slow.user_lock.as_mut().expect("lock").elapsed_us = Some(1_000_000);
        assert!(!passed(&slow));
        let mut remote = passing();
        remote
            .service_disconnect
            .as_mut()
            .expect("disconnect")
            .notification = Some(WTS_REMOTE_DISCONNECT);
        assert!(!passed(&remote));
        let mut dead_app = passing();
        dead_app
            .session_disconnect
            .as_mut()
            .expect("session")
            .heartbeat_gap_ms = None;
        assert!(!passed(&dead_app));
    }

    #[test]
    fn a_service_outside_session_zero_does_not_pass() {
        let mut outcome = passing();
        outcome.service_session_id = Some(2);
        assert!(!passed(&outcome));
    }
}
