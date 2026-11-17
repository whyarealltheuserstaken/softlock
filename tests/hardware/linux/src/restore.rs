//! Restore grabs, uinput, inhibitors, and display mode. Fail-closed lock.

use std::fs;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::fail_closed::{FORBIDDEN_UNLOCK, FailClosed, on_failure, state_label};

pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

struct Inner {
    armed: bool,
    grab_fds: Vec<i32>,
    uinput_fd: Option<i32>,
    inhibit: Option<Child>,
    xrandr_restore: Option<(String, String)>,
    os_lock_requested: bool,
}

impl Inner {
    fn restore(&mut self) {
        if let Some(mut child) = self.inhibit.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(fd) = self.uinput_fd.take() {
            destroy_uinput(fd);
        }
        for fd in self.grab_fds.drain(..) {
            ungrab(fd);
        }
        if let Some((output, mode)) = self.xrandr_restore.take() {
            let _ = Command::new("xrandr")
                .args(["--output", &output, "--mode", &mode])
                .status();
        }
    }
}

fn state() -> &'static Mutex<Inner> {
    static STATE: OnceLock<Mutex<Inner>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(Inner {
            armed: false,
            grab_fds: Vec::new(),
            uinput_fd: None,
            inhibit: None,
            xrandr_restore: None,
            os_lock_requested: false,
        })
    })
}

extern "C" fn on_signal(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

pub fn install_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_signal as *const () as libc::sighandler_t);
    }
    let _ = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        let _ = writeln!(std::io::stderr(), "panic: {info}");
        emergency();
    }));
}

pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

pub fn arm() {
    if let Ok(mut g) = state().lock() {
        g.armed = true;
    }
}

pub fn disarm() {
    if let Ok(mut g) = state().lock() {
        g.armed = false;
    }
}

pub fn register_grab(fd: i32) {
    if let Ok(mut g) = state().lock() {
        g.grab_fds.push(fd);
    }
}

pub fn unregister_grab(fd: i32) {
    if let Ok(mut g) = state().lock() {
        g.grab_fds.retain(|x| *x != fd);
    }
}

pub fn register_uinput(fd: i32) {
    if let Ok(mut g) = state().lock() {
        g.uinput_fd = Some(fd);
    }
}

pub fn unregister_uinput() {
    if let Ok(mut g) = state().lock() {
        g.uinput_fd = None;
    }
}

pub fn register_inhibit(child: Child) {
    if let Ok(mut g) = state().lock() {
        g.inhibit = Some(child);
    }
}

pub fn take_inhibit() -> Option<Child> {
    state().lock().ok().and_then(|mut g| g.inhibit.take())
}

pub fn register_xrandr(output: String, mode: String) {
    if let Ok(mut g) = state().lock() {
        g.xrandr_restore = Some((output, mode));
    }
}

pub fn clear_xrandr() {
    if let Ok(mut g) = state().lock() {
        g.xrandr_restore = None;
    }
}

pub fn restore_all() -> bool {
    if let Ok(mut g) = state().lock() {
        g.restore();
        return true;
    }
    false
}

pub fn fail_closed_if_armed() -> FailClosed {
    let armed = state().lock().map(|g| g.armed).unwrap_or(true);
    let action = on_failure(armed);
    if action == FailClosed::OsLock {
        request_os_lock();
    }
    action
}

pub fn request_os_lock() {
    // Never unlock. lock-session is the fail-closed path.
    let _ = FORBIDDEN_UNLOCK.len();
    let _ = Command::new("loginctl")
        .arg("lock-session")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if let Ok(mut g) = state().lock() {
        g.os_lock_requested = true;
        g.armed = false;
        g.restore();
    }
}

pub fn emergency() {
    let _ = fail_closed_if_armed();
    let _ = restore_all();
}

pub fn final_state(interrupted: bool) -> (bool, String) {
    let (os_lock, restored) = match state().lock() {
        Ok(g) => (
            g.os_lock_requested,
            g.grab_fds.is_empty() && g.uinput_fd.is_none(),
        ),
        Err(_) => (true, false),
    };
    let action = if os_lock {
        FailClosed::OsLock
    } else {
        FailClosed::Idle
    };
    (restored, state_label(action, interrupted).to_string())
}

fn ungrab(fd: i32) {
    unsafe {
        let _ = libc::ioctl(fd, crate::input::EVIOCGRAB, 0);
        libc::close(fd);
    }
}

fn destroy_uinput(fd: i32) {
    unsafe {
        let _ = libc::ioctl(fd, crate::input::UI_DEV_DESTROY, 0);
        libc::close(fd);
    }
}

/// Keep a sleep inhibit during a physical wait so a lid or power key does not suspend.
pub fn hold_sleep_inhibit() -> Result<(), String> {
    let mut child = Command::new("systemd-inhibit")
        .args([
            "--what=sleep:idle:handle-lid-switch",
            "--who=linux-hardware-suite",
            "--why=physical-wait",
            "--mode=block",
            "sleep",
            "120",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    // Give logind a moment; if the child died, skip.
    std::thread::sleep(std::time::Duration::from_millis(100));
    if let Ok(Some(status)) = child.try_wait() {
        return Err(format!("inhibit exited {status}"));
    }
    register_inhibit(child);
    Ok(())
}

pub fn release_sleep_inhibit() {
    if let Some(mut child) = take_inhibit() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub fn lock_marker_path() -> &'static str {
    "/tmp/linux-hardware-suite.lock"
}

pub fn write_lock_marker() {
    let _ = fs::write(lock_marker_path(), "armed\n");
}

pub fn clear_lock_marker() {
    let _ = fs::remove_file(lock_marker_path());
}
