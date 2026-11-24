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

/// Password failures and PIN failures are different counters.
///
/// [`delay_after_failure`] is the PIN counter. A PIN failure never calls the
/// Windows logon API, so it cannot move the account lockout count. A password
/// failure is the only one that can. The password budget is at most
/// `threshold - 2` attempts per observation window (and at least one when
/// lockout is enabled). The two counters do not share a count or a wait.
///
/// The password wait is the stricter of a monotonic clock and the wall clock.
/// Windows clears the bad-password count only after the observation window
/// has passed since the latest failure, and then it clears every failure at
/// once. A rollback of either clock keeps that latest failure inside the
/// window. Threshold `0` means Windows lockout is disabled, and the password
/// path then uses only the escalating wait. Threshold `1` or `2` allows no
/// password attempt: there is no margin under the threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PasswordStamp {
    pub tick: u64,
    pub wall_ms: u64,
}

/// Lockout policy from `NetUserModalsGet` level 3. `window` is the observation
/// window. Duration is not part of the cap: the wait tracks the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockoutPolicy {
    pub threshold: u32,
    pub window: Duration,
}

/// What the password path may do right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasswordGate {
    /// No logon call until this has elapsed on both clocks.
    pub wait: Duration,
    /// The window already holds the attempt cap.
    pub exhausted: bool,
    /// Failures that still count inside the window.
    pub active: u32,
    /// True only when a password check may call the logon API.
    pub allow: bool,
}

/// A policy read that failed. The threshold is low enough that the password
/// path allows nothing, and the window stays open so a later read is what
/// would have to reopen it.
pub fn unreadable_lockout_policy() -> LockoutPolicy {
    LockoutPolicy {
        threshold: 2,
        window: Duration::from_secs(99_999 * 60),
    }
}

/// `None` when lockout is disabled. `Some(0)` when the threshold is 1 or 2,
/// because one attempt can lock the account. Otherwise `threshold - 2`.
pub fn password_attempt_limit(threshold: u32) -> Option<u32> {
    match threshold {
        0 => None,
        1 | 2 => Some(0),
        _ => Some(threshold - 2),
    }
}

/// Wait after `count` password failures that are still inside the window.
/// The sequence is 30s, 1 min, 2 min, 5 min, 10 min, then 15 min.
pub fn password_escalation(count: u32) -> Duration {
    const STEPS: [u64; 6] = [30, 60, 120, 300, 600, 900];
    if count == 0 {
        return Duration::ZERO;
    }
    let index = (count as usize - 1).min(STEPS.len() - 1);
    Duration::from_secs(STEPS[index])
}

pub fn password_gate(
    stamps: &[PasswordStamp],
    policy: LockoutPolicy,
    now: ClockSample,
) -> PasswordGate {
    let window_ms = if policy.threshold == 0 {
        0
    } else {
        duration_ms(policy.window)
    };
    // While the latest failure is still inside the window, every recorded
    // failure counts. Once that latest failure has aged out on both clocks,
    // Windows has reset the counter, so the count is zero.
    let open = if policy.threshold == 0 {
        !stamps.is_empty()
    } else {
        latest(stamps).is_some_and(|stamp| stamp_counts(stamp, now, window_ms))
    };
    let over_bound = stamps.len() > 1_000;
    let count = if over_bound {
        1_000
    } else if open {
        u32::try_from(stamps.len()).unwrap_or(u32::MAX)
    } else {
        0
    };
    let limit = password_attempt_limit(policy.threshold);
    let exhausted = limit.is_some_and(|cap| count >= cap) || over_bound;
    let basis = if open { latest(stamps) } else { None };
    let escalation = stricter_remaining(basis, password_escalation(count), now);
    let window_left = if exhausted && open && policy.threshold > 0 {
        stricter_remaining(basis, policy.window, now)
    } else {
        Duration::ZERO
    };
    let wait = max_duration(escalation, window_left);
    PasswordGate {
        wait,
        exhausted,
        active: count,
        allow: wait.is_zero() && !exhausted,
    }
}

fn stamp_counts(stamp: PasswordStamp, now: ClockSample, window_ms: u64) -> bool {
    if window_ms == 0 || stamp.tick == 0 || stamp.wall_ms == 0 || now.tick == 0 || now.wall_ms == 0
    {
        return true;
    }
    if now.tick < stamp.tick || now.wall_ms < stamp.wall_ms {
        return true;
    }
    let tick_aged = now.tick.saturating_sub(stamp.tick) >= window_ms;
    let wall_aged = now.wall_ms.saturating_sub(stamp.wall_ms) >= window_ms;
    !(tick_aged && wall_aged)
}

fn latest(stamps: &[PasswordStamp]) -> Option<PasswordStamp> {
    stamps
        .iter()
        .copied()
        .max_by_key(|stamp| (stamp.tick, stamp.wall_ms))
}

/// True while `stamp` has not aged out of `window` on both clocks. A zero
/// window never ages out: there is no observation interval to trust.
pub fn stamp_inside_window(stamp: PasswordStamp, window: Duration, now: ClockSample) -> bool {
    stamp_counts(stamp, now, duration_ms(window))
}

fn stricter_remaining(stamp: Option<PasswordStamp>, delay: Duration, now: ClockSample) -> Duration {
    let Some(stamp) = stamp else {
        return Duration::ZERO;
    };
    let delay_ms = duration_ms(delay);
    let tick_left = clock_left(now.tick, stamp.tick, delay_ms);
    let wall_left = clock_left(now.wall_ms, stamp.wall_ms, delay_ms);
    Duration::from_millis(tick_left.max(wall_left))
}

fn clock_left(now: u64, start: u64, delay_ms: u64) -> u64 {
    if now == 0 || start == 0 || now < start {
        return delay_ms;
    }
    delay_ms.saturating_sub(now.saturating_sub(start))
}

fn max_duration(left: Duration, right: Duration) -> Duration {
    if left >= right { left } else { right }
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
    fn a_password_budget_stays_under_the_lockout_threshold() {
        assert_eq!(password_attempt_limit(10), Some(8));
        assert_eq!(password_attempt_limit(5), Some(3));
        assert_eq!(password_attempt_limit(3), Some(1));
        // Threshold 2 or 1 cannot leave a failure of margin, so the path
        // allows no password attempts at all.
        assert_eq!(password_attempt_limit(2), Some(0));
        assert_eq!(password_attempt_limit(1), Some(0));
        assert_eq!(password_attempt_limit(0), None);
        let unread = unreadable_lockout_policy();
        assert_eq!(password_attempt_limit(unread.threshold), Some(0));
        assert!(unread.window >= Duration::from_secs(99_999 * 60));
        let now = ClockSample {
            tick: 1_000,
            wall_ms: 1_000,
        };
        let blocked = password_gate(&[], unread, now);
        assert!(blocked.exhausted);
        assert!(!blocked.allow);
    }

    #[test]
    fn password_waits_escalate_and_then_cap() {
        assert_eq!(password_escalation(0), Duration::ZERO);
        assert_eq!(password_escalation(1), Duration::from_secs(30));
        assert_eq!(password_escalation(2), Duration::from_secs(60));
        assert_eq!(password_escalation(3), Duration::from_secs(120));
        assert_eq!(password_escalation(4), Duration::from_secs(300));
        assert_eq!(password_escalation(5), Duration::from_secs(600));
        assert_eq!(password_escalation(6), Duration::from_secs(900));
        assert_eq!(password_escalation(40), Duration::from_secs(900));
    }

    #[test]
    fn a_password_failure_stays_until_both_clocks_leave_the_window() {
        let policy = LockoutPolicy {
            threshold: 5,
            window: Duration::from_secs(600),
        };
        let start = ClockSample {
            tick: 1_000_000,
            wall_ms: 5_000_000,
        };
        let mut stamps = Vec::new();
        for index in 0..3 {
            let now = ClockSample {
                tick: start.tick + index * 120_000,
                wall_ms: start.wall_ms + index * 120_000,
            };
            let gate = password_gate(&stamps, policy, now);
            assert!(gate.allow, "attempt {index} was blocked");
            stamps.push(PasswordStamp {
                tick: now.tick,
                wall_ms: now.wall_ms,
            });
        }
        let full = password_gate(
            &stamps,
            policy,
            ClockSample {
                tick: start.tick + 240_000,
                wall_ms: start.wall_ms + 240_000,
            },
        );
        assert!(full.exhausted);
        assert!(!full.allow);
        assert_eq!(full.active, 3);
        assert!(full.wait >= Duration::from_secs(120));

        let jumped_tick = password_gate(
            &stamps,
            policy,
            ClockSample {
                tick: start.tick + 240_000 + 3_600_000,
                wall_ms: start.wall_ms + 241_000,
            },
        );
        assert!(
            !jumped_tick.allow,
            "a monotonic jump must not end the window while wall time still counts"
        );
        assert!(jumped_tick.exhausted);

        let rolled = password_gate(
            &stamps,
            policy,
            ClockSample {
                tick: start.tick + 900_000,
                wall_ms: start.wall_ms - 1,
            },
        );
        assert!(!rolled.allow, "a wall rollback kept the failures");

        let aged = password_gate(
            &stamps,
            policy,
            ClockSample {
                tick: start.tick + 240_000 + 600_000,
                wall_ms: start.wall_ms + 240_000 + 600_000,
            },
        );
        assert!(aged.allow, "both clocks left the window");
        assert!(!aged.exhausted);
        assert_eq!(aged.active, 0);
    }

    #[test]
    fn the_window_resets_only_after_the_latest_failure_ages_out_on_both_clocks() {
        // Windows keeps the whole bad-password count until the observation
        // window has passed since the last failure, then clears it at once.
        // Aging each stamp on its own hands the attempt back and locks the
        // account. These are the times a throwaway probe was allowed through.
        let policy = LockoutPolicy {
            threshold: 10,
            window: Duration::from_secs(600),
        };
        let times = [0u64, 30, 90, 210, 510, 630, 750, 870, 1110, 1230];
        let allowed = allowed_at(policy, &times);
        assert!(
            allowed <= 8,
            "threshold 10 allowed {allowed} attempts with no 10-minute gap after the last failure"
        );
        assert!(allowed < times.len());

        // Threshold 5, 10-minute window. The fourth and fifth attempts sit
        // where the oldest stamp would have aged out and the latest has not.
        let five = LockoutPolicy {
            threshold: 5,
            window: Duration::from_secs(600),
        };
        let tight = allowed_at(five, &[0, 30, 90, 600, 660]);
        assert!(
            tight <= 3,
            "threshold 5 with a 10-minute window allowed {tight}"
        );

        let half_hour = LockoutPolicy {
            threshold: 5,
            window: Duration::from_secs(30 * 60),
        };
        let wider = allowed_at(half_hour, &[0, 30, 90, 1800, 1860]);
        assert!(
            wider <= 3,
            "threshold 5 with a 30-minute window allowed {wider}"
        );

        let mut stamps = Vec::new();
        for secs in [0u64, 30, 90] {
            let now = at(secs);
            stamps.push(PasswordStamp {
                tick: now.tick,
                wall_ms: now.wall_ms,
            });
        }
        // Only the oldest failure has left the window. The latest has not,
        // so the count is still every failure, not two.
        let oldest_only = password_gate(&stamps, five, at(600));
        assert_eq!(oldest_only.active, 3);
        assert!(oldest_only.exhausted);
        assert!(!oldest_only.allow);

        let cleared = password_gate(&stamps, five, at(90 + 600));
        assert_eq!(cleared.active, 0);
        assert!(!cleared.exhausted);
        assert!(cleared.allow);
    }

    fn at(secs: u64) -> ClockSample {
        // Tick 0 is an untrusted stamp and never ages out. Offsets sit on a
        // normal clock.
        ClockSample {
            tick: 10_000_000 + secs * 1000,
            wall_ms: 50_000_000 + secs * 1000,
        }
    }

    fn allowed_at(policy: LockoutPolicy, times: &[u64]) -> usize {
        let mut stamps = Vec::new();
        for secs in times {
            let now = at(*secs);
            if password_gate(&stamps, policy, now).allow {
                stamps.push(PasswordStamp {
                    tick: now.tick,
                    wall_ms: now.wall_ms,
                });
            }
        }
        stamps.len()
    }

    #[test]
    fn disabled_lockout_uses_the_escalating_wait_without_a_cap() {
        let policy = LockoutPolicy {
            threshold: 0,
            window: Duration::ZERO,
        };
        let stamp = PasswordStamp {
            tick: 10_000,
            wall_ms: 20_000,
        };
        let gate = password_gate(
            &[stamp, stamp],
            policy,
            ClockSample {
                tick: 10_000,
                wall_ms: 20_000,
            },
        );
        assert!(!gate.exhausted);
        assert!(!gate.allow);
        assert_eq!(gate.wait, Duration::from_secs(60));
        let later = password_gate(
            &[stamp, stamp],
            policy,
            ClockSample {
                tick: 80_000,
                wall_ms: 90_000,
            },
        );
        assert!(later.allow);
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
