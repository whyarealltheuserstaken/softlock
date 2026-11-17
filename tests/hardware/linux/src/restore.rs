//! Restore grabs, uinput, inhibitors, and a registered xrandr mode.
//! A failure while armed requests `loginctl lock-session` and never unlocks.

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use crate::exec::{self, Stub};
use crate::fail_closed::{FailClosed, on_failure, state_label};
use crate::power::parse_lid_closed_property;

pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

fn exclusive() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// No `SA_RESTART`, so a blocking read returns and the prompt loop can stop.
pub const HANDLER_FLAGS: libc::c_int = 0;

pub const PHYSICAL_INHIBIT_WHAT: &str = "handle-power-key:handle-lid-switch:shutdown:sleep:idle";

struct XrandrMode {
    output: String,
    mode: String,
    rate: String,
}

struct Inner {
    armed: bool,
    grab_fds: Vec<i32>,
    uinput_fd: Option<i32>,
    inhibit: Option<Child>,
    xrandr_restore: Option<XrandrMode>,
    os_lock_requested: bool,
}

impl Inner {
    fn restore(&mut self) {
        if let Some(mut child) = self.inhibit.take() {
            handoff_if_lid_closed();
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(fd) = self.uinput_fd.take() {
            destroy_uinput(fd);
        }
        for fd in self.grab_fds.drain(..) {
            ungrab(fd);
        }
        if let Some(mode) = self.xrandr_restore.take() {
            apply_xrandr(&mode);
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
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_signal as *const () as libc::sighandler_t;
        action.sa_flags = HANDLER_FLAGS;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGHUP, &action, std::ptr::null_mut());
    }
    let _ = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        let _ = writeln!(std::io::stderr(), "panic: {info}");
        if let Ok(mut guard) = state().try_lock() {
            if on_failure(guard.armed) == FailClosed::OsLock {
                issue_lock_session();
                guard.os_lock_requested = true;
                guard.armed = false;
            }
            guard.restore();
        }
    }));
}

pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

pub fn arm() {
    if let Ok(mut guard) = state().lock() {
        guard.armed = true;
    }
}

pub fn disarm() {
    if let Ok(mut guard) = state().lock() {
        guard.armed = false;
    }
}

pub fn is_armed() -> bool {
    state().lock().map(|guard| guard.armed).unwrap_or(true)
}

pub fn register_grab(fd: i32) {
    if let Ok(mut guard) = state().lock() {
        guard.grab_fds.push(fd);
    }
}

pub fn unregister_grab(fd: i32) {
    if let Ok(mut guard) = state().lock() {
        guard.grab_fds.retain(|open| *open != fd);
    }
}

pub fn register_uinput(fd: i32) {
    if let Ok(mut guard) = state().lock() {
        guard.uinput_fd = Some(fd);
    }
}

pub fn unregister_uinput() {
    if let Ok(mut guard) = state().lock() {
        guard.uinput_fd = None;
    }
}

pub fn register_inhibit(child: Child) {
    let previous = state()
        .lock()
        .ok()
        .and_then(|mut guard| guard.inhibit.replace(child));
    if let Some(mut previous) = previous {
        let _ = previous.kill();
        let _ = previous.wait();
    }
}

pub fn register_xrandr(output: String, mode: String, rate: String) {
    if let Ok(mut guard) = state().lock() {
        guard.xrandr_restore = Some(XrandrMode { output, mode, rate });
    }
}

pub fn take_xrandr() -> Option<(String, String, String)> {
    state().lock().ok().and_then(|mut guard| {
        guard
            .xrandr_restore
            .take()
            .map(|mode| (mode.output, mode.mode, mode.rate))
    })
}

pub fn restore_all() -> bool {
    if let Ok(mut guard) = state().lock() {
        guard.restore();
        return true;
    }
    false
}

pub fn fail_closed_if_armed() -> FailClosed {
    let armed = state().lock().map(|guard| guard.armed).unwrap_or(true);
    let action = on_failure(armed);
    if action == FailClosed::OsLock {
        issue_lock_session();
        if let Ok(mut guard) = state().lock() {
            guard.os_lock_requested = true;
            guard.armed = false;
            guard.restore();
        }
    }
    action
}

fn issue_lock_session() {
    let _ = exec::run_cmd("loginctl", &["lock-session"]);
}

pub fn clear_os_lock_requested() {
    if let Ok(mut guard) = state().lock() {
        guard.os_lock_requested = false;
    }
}

/// Arm, request the OS lock through the installed stub, then disarm.
/// The drill does not leave the suite in the os-locked state.
pub fn prove_fail_closed() -> Result<(), String> {
    let _guard = exclusive().lock().map_err(|err| err.to_string())?;
    let stub = std::sync::Arc::new(Stub::new());
    exec::install_stub(stub.clone());
    let saved = take_xrandr();
    arm();
    let action = fail_closed_if_armed();
    exec::clear_stub();
    clear_os_lock_requested();
    disarm();
    if let Some((output, mode, rate)) = saved {
        register_xrandr(output, mode, rate);
    }
    if action != FailClosed::OsLock {
        return Err("armed failure did not request the OS lock".into());
    }
    let calls = stub.calls.lock().map_err(|e| e.to_string())?;
    let locked = calls.iter().any(|(program, args)| {
        program == "loginctl" && args.first().map(String::as_str) == Some("lock-session")
    });
    if !locked {
        return Err("stub did not see lock-session".into());
    }
    if calls
        .iter()
        .any(|(_, args)| args.iter().any(|arg| arg == "unlock-session"))
    {
        return Err("stub saw an unlock".into());
    }
    Ok(())
}

pub fn prove_xrandr_restore() -> Result<(), String> {
    let _guard = exclusive().lock().map_err(|err| err.to_string())?;
    let stub = std::sync::Arc::new(Stub::new());
    exec::install_stub(stub.clone());
    let saved = take_xrandr();
    register_xrandr("eDP-1".into(), "1920x1080".into(), "60.00".into());
    let restored = restore_all();
    exec::clear_stub();
    if let Some((output, mode, rate)) = saved {
        register_xrandr(output, mode, rate);
    }
    if !restored {
        return Err("restore_all failed".into());
    }
    let calls = stub.calls.lock().map_err(|e| e.to_string())?;
    let ok = calls.iter().any(|(program, args)| {
        program == "xrandr"
            && args
                .windows(2)
                .any(|pair| pair[0] == "--rate" && pair[1] == "60.00")
            && args
                .windows(2)
                .any(|pair| pair[0] == "--mode" && pair[1] == "1920x1080")
    });
    if !ok {
        return Err("xrandr restore did not pass --mode and --rate".into());
    }
    Ok(())
}

pub fn final_state(interrupted: bool) -> (bool, String) {
    let (os_lock, restored) = match state().lock() {
        Ok(guard) => (
            guard.os_lock_requested,
            guard.grab_fds.is_empty() && guard.uinput_fd.is_none() && guard.inhibit.is_none(),
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

fn apply_xrandr(mode: &XrandrMode) {
    let _ = exec::run_cmd(
        "xrandr",
        &[
            "--output",
            &mode.output,
            "--mode",
            &mode.mode,
            "--rate",
            &mode.rate,
        ],
    );
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

pub fn hold_sleep_inhibit() -> Result<(), String> {
    let mut child = spawn_inhibit(PHYSICAL_INHIBIT_WHAT, "150", true)?;
    thread::sleep(Duration::from_millis(200));
    if let Ok(Some(status)) = child.try_wait() {
        let err = read_child_stderr(&mut child);
        return Err(format!("inhibit exited {status}: {err}"));
    }
    register_inhibit(child);
    Ok(())
}

pub fn release_sleep_inhibit() {
    let Some(mut child) = take_inhibit_child() else {
        return;
    };
    handoff_if_lid_closed();
    let _ = child.kill();
    let _ = child.wait();
}

fn take_inhibit_child() -> Option<Child> {
    state()
        .lock()
        .ok()
        .and_then(|mut guard| guard.inhibit.take())
}

fn handoff_if_lid_closed() {
    if !lid_is_closed() {
        return;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if !lid_is_closed() {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
    if let Ok(child) = spawn_inhibit("handle-lid-switch:sleep", "60", false) {
        thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });
        thread::sleep(Duration::from_millis(200));
    }
}

pub fn lid_is_closed() -> bool {
    let out = exec::run_cmd(
        "busctl",
        &[
            "get-property",
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "LidClosed",
        ],
    );
    if out.code != Some(0) {
        return false;
    }
    parse_lid_closed_property(&out.stdout) == Some(true)
}

fn spawn_inhibit(what: &str, seconds: &str, die_with_parent: bool) -> Result<Child, String> {
    let mut cmd = Command::new("systemd-inhibit");
    cmd.args([
        &format!("--what={what}"),
        "--who=linux-hardware-suite",
        "--why=suite",
        "--mode=block",
        "sleep",
        seconds,
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(move || {
            if die_with_parent {
                let _ = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            } else {
                let _ = libc::setsid();
            }
            Ok(())
        });
    }
    cmd.spawn().map_err(|err| err.to_string())
}

fn read_child_stderr(child: &mut Child) -> String {
    let mut text = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = std::io::Read::read_to_string(&mut pipe, &mut text);
    }
    text
}

pub fn suite_exit_code(interrupted: bool, fails: u32, serial: bool) -> i32 {
    if serial || interrupted || fails > 0 {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HANDLER_FLAGS, PHYSICAL_INHIBIT_WHAT, prove_fail_closed, prove_xrandr_restore,
        suite_exit_code,
    };

    #[test]
    fn handler_flags_omit_sa_restart() {
        assert_eq!(HANDLER_FLAGS & libc::SA_RESTART, 0);
    }

    #[test]
    fn inhibit_covers_power_key_shutdown_and_sleep() {
        assert!(PHYSICAL_INHIBIT_WHAT.contains("handle-power-key"));
        assert!(PHYSICAL_INHIBIT_WHAT.contains("shutdown"));
        assert!(PHYSICAL_INHIBIT_WHAT.contains("sleep"));
        assert!(PHYSICAL_INHIBIT_WHAT.contains("handle-lid-switch"));
    }

    #[test]
    fn armed_failure_requests_lock_session() {
        prove_fail_closed().expect("lock-session");
    }

    #[test]
    fn xrandr_restore_passes_rate() {
        prove_xrandr_restore().expect("xrandr --rate");
    }

    #[test]
    fn interrupt_is_a_nonzero_exit() {
        assert_eq!(suite_exit_code(true, 0, false), 1);
        assert_eq!(suite_exit_code(false, 0, false), 0);
        assert_eq!(suite_exit_code(false, 1, false), 1);
    }
}
