//! Timing rule from the fail-closed proof. No operating-system calls.

pub const ONE_SECOND_US: u128 = 1_000_000;

pub fn within_one_second(elapsed_us: u128) -> bool {
    elapsed_us < ONE_SECOND_US
}

/// `LockWorkStation` is blocked when no lock notification arrived and the
/// session is not reported locked. `None` means the flag could not be read.
pub fn lock_blocked(saw_lock: bool, session_locked: Option<bool>) -> bool {
    !saw_lock && session_locked != Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_second_is_not_under_one_second() {
        assert!(within_one_second(999_999));
        assert!(!within_one_second(1_000_000));
    }

    #[test]
    fn a_policy_blocks_the_lock_only_when_the_session_stays_unlocked() {
        assert!(lock_blocked(false, Some(false)));
        assert!(lock_blocked(false, None));
        assert!(!lock_blocked(true, Some(false)));
        assert!(!lock_blocked(false, Some(true)));
    }
}
