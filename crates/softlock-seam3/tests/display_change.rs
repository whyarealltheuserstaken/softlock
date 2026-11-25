//! Seam 3, ticket 36 (GitHub #89): a display change while locked leaves
//! every monitor covered. A resolution change is what the hosted images can
//! do. Add and remove run only when the image already has a detached output;
//! nothing here installs a display driver. Each case that cannot run says so
//! in the log, with the words "unproven in CI".

#![cfg(all(windows, feature = "dev-launch"))]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use softlock_lockhost::{DevConfig, DevLock, OVERLAY_MARKER};
use softlock_seam3::{
    Rgb, ScreenRect, attach_output, change_primary_resolution, detached_output, monitor_rects,
    os_identity, stray, uncovered, window_pixels, window_rect,
};

const MARKER_PIXELS: u64 = 8_000;
const SETTLE: Duration = Duration::from_secs(5);

fn gate() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn arm() -> DevLock {
    DevLock::arm(DevConfig {
        authority: Box::new(softlock_service::PinChecker::issue("unused")),
        while_arming: None,
    })
    .expect("lock host did not arm")
}

fn identity_line() {
    let id = os_identity();
    println!(
        "runner product=\"{}\" version=\"{}\" build={}",
        id.product, id.display_version, id.build
    );
}

fn overlay_rects(lock: &DevLock) -> Vec<ScreenRect> {
    lock.overlay_windows()
        .into_iter()
        .filter_map(window_rect)
        .collect()
}

fn painted(lock: &DevLock) -> bool {
    let overlay = Rgb {
        r: OVERLAY_MARKER.0,
        g: OVERLAY_MARKER.1,
        b: OVERLAY_MARKER.2,
    };
    let windows = lock.overlay_windows();
    !windows.is_empty()
        && windows
            .into_iter()
            .all(|hwnd| window_pixels(hwnd, overlay, 0).is_ok_and(|count| count >= MARKER_PIXELS))
}

/// Waits until the overlay set matches the monitor set exactly: every
/// monitor inside a window, no window on a monitor that is gone, and every
/// window painted. Returns the monitors it settled on.
fn settle(lock: &DevLock, what: &str) -> Vec<ScreenRect> {
    let deadline = Instant::now() + SETTLE;
    loop {
        let monitors = monitor_rects();
        let windows = overlay_rects(lock);
        let open = uncovered(&monitors, &windows);
        let extra = stray(&monitors, &windows);
        if open.is_empty() && extra.is_empty() && painted(lock) {
            return monitors;
        }
        if Instant::now() >= deadline {
            panic!(
                "{what}: overlay did not settle; monitors={monitors:?} windows={windows:?} \
                 uncovered={open:?} stray={extra:?}"
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Samples the overlay window count on another thread while `change` runs
/// and the overlay settles. A rebuild must never leave zero windows.
fn without_a_gap<T>(lock: &DevLock, change: impl FnOnce() -> T) -> T {
    let done = AtomicBool::new(false);
    let fewest = AtomicUsize::new(usize::MAX);
    let result = thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                fewest.fetch_min(lock.overlay_windows().len(), Ordering::AcqRel);
                thread::sleep(Duration::from_millis(1));
            }
        });
        let result = change();
        thread::sleep(Duration::from_millis(300));
        done.store(true, Ordering::Release);
        result
    });
    assert_ne!(
        fewest.load(Ordering::Acquire),
        0,
        "the overlay had no windows during the display change"
    );
    result
}

#[test]
fn a_resolution_change_while_locked_keeps_every_monitor_covered() {
    let _guard = gate();
    identity_line();
    let lock = arm();
    let before = settle(&lock, "before the change");

    let change = without_a_gap(&lock, || {
        change_primary_resolution()
            .expect("resolution change")
            .expect("the display driver lists no second mode")
    });
    let during = settle(&lock, "after the resolution change");
    assert_ne!(before, during, "the monitor rectangles did not change");
    println!(
        "resolution change {}x{} -> {}x{} covered={}",
        change.from.0,
        change.from.1,
        change.to.0,
        change.to.1,
        during.len()
    );

    without_a_gap(&lock, || drop(change));
    let after = settle(&lock, "after the restore");
    println!("resolution restored covered={}", after.len());
}

#[test]
fn a_scale_change_is_covered_or_named_unproven() {
    let _guard = gate();
    identity_line();
    // Windows has no documented call that changes a display's scale factor.
    // The ticket allows this outcome when the log names it; the real change
    // is a row on the ThinkStation P7 kit (planning ticket 39).
    let lock = arm();
    let monitors = settle(&lock, "scale check");
    println!(
        "display-dpi unproven in CI: no documented API changes the display scale \
         (covered={} at the current scale)",
        monitors.len()
    );
}

#[test]
fn an_added_display_is_covered_and_a_removed_one_drops_out_or_named_unproven() {
    let _guard = gate();
    identity_line();
    let Some(output) = detached_output() else {
        println!(
            "display-add unproven in CI: no virtual display driver or detached output on this image"
        );
        println!(
            "display-remove unproven in CI: no virtual display driver or detached output on this image"
        );
        return;
    };
    let lock = arm();
    let before = settle(&lock, "before the add");
    let right = before.iter().map(|rect| rect.right).max().unwrap_or(0);

    let attached = without_a_gap(&lock, || attach_output(&output, right));
    let mut attached = match attached {
        Ok(attached) => attached,
        Err(error) => {
            println!("display-add unproven in CI: {output} did not attach ({error})");
            println!("display-remove unproven in CI: nothing was added");
            return;
        }
    };
    let added = settle(&lock, "after the add");
    assert_eq!(
        added.len(),
        before.len() + 1,
        "the added output is not a monitor"
    );
    println!("display-add covered={} output={output}", added.len());

    without_a_gap(&lock, || attached.detach()).expect("detach");
    let removed = settle(&lock, "after the remove");
    assert_eq!(
        removed.len(),
        before.len(),
        "the removed output is still listed"
    );
    println!("display-remove covered={} output={output}", removed.len());
}
