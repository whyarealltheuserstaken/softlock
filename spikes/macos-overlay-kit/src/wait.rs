//! Prompt wait that pumps, polls stdin, and honours stop, Escape, and a timeout.
//! Ctrl-C does not wait for Enter: poll returns at least every `POLL_MS`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub const POLL_MS: i32 = 100;
pub const DEFAULT_TIMEOUT_MS: u64 = 600_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitOutcome {
    Line,
    Eof,
    Interrupted,
    Escape,
    Timeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollResult {
    Idle,
    Ready,
    Eof,
    Interrupted,
}

pub fn timed_out(started: Instant, timeout_ms: u64) -> bool {
    started.elapsed() >= Duration::from_millis(timeout_ms)
}

/// One 100 ms slice: stop and Escape beat stdin, then timeout, then poll.
pub fn wait_slice(
    started: Instant,
    timeout_ms: u64,
    stop: bool,
    escape: bool,
    poll: PollResult,
) -> Option<WaitOutcome> {
    if stop {
        return Some(WaitOutcome::Interrupted);
    }
    if escape {
        return Some(WaitOutcome::Escape);
    }
    if timed_out(started, timeout_ms) {
        return Some(WaitOutcome::Timeout);
    }
    match poll {
        PollResult::Idle => None,
        PollResult::Ready => Some(WaitOutcome::Line),
        PollResult::Eof => Some(WaitOutcome::Eof),
        PollResult::Interrupted => Some(WaitOutcome::Interrupted),
    }
}

/// Sets `stop` after `timeout_ms`. The main thread tears the overlay down.
pub fn spawn_watchdog(timeout_ms: u64, stop: Arc<AtomicBool>, timed_out_flag: Arc<AtomicBool>) {
    thread::spawn(move || {
        let slice = Duration::from_millis(POLL_MS as u64);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(slice.min(deadline.saturating_duration_since(Instant::now())));
        }
        timed_out_flag.store(true, Ordering::Relaxed);
        stop.store(true, Ordering::Relaxed);
    });
}

#[cfg(unix)]
pub fn poll_stdin(timeout_ms: i32) -> PollResult {
    let mut fds = [libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    }];
    let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout_ms) };
    if rc < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::Interrupted {
            return PollResult::Interrupted;
        }
        return PollResult::Eof;
    }
    if rc == 0 {
        return PollResult::Idle;
    }
    if fds[0].revents & libc::POLLHUP != 0 && fds[0].revents & libc::POLLIN == 0 {
        return PollResult::Eof;
    }
    if fds[0].revents & libc::POLLIN != 0 {
        return PollResult::Ready;
    }
    PollResult::Idle
}

#[cfg(not(unix))]
pub fn poll_stdin(_timeout_ms: i32) -> PollResult {
    PollResult::Idle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_c_during_a_prompt_does_not_wait_for_enter() {
        let started = Instant::now();
        assert_eq!(
            wait_slice(started, 60_000, true, false, PollResult::Idle),
            Some(WaitOutcome::Interrupted)
        );
        assert_eq!(
            wait_slice(started, 60_000, false, true, PollResult::Ready),
            Some(WaitOutcome::Escape)
        );
        assert_eq!(
            wait_slice(started, 60_000, false, false, PollResult::Idle),
            None
        );
        assert_eq!(POLL_MS, 100);
    }

    #[test]
    fn timeout_watchdog_sets_stop_without_an_enter_key() {
        let stop = Arc::new(AtomicBool::new(false));
        let timed = Arc::new(AtomicBool::new(false));
        spawn_watchdog(40, stop.clone(), timed.clone());
        let deadline = Instant::now() + Duration::from_millis(800);
        while Instant::now() < deadline
            && (!stop.load(Ordering::Relaxed) || !timed.load(Ordering::Relaxed))
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(timed.load(Ordering::Relaxed));
        assert!(stop.load(Ordering::Relaxed));
        assert_eq!(
            wait_slice(
                Instant::now() - Duration::from_secs(20),
                1_000,
                false,
                false,
                PollResult::Idle
            ),
            Some(WaitOutcome::Timeout)
        );
    }
}
