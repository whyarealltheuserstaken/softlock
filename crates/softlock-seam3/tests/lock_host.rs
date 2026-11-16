//! Seam 3 coverage for the dev-launch lock host.
//!
//! Nonce-tagged `SendInput` stands in for physical input. The runner has no
//! physical keyboard. Injected input that is not tagged must still arrive.

#![cfg(all(windows, feature = "dev-launch"))]

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use softlock_lockhost::{
    DevConfig, DevLock, OVERLAY_MARKER, RealLock, RunNonce, clear_run_nonce, current_session_id,
    decide, disconnect_session, set_run_nonce,
};
use softlock_seam3::{
    Probe, Rgb, capture_marker, capture_near, cursor_position, keyboard_focus, send_left_click,
    send_mouse_absolute, send_mouse_move, send_virtual_key, window_pixels,
};

const VK_F13: u16 = 0x7C;
const VK_F14: u16 = 0x7D;
const VK_RETURN: u16 = 0x0D;
const MARKER_PIXELS: u64 = 8_000;
const OVERLAY_PIXEL_LIMIT: u64 = 200;

fn gate() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

struct Nonce(RunNonce);

impl Nonce {
    fn install() -> Self {
        let nonce = RunNonce::random().expect("nonce");
        set_run_nonce(nonce);
        Self(nonce)
    }

    fn extra(&self) -> u64 {
        self.0.as_extra_info()
    }
}

impl Drop for Nonce {
    fn drop(&mut self) {
        clear_run_nonce();
    }
}

fn arm(hash: &str, while_arming: Option<Box<dyn FnOnce() + Send>>) -> DevLock {
    DevLock::arm(DevConfig {
        pin_hash: hash.to_string(),
        while_arming,
    })
    .expect("lock host did not arm")
}

fn wait_for(timeout: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if ready() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    ready()
}

fn type_codes(bytes: &[u8], extra: u64) {
    for byte in bytes {
        send_virtual_key(u16::from(*byte), extra).expect("digit key");
    }
}

fn spec_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let secret = softlock_core::Secret::new(CORRECT);
        softlock_core::hash_softlock_pin(&secret)
            .expect("spec hash")
            .to_string()
    })
}

const CORRECT: &[u8] = b"135742";
const WRONG: &[u8] = b"000000";

#[test]
fn injected_keys_and_clicks_reach_the_probe_while_locked() {
    let _guard = gate();
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let nonce = Nonce::install();
    let _lock = arm("unused", None);

    send_virtual_key(VK_F13, 0).expect("injected key");
    probe
        .wait_for_key(VK_F13, Duration::from_secs(3))
        .expect("injected key did not reach the probe");

    send_virtual_key(VK_F14, nonce.extra()).expect("simulated key");
    thread::sleep(Duration::from_millis(250));
    assert!(
        probe
            .inputs()
            .iter()
            .all(|input| input.virtual_key != VK_F14),
        "simulated-physical key reached the probe while locked"
    );

    let (x, y) = probe.center();
    send_mouse_absolute(x, y, 0).expect("injected move");
    assert!(
        wait_for(Duration::from_secs(2), || {
            cursor_position()
                .map(|(cx, cy)| (cx - x).abs() <= 4 && (cy - y).abs() <= 4)
                .unwrap_or(false)
        }),
        "injected move did not place the cursor on the probe"
    );
    // SendInput delivers the click before it returns, so the count has to
    // be taken first. Waiting for a click that is already recorded times out.
    let before_clicks = probe.clicks().len();
    send_left_click(0).expect("injected click");
    assert!(
        wait_for(Duration::from_secs(3), || {
            probe.clicks().len() > before_clicks
        }),
        "injected click did not reach the probe"
    );
    let clicks = probe.clicks().len();
    send_left_click(nonce.extra()).expect("simulated click");
    thread::sleep(Duration::from_millis(250));
    assert_eq!(
        probe.clicks().len(),
        clicks,
        "simulated-physical click reached the probe while locked"
    );
}

#[test]
fn locking_leaves_keyboard_focus_unchanged() {
    let _guard = gate();
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    assert!(
        wait_for(Duration::from_secs(2), || {
            keyboard_focus().foreground == probe.hwnd()
        }),
        "probe did not take focus before locking"
    );
    let before = keyboard_focus();
    let _lock = arm("unused", None);
    let after = keyboard_focus();
    assert_eq!(
        after.foreground, before.foreground,
        "locking changed the foreground window"
    );
    assert_eq!(after.focus, before.focus, "locking changed keyboard focus");
}

#[test]
fn captures_omit_the_overlay_marker() {
    let _guard = gate();
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let lock = arm("unused", None);
    let overlay_windows = lock.overlay_windows();
    assert!(
        !overlay_windows.is_empty(),
        "locking did not create an overlay window"
    );
    thread::sleep(Duration::from_millis(150));
    probe.repaint();
    let overlay = Rgb {
        r: OVERLAY_MARKER.0,
        g: OVERLAY_MARKER.1,
        b: OVERLAY_MARKER.2,
    };
    for hwnd in overlay_windows {
        let painted = window_pixels(hwnd, overlay, 0).expect("overlay window dc");
        assert!(
            painted >= MARKER_PIXELS,
            "overlay window did not paint its colour ({painted} pixels)"
        );
    }
    let shots = capture_near(probe.hwnd(), overlay, 0).expect("overlay capture");
    assert_eq!(shots.len(), 3);
    for shot in &shots {
        assert!(
            shot.marker_pixels < OVERLAY_PIXEL_LIMIT,
            "{:?} capture contained the overlay colour ({} pixels)",
            shot.method,
            shot.marker_pixels
        );
    }
    let probe_shots = capture_marker(probe.hwnd()).expect("probe capture");
    for shot in &probe_shots {
        assert!(
            shot.marker_pixels >= MARKER_PIXELS,
            "{:?} capture missed the probe ({})",
            shot.method,
            shot.marker_pixels
        );
    }
}

#[test]
fn simulated_physical_mouse_does_not_move_the_cursor() {
    let _guard = gate();
    let nonce = Nonce::install();
    let _lock = arm("unused", None);
    send_mouse_absolute(200, 200, 0).expect("place cursor");
    assert!(
        wait_for(Duration::from_secs(2), || {
            cursor_position()
                .map(|(x, y)| (x - 200).abs() <= 4 && (y - 200).abs() <= 4)
                .unwrap_or(false)
        }),
        "injected move did not place the cursor"
    );
    let (start_x, _) = cursor_position().expect("cursor");
    send_mouse_move(40, 0, 0).expect("injected relative move");
    assert!(
        wait_for(Duration::from_secs(2), || {
            cursor_position()
                .map(|(x, _)| x != start_x)
                .unwrap_or(false)
        }),
        "injected relative move did not move the cursor"
    );
    let parked = cursor_position().expect("cursor");
    send_mouse_move(40, 0, nonce.extra()).expect("simulated move");
    thread::sleep(Duration::from_millis(250));
    assert_eq!(
        cursor_position().expect("cursor"),
        parked,
        "simulated-physical mouse movement moved the cursor"
    );
}

#[test]
fn arming_stays_under_500ms_and_swallows_physical_keys() {
    let _guard = gate();
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let (tx, rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    let lock = DevLock::arm(DevConfig {
        pin_hash: "unused".to_string(),
        while_arming: Some(Box::new(move || {
            let mut sent = 0u32;
            for _ in 0..8 {
                if send_virtual_key(VK_F13, extra).is_ok() {
                    sent += 1;
                }
            }
            let _ = tx.send(sent);
        })),
    })
    .expect("lock host did not arm");
    let measured = lock.elapsed();
    // No newline: the test harness finishes the same line with `ok`.
    print!("arming took {} ms ", measured.as_millis());
    let _ = std::io::Write::flush(&mut std::io::stdout());
    assert!(
        measured < Duration::from_millis(500),
        "arming took {measured:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "arm() returned after {:?}",
        started.elapsed()
    );
    assert_eq!(rx.recv().expect("arming callback"), 8);
    thread::sleep(Duration::from_millis(200));
    assert!(
        probe.inputs().is_empty(),
        "a key sent while arming reached the probe: {:?}",
        probe.inputs()
    );
}

#[test]
fn correct_pin_unlocks_and_wrong_pin_does_not() {
    let _guard = gate();
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let lock = arm(spec_hash(), None);

    send_virtual_key(VK_F13, extra).expect("wake");
    assert!(
        wait_for(Duration::from_secs(3), || lock.prompt_open()),
        "the wake key did not open the pin prompt"
    );
    assert_eq!(
        lock.pin_chars(),
        0,
        "the wake key was stored as a pin character"
    );

    type_codes(WRONG, extra);
    assert!(
        wait_for(Duration::from_secs(3), || lock.pin_chars() == 6),
        "the wrong pin was not collected"
    );
    send_virtual_key(VK_RETURN, extra).expect("submit");
    assert!(
        wait_for(Duration::from_secs(20), || {
            lock.pin_chars() == 0 && lock.is_locked() && lock.prompt_open()
        }),
        "the wrong pin did not leave the host locked"
    );

    send_virtual_key(VK_F14, extra).expect("still locked");
    thread::sleep(Duration::from_millis(200));
    assert!(
        probe
            .inputs()
            .iter()
            .all(|input| input.virtual_key != VK_F14),
        "simulated-physical input reached the probe after a wrong pin"
    );

    type_codes(CORRECT, extra);
    send_virtual_key(VK_RETURN, extra).expect("submit");
    assert!(
        wait_for(Duration::from_secs(25), || !lock.is_locked()),
        "the correct pin did not unlock"
    );

    send_virtual_key(VK_F13, extra).expect("after unlock");
    probe
        .wait_for_key(VK_F13, Duration::from_secs(3))
        .expect("simulated-physical key did not reach the probe after unlock");
    assert!(
        probe
            .inputs()
            .iter()
            .all(|input| !(0x30..=0x39).contains(&input.virtual_key)),
        "a typed pin character reached the probe"
    );
}

#[test]
fn wts_disconnect_on_an_unused_session_fails() {
    let _guard = gate();
    let before = current_session_id();
    // `0xFFFFFFFF` is WTS_CURRENT_SESSION and would disconnect this runner.
    let unused = 0x00FE_FFFE;
    assert!(
        !disconnect_session(unused),
        "disconnecting an unused session should fail"
    );
    assert_eq!(
        current_session_id(),
        before,
        "the failed disconnect changed this session"
    );
}

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo) + Send + Sync + 'static>;

struct RestoreHook(Option<PanicHook>);

impl Drop for RestoreHook {
    fn drop(&mut self) {
        // `set_hook` panics when the thread is already unwinding. That second
        // panic fail-fasts the process (0xc0000409) and hides the assertion.
        if std::thread::panicking() {
            return;
        }
        if let Some(hook) = self.0.take() {
            std::panic::set_hook(hook);
        }
    }
}

fn quiet_hook() -> RestoreHook {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    RestoreHook(Some(previous))
}

fn panic_log_hook() -> Option<RestoreHook> {
    let path = std::env::var_os("SOFTLOCK_PANIC_LOG")?;
    if path.is_empty() {
        return None;
    }
    let _ = panic_log_path();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string payload");
        let location = info
            .location()
            .map(|place| place.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("");
        panic_line(&format!(
            "panic thread={thread:?} name={name} loc={location} msg={message}"
        ));
    }));
    Some(RestoreHook(Some(previous)))
}

fn panic_log_path() -> Option<&'static std::ffi::OsStr> {
    static PATH: OnceLock<Option<std::ffi::OsString>> = OnceLock::new();
    PATH.get_or_init(|| std::env::var_os("SOFTLOCK_PANIC_LOG"))
        .as_ref()
        .map(std::ffi::OsString::as_os_str)
}

fn panic_note(message: &str) {
    if panic_log_path().is_none() {
        return;
    }
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("");
    panic_line(&format!("note thread={thread:?} name={name} {message}"));
}

fn panic_line(text: &str) {
    let Some(path) = panic_log_path() else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(file) => file,
        Err(_) => return,
    };
    use std::io::Write;
    let _ = file.write_all(text.as_bytes());
    let _ = file.write_all(b"\n");
    let _ = file.flush();
    let _ = file.sync_all();
}

fn wake(lock: &DevLock, extra: u64) {
    send_virtual_key(VK_F13, extra).expect("wake");
    assert!(
        wait_for(Duration::from_secs(3), || lock.prompt_open()),
        "the wake key did not open the pin prompt"
    );
}

fn submit_pin(lock: &DevLock, bytes: &[u8], extra: u64) {
    type_codes(bytes, extra);
    let count = u32::try_from(bytes.len()).unwrap_or(0);
    assert!(
        wait_for(Duration::from_secs(15), || lock.pin_chars() == count),
        "the pin was not collected"
    );
    send_virtual_key(VK_RETURN, extra).expect("submit");
}

#[test]
fn a_correct_pin_during_backoff_stays_locked() {
    let _guard = gate();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let lock = arm(spec_hash(), None);
    wake(&lock, extra);
    for _ in 0..4 {
        submit_pin(&lock, WRONG, extra);
        assert!(
            wait_for(Duration::from_secs(20), || {
                lock.pin_chars() == 0 && lock.is_locked()
            }),
            "a wrong pin did not leave the host locked"
        );
    }
    submit_pin(&lock, CORRECT, extra);
    assert!(
        wait_for(Duration::from_secs(20), || {
            lock.pin_chars() == 0 && lock.is_locked()
        }),
        "a correct pin during backoff unlocked"
    );
    thread::sleep(Duration::from_millis(400));
    assert!(
        lock.is_locked(),
        "the host unlocked before backoff had expired"
    );
}

#[test]
fn backoff_expires_after_the_advanced_time() {
    let _guard = gate();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let lock = arm(spec_hash(), None);
    wake(&lock, extra);
    for _ in 0..4 {
        submit_pin(&lock, WRONG, extra);
        assert!(
            wait_for(Duration::from_secs(20), || {
                lock.pin_chars() == 0 && lock.is_locked()
            }),
            "a wrong pin did not leave the host locked"
        );
    }
    submit_pin(&lock, CORRECT, extra);
    assert!(
        wait_for(Duration::from_secs(20), || {
            lock.pin_chars() == 0 && lock.is_locked()
        }),
        "a correct pin during backoff unlocked"
    );
    // The window loop feeds wall time in with `Event::AdvanceTime`. The
    // fourth failure waits 30 seconds. This sleep is that advanced time.
    thread::sleep(Duration::from_secs(32));
    assert!(lock.is_locked(), "the host unlocked during the wait");
    submit_pin(&lock, CORRECT, extra);
    assert!(
        wait_for(Duration::from_secs(25), || !lock.is_locked()),
        "the correct pin did not unlock after backoff expired"
    );
}

#[test]
fn the_heartbeat_watchdog_fires() {
    let _guard = gate();
    let lock = arm("unused", None);
    lock.stub_os_lock_unavailable();
    lock.hold_heartbeats();
    assert!(
        wait_for(Duration::from_secs(3), || lock.heartbeat_missed()),
        "the heartbeat watchdog did not fail closed"
    );
    assert!(lock.is_locked(), "the watchdog released the soft lock");
    assert!(
        !lock.overlay_windows().is_empty(),
        "the watchdog took the overlay down"
    );
    assert!(
        lock.os_lock_attempts() >= 1,
        "the watchdog did not try the OS lock"
    );
    let notice = lock.overlay_notice().expect("overlay error");
    assert!(notice.contains("Windows lock did not engage"));
}

#[test]
fn both_os_lock_paths_failing_keeps_the_guard_and_retries() {
    let _guard = gate();
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let lock = arm(spec_hash(), None);
    // Stubbed Windows calls. `decide` is the real decision both paths use.
    assert_eq!(decide(false, false), RealLock::Unavailable);
    lock.stub_os_lock_unavailable();
    lock.signal_guard_down();
    assert!(
        wait_for(Duration::from_secs(3), || {
            lock.os_lock_attempts() >= 1 && lock.os_lock_error() && lock.is_locked()
        }),
        "both OS lock paths failing did not stay locked"
    );
    assert!(
        !lock.overlay_windows().is_empty(),
        "the overlay came down when both OS lock paths failed"
    );
    let notice = lock.overlay_notice().expect("overlay error");
    assert!(notice.contains("Ctrl+Alt+Del"));

    send_virtual_key(VK_F14, extra).expect("while guarded");
    thread::sleep(Duration::from_millis(200));
    assert!(
        probe
            .inputs()
            .iter()
            .all(|input| input.virtual_key != VK_F14),
        "simulated-physical input reached the probe while the OS lock was unavailable"
    );
    assert!(
        wait_for(Duration::from_secs(3), || lock.os_lock_attempts() >= 2),
        "the OS lock was not retried"
    );

    wake(&lock, extra);
    submit_pin(&lock, WRONG, extra);
    assert!(
        wait_for(Duration::from_secs(20), || {
            lock.pin_chars() == 0 && lock.is_locked()
        }),
        "a wrong pin released the guard"
    );
    submit_pin(&lock, CORRECT, extra);
    assert!(
        wait_for(Duration::from_secs(25), || !lock.is_locked()),
        "a verified pin did not release the guard"
    );
    assert!(
        lock.overlay_windows().is_empty(),
        "the overlay stayed up after the verified pin"
    );
    send_virtual_key(VK_F14, extra).expect("after release");
    probe
        .wait_for_key(VK_F14, Duration::from_secs(3))
        .expect("input stayed blocked after the verified pin");
}

#[test]
fn a_liveness_miss_fail_closes() {
    let _guard = gate();
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let lock = arm("unused", None);
    lock.stub_os_lock_unavailable();
    lock.ignore_liveness_marker();
    assert!(
        wait_for(Duration::from_secs(4), || {
            lock.os_lock_attempts() >= 1 && lock.os_lock_error() && lock.is_locked()
        }),
        "a liveness miss did not fail closed"
    );
    assert!(
        !lock.overlay_windows().is_empty(),
        "a liveness miss took the overlay down"
    );
    // The hook is still installed. It stopped counting the marker; it did
    // not die. Input stays blocked. A hook that had actually died would not
    // be blocking input.
    send_virtual_key(VK_F14, extra).expect("while guarded");
    thread::sleep(Duration::from_millis(250));
    assert!(
        probe
            .inputs()
            .iter()
            .all(|input| input.virtual_key != VK_F14),
        "simulated-physical input reached the probe after the liveness miss"
    );
}

#[test]
fn a_window_thread_panic_tries_the_os_lock_then_drops_the_hooks() {
    let _guard = gate();
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let lock = arm("unused", None);
    lock.stub_os_lock_unavailable();
    // The panic is caught. A quiet hook keeps the test line as `... ok`.
    let _quiet = quiet_hook();
    lock.panic_window_thread();
    assert!(
        wait_for(Duration::from_secs(3), || {
            !lock.is_locked() && lock.overlay_windows().is_empty() && lock.os_lock_attempts() >= 1
        }),
        "a window-thread panic left the guard up or skipped the OS lock"
    );
    send_virtual_key(VK_F14, extra).expect("after panic");
    probe
        .wait_for_key(VK_F14, Duration::from_secs(3))
        .expect("input stayed blocked after the window thread panicked");
}

fn enter_failing_closed(lock: &DevLock) {
    lock.stub_os_lock_unavailable();
    lock.signal_guard_down();
    assert!(
        wait_for(Duration::from_secs(3), || {
            lock.os_lock_error() && lock.is_locked()
        }),
        "the guard-down did not fail closed"
    );
}

fn five_wrong_pins(lock: &DevLock, extra: u64) {
    wake(lock, extra);
    for _ in 0..5 {
        submit_pin(lock, WRONG, extra);
        assert!(
            wait_for(Duration::from_secs(20), || {
                lock.pin_chars() == 0 && lock.is_locked()
            }),
            "a wrong pin released the guard"
        );
    }
}

#[test]
fn failing_closed_wrong_pins_then_a_correct_pin_during_backoff_stays_locked() {
    let _guard = gate();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let lock = arm(spec_hash(), None);
    enter_failing_closed(&lock);
    five_wrong_pins(&lock, extra);
    submit_pin(&lock, CORRECT, extra);
    assert!(
        wait_for(Duration::from_secs(5), || {
            lock.pin_chars() == 0 && lock.is_locked()
        }),
        "a correct pin during failing-closed backoff released the guard"
    );
    thread::sleep(Duration::from_millis(400));
    assert!(
        lock.is_locked(),
        "the guard released before failing-closed backoff expired"
    );
    assert!(
        !lock.overlay_windows().is_empty(),
        "the overlay came down during failing-closed backoff"
    );
}

#[test]
fn failing_closed_correct_pin_releases_after_backoff() {
    let _guard = gate();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let lock = arm(spec_hash(), None);
    enter_failing_closed(&lock);
    five_wrong_pins(&lock, extra);
    submit_pin(&lock, CORRECT, extra);
    assert!(
        wait_for(Duration::from_secs(5), || {
            lock.pin_chars() == 0 && lock.is_locked()
        }),
        "a correct pin during failing-closed backoff released the guard"
    );
    thread::sleep(Duration::from_secs(32));
    assert!(lock.is_locked(), "the guard released during the wait");
    submit_pin(&lock, CORRECT, extra);
    assert!(
        wait_for(Duration::from_secs(25), || !lock.is_locked()),
        "a correct pin did not release the guard after failing-closed backoff"
    );
}

#[test]
fn a_hook_thread_panic_tries_the_os_lock_then_drops_the_hooks() {
    let _guard = gate();
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let lock = arm("unused", None);
    lock.stub_os_lock_unavailable();
    let _quiet = quiet_hook();
    lock.panic_hook_thread();
    assert!(
        wait_for(Duration::from_secs(3), || lock.os_lock_attempts() >= 1),
        "a hook-thread panic did not try the OS lock"
    );
    send_virtual_key(VK_F14, extra).expect("after hook panic");
    probe
        .wait_for_key(VK_F14, Duration::from_secs(3))
        .expect("input stayed trapped after the hook thread panicked");
}

#[test]
fn a_liveness_thread_panic_tries_the_os_lock_without_trapping_input() {
    let _guard = gate();
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let lock = arm("unused", None);
    lock.stub_os_lock_unavailable();
    let _quiet = quiet_hook();
    lock.panic_liveness_thread();
    assert!(
        wait_for(Duration::from_secs(3), || {
            lock.os_lock_attempts() >= 1 && lock.is_locked() && !lock.overlay_windows().is_empty()
        }),
        "a liveness panic did not keep the overlay up after trying the OS lock"
    );
    // The window thread is still up, so the guard is the lock, not a trap.
    send_virtual_key(VK_F14, extra).expect("while the guard remains");
    thread::sleep(Duration::from_millis(250));
    assert!(
        probe
            .inputs()
            .iter()
            .all(|input| input.virtual_key != VK_F14),
        "the liveness panic removed the guard and left input open"
    );
}

#[test]
fn a_callback_panic_tries_the_os_lock_then_drops_the_hooks() {
    let _guard = gate();
    // When SOFTLOCK_PANIC_LOG is set, record every panic's location and
    // thread for the whole test. Otherwise install the quiet hook only after
    // arming, so a caught callback panic still prints as `ok`.
    let _logged = panic_log_hook();
    panic_note("test-start");
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    panic_note("before-arm");
    let lock = arm("unused", None);
    panic_note("armed");
    lock.stub_os_lock_unavailable();
    let _quiet = if std::env::var_os("SOFTLOCK_PANIC_LOG").is_some() {
        None
    } else {
        Some(quiet_hook())
    };
    lock.panic_in_callback();
    panic_note("flag-set");
    send_virtual_key(VK_F13, extra).expect("callback panic");
    panic_note("send-returned");
    assert!(
        wait_for(Duration::from_secs(3), || lock.os_lock_attempts() >= 1),
        "a callback panic did not try the OS lock"
    );
    panic_note("os-lock-seen");
    send_virtual_key(VK_F14, extra).expect("after callback panic");
    probe
        .wait_for_key(VK_F14, Duration::from_secs(3))
        .expect("input stayed trapped after the hook callback panicked");
    panic_note("test-end");
}
