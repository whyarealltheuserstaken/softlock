//! Seam 3 coverage for the dev-launch lock host.
//!
//! Nonce-tagged `SendInput` stands in for physical input. The runner has no
//! physical keyboard. Injected input that is not tagged must still arrive.

#![cfg(all(windows, feature = "dev-launch"))]

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use softlock_lockhost::{
    DevConfig, DevLock, OVERLAY_MARKER, RunNonce, clear_run_nonce, current_session_id,
    disconnect_session, last_mouse_hook_sample, set_run_nonce,
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
        "simulated-physical click reached the probe while locked; hook={:?} nonce={:#x}",
        last_mouse_hook_sample(),
        nonce.extra()
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
        "simulated-physical mouse movement moved the cursor; hook={:?} nonce={:#x}",
        last_mouse_hook_sample(),
        nonce.extra()
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
    assert!(
        lock.elapsed() < Duration::from_millis(500),
        "arming took {:?}",
        lock.elapsed()
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

#[test]
fn both_os_lock_paths_failing_releases_the_soft_lock() {
    let _guard = gate();
    let probe = Probe::open().expect("probe");
    probe.claim_focus();
    let nonce = Nonce::install();
    let extra = nonce.extra();
    let lock = arm("unused", None);

    send_virtual_key(VK_F14, extra).expect("while locked");
    thread::sleep(Duration::from_millis(200));
    assert!(
        probe
            .inputs()
            .iter()
            .all(|input| input.virtual_key != VK_F14),
        "simulated-physical input reached the probe before the release"
    );

    // Same release the platform runs when LockWorkStation did not lock and
    // WTSDisconnectSession did not disconnect. Neither API is called here:
    // calling them on this session would lock or disconnect the runner.
    lock.release_soft_lock();
    assert!(
        !lock.is_locked(),
        "the soft lock stayed up after both OS paths were unavailable"
    );
    assert!(
        lock.overlay_windows().is_empty(),
        "the overlay stayed up after both OS paths were unavailable"
    );

    send_virtual_key(VK_F14, extra).expect("after release");
    probe
        .wait_for_key(VK_F14, Duration::from_secs(3))
        .expect("input stayed blocked after both OS lock paths failed");
}
