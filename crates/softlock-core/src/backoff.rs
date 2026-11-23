//! Unlock backoff. Attempts 1–4 have no delay. The delay starts after the
//! 4th failure, so the 5th attempt waits 30 seconds, and each later failure
//! doubles that wait up to 15 minutes.

use std::time::Duration;

pub fn delay_after_failure(failures: u32) -> Duration {
    if failures < 4 {
        return Duration::ZERO;
    }
    let shift = failures - 4;
    let cap_seconds = 15 * 60;
    let seconds = if shift >= 5 {
        cap_seconds
    } else {
        (30u64 << shift).min(cap_seconds)
    };
    Duration::from_secs(seconds)
}

/// Saved with the settings journal. `written_tick` and `until_tick` are a
/// monotonic millisecond clock (`GetTickCount64` on Windows). `written_wall_ms`
/// is Unix time, used only to notice a clock rollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackoffRecord {
    pub failures: u32,
    pub written_tick: u64,
    pub until_tick: u64,
    pub written_wall_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockSample {
    pub tick: u64,
    pub wall_ms: u64,
}

/// How long a restarted service keeps refusing attempts.
///
/// `Wait` is fail-closed: a rollback, a reboot, or a record that cannot prove
/// its deadline restarts the full delay for the saved failure count. A wall
/// clock that jumps forward does not shorten it. The monotonic tick is what
/// expires a delay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackoffRestore {
    Clear,
    Wait(Duration),
}

pub fn restore_backoff(record: BackoffRecord, sample: ClockSample) -> BackoffRestore {
    if record.failures == 0 {
        return BackoffRestore::Clear;
    }
    let full = delay_after_failure(record.failures);
    if full.is_zero() {
        return BackoffRestore::Clear;
    }
    let gap = record.until_tick.saturating_sub(record.written_tick);
    let cap = duration_ms(full).saturating_add(5_000);
    let untrusted = record.written_tick == 0
        || record.written_wall_ms == 0
        || record.until_tick < record.written_tick
        || gap > cap
        || sample.tick == 0
        || sample.wall_ms == 0
        || sample.wall_ms < record.written_wall_ms
        || sample.tick < record.written_tick;
    if untrusted {
        return BackoffRestore::Wait(full);
    }
    if sample.tick < record.until_tick {
        return BackoffRestore::Wait(Duration::from_millis(record.until_tick - sample.tick));
    }
    BackoffRestore::Clear
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(failures: u32, written: u64, until: u64, wall: u64) -> BackoffRecord {
        BackoffRecord {
            failures,
            written_tick: written,
            until_tick: until,
            written_wall_ms: wall,
        }
    }

    #[test]
    fn the_fifth_attempt_waits_thirty_seconds_and_the_delay_caps_at_fifteen_minutes() {
        assert_eq!(delay_after_failure(3), Duration::ZERO);
        assert_eq!(delay_after_failure(4), Duration::from_secs(30));
        assert_eq!(delay_after_failure(5), Duration::from_secs(60));
        assert_eq!(delay_after_failure(9), Duration::from_secs(15 * 60));
        assert_eq!(delay_after_failure(40), Duration::from_secs(15 * 60));
    }

    #[test]
    fn a_restart_keeps_the_remaining_delay() {
        let saved = record(4, 1_000, 31_000, 50_000);
        let left = restore_backoff(
            saved,
            ClockSample {
                tick: 11_000,
                wall_ms: 60_000,
            },
        );
        assert_eq!(left, BackoffRestore::Wait(Duration::from_secs(20)));
    }

    #[test]
    fn a_forward_wall_clock_does_not_expire_the_delay() {
        let saved = record(4, 1_000, 31_000, 50_000);
        let left = restore_backoff(
            saved,
            ClockSample {
                tick: 2_000,
                wall_ms: 50_000 + 3_600_000,
            },
        );
        assert_eq!(left, BackoffRestore::Wait(Duration::from_secs(29)));
    }

    #[test]
    fn a_wall_clock_rollback_restarts_the_full_delay() {
        let saved = record(4, 1_000, 31_000, 50_000);
        let left = restore_backoff(
            saved,
            ClockSample {
                tick: 40_000,
                wall_ms: 49_000,
            },
        );
        assert_eq!(left, BackoffRestore::Wait(Duration::from_secs(30)));
    }

    #[test]
    fn a_tick_rollback_or_a_corrupt_deadline_restarts_the_full_delay() {
        let saved = record(4, 10_000, 40_000, 50_000);
        assert_eq!(
            restore_backoff(
                saved,
                ClockSample {
                    tick: 9_000,
                    wall_ms: 51_000,
                },
            ),
            BackoffRestore::Wait(Duration::from_secs(30))
        );
        let corrupt = record(5, 10_000, 9_000, 50_000);
        assert_eq!(
            restore_backoff(
                corrupt,
                ClockSample {
                    tick: 11_000,
                    wall_ms: 51_000,
                },
            ),
            BackoffRestore::Wait(Duration::from_secs(60))
        );
        let huge = record(4, 1_000, 1_000 + 3_600_000, 50_000);
        assert_eq!(
            restore_backoff(
                huge,
                ClockSample {
                    tick: 2_000,
                    wall_ms: 51_000,
                },
            ),
            BackoffRestore::Wait(Duration::from_secs(30))
        );
    }

    #[test]
    fn an_expired_monotonic_deadline_with_a_steady_clock_clears() {
        let saved = record(4, 1_000, 31_000, 50_000);
        assert_eq!(
            restore_backoff(
                saved,
                ClockSample {
                    tick: 31_000,
                    wall_ms: 80_000,
                },
            ),
            BackoffRestore::Clear
        );
    }

    #[test]
    fn fewer_than_four_failures_have_no_wait_to_restore() {
        let saved = record(3, 1_000, 1_000, 50_000);
        assert_eq!(
            restore_backoff(
                saved,
                ClockSample {
                    tick: 2_000,
                    wall_ms: 10,
                },
            ),
            BackoffRestore::Clear
        );
    }
}
