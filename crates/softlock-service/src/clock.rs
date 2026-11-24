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

#[cfg(all(test, windows, feature = "simulated-physical"))]
mod gate_tests {
    use std::sync::Mutex;

    use super::file_clock;

    static ENV: Mutex<()> = Mutex::new(());

    #[test]
    fn file_clock_returns_none_when_the_runner_may_not_lock() {
        let _guard = ENV.lock().unwrap_or_else(|poison| poison.into_inner());
        let dir = std::env::temp_dir().join(format!("sl-clock-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("clock");
        std::fs::write(&path, "5000 6000").expect("clock file");
        let clock = std::env::var("SOFTLOCK_CLOCK_FILE").ok();
        let actions = std::env::var("GITHUB_ACTIONS").ok();
        let runner = std::env::var("RUNNER_ENVIRONMENT").ok();
        struct Restore {
            clock: Option<String>,
            actions: Option<String>,
            runner: Option<String>,
            path: std::path::PathBuf,
        }
        impl Drop for Restore {
            fn drop(&mut self) {
                restore("SOFTLOCK_CLOCK_FILE", &self.clock);
                restore("GITHUB_ACTIONS", &self.actions);
                restore("RUNNER_ENVIRONMENT", &self.runner);
                let _ = std::fs::remove_file(&self.path);
            }
        }
        let _restore = Restore {
            clock,
            actions,
            runner,
            path: path.clone(),
        };
        unsafe {
            std::env::set_var("SOFTLOCK_CLOCK_FILE", &path);
            std::env::remove_var("GITHUB_ACTIONS");
            std::env::remove_var("RUNNER_ENVIRONMENT");
        }
        assert!(
            softlock_windows::runner_may_lock().is_err(),
            "the runner gate stayed open"
        );
        assert!(
            file_clock().is_none(),
            "SOFTLOCK_CLOCK_FILE was read while the runner gate was shut"
        );
    }

    fn restore(name: &str, value: &Option<String>) {
        unsafe {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
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
