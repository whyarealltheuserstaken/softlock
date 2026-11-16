//! Watchdog timing from the spec. The controller emits failures; it does not
//! call the operating-system lock itself.

use std::time::Duration;

/// Lock host heartbeat period.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(250);

/// Three missed periods is a failure (750 ms, within the one-second bound).
pub const HEARTBEAT_MISSES_BEFORE_FAILURE: u32 = 3;

/// An arming step that takes longer than this is a failure.
pub const ARM_STEP_TIMEOUT: Duration = Duration::from_secs(2);

/// The lock host fails closed when the service has been silent this long.
pub const SERVICE_SILENCE_LIMIT: Duration = Duration::from_secs(1);

pub fn lock_host_fails_closed_when_service_is_silent(silent_for: Duration) -> bool {
    silent_for >= SERVICE_SILENCE_LIMIT
}
