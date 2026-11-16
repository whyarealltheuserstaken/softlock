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
