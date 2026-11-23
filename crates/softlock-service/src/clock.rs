//! Clocks for the password backoff.
//!
//! Production reads the monotonic tick and the wall clock. A test build can
//! point `SOFTLOCK_CLOCK_FILE` at a `tick_ms wall_ms` file. That hook is not
//! in a default build. The password gate takes the stricter of the two, so a
//! file that moves only one of them does not expire a wait.

use softlock_core::ClockSample;

pub fn password_clock() -> ClockSample {
    #[cfg(any(feature = "simulated-physical", feature = "live-fail-closed"))]
    if let Some(sample) = file_clock() {
        return sample;
    }
    real_clock()
}

#[cfg(any(feature = "simulated-physical", feature = "live-fail-closed"))]
fn file_clock() -> Option<ClockSample> {
    let path = std::env::var("SOFTLOCK_CLOCK_FILE").ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let mut parts = text.split_whitespace();
    let tick = parts.next()?.parse().ok()?;
    let wall_ms = parts.next()?.parse().ok()?;
    if tick == 0 || wall_ms == 0 {
        return None;
    }
    Some(ClockSample { tick, wall_ms })
}

fn real_clock() -> ClockSample {
    let wall_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    #[cfg(windows)]
    let tick = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
    #[cfg(not(windows))]
    let tick = wall_ms;
    ClockSample { tick, wall_ms }
}
