//! Run length uses a sleep-aware monotonic clock.
//!
//! `Instant` and `QueryUnbiasedInterruptTime` stop during sleep, so a laptop
//! that sleeps would keep the run going after wake. `QueryInterruptTime` keeps
//! counting through sleep and is not moved by a wall-clock change. Log lines
//! still use UTC wall-clock time, so a clock change shows up in the timestamps
//! without shortening or stretching the run.

/// A run has reached its planned length when this much monotonic time has passed.
pub fn deadline_reached(start_ms: u64, now_ms: u64, duration_ms: u64) -> bool {
    now_ms.saturating_sub(start_ms) >= duration_ms
}

pub fn sleep_aware_now_ms() -> u64 {
    #[cfg(windows)]
    {
        // SAFETY: QueryInterruptTime writes one u64. The count includes time
        // the system spends asleep and ignores wall-clock changes.
        unsafe { windows::Win32::System::WindowsProgramming::QueryInterruptTime() / 10_000 }
    }
    #[cfg(not(windows))]
    {
        crate::timefmt::unix_now_ms()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deadline_uses_elapsed_monotonic_time() {
        assert!(!deadline_reached(1_000, 1_000, 60_000));
        assert!(!deadline_reached(1_000, 60_999, 60_000));
        assert!(deadline_reached(1_000, 61_000, 60_000));
        assert!(deadline_reached(5, 4, 0));
        assert!(deadline_reached(u64::MAX - 10, u64::MAX, 10));
    }
}
