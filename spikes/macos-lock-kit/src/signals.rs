//! Ctrl-C, closing the terminal, and SIGTERM must end locked.
//!
//! The helper calls `setsid` and ignores SIGINT, SIGHUP, and SIGTERM, so a
//! signal aimed at the parent's process group does not kill it. The parent's
//! handler closes the helper's stdin. End of input without `complete` is the
//! helper's order to lock. The parent then runs the fail-closed fallback, if
//! the session is not locked yet, and exits. It does not write `complete`.

use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, IntoRawFd, RawFd};
use std::process::{Command, Stdio};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::thread;
use std::time::Duration;

use crate::helper::COMPLETE_LINE;

static INTERRUPT: AtomicBool = AtomicBool::new(false);
static HELPER_FD: AtomicI32 = AtomicI32::new(-1);

pub fn tripped() -> bool {
    INTERRUPT.load(Ordering::SeqCst)
}

pub fn detach_helper_session() {
    unsafe {
        libc::setsid();
        ignore(libc::SIGINT);
        ignore(libc::SIGHUP);
        ignore(libc::SIGTERM);
    }
}

pub fn install(helper_stdin: RawFd) {
    HELPER_FD.store(helper_stdin, Ordering::SeqCst);
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_stop as *const () as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        action.sa_flags = 0;
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGHUP, &action, std::ptr::null_mut());
    });
}

pub fn take_fd() -> Option<RawFd> {
    let fd = HELPER_FD.swap(-1, Ordering::SeqCst);
    if fd >= 0 { Some(fd) } else { None }
}

fn ignore(signal: libc::c_int) {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = libc::SIG_IGN;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut());
    }
}

extern "C" fn on_stop(_signal: libc::c_int) {
    INTERRUPT.store(true, Ordering::SeqCst);
    let fd = HELPER_FD.swap(-1, Ordering::SeqCst);
    if fd >= 0 {
        unsafe {
            libc::close(fd);
        }
    }
}

pub fn simulate_from_env() -> Option<i32> {
    let role = std::env::var("MACOS_LOCK_KIT_SIGNAL_SIM").ok()?;
    Some(match role.as_str() {
        "helper" => sim_helper(),
        "parent" => sim_parent(),
        _ => 2,
    })
}

fn sim_helper() -> i32 {
    detach_helper_session();
    let dir = std::env::var("MACOS_LOCK_KIT_SIGNAL_DIR").unwrap_or_default();
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    let outcome = if text.lines().any(|line| line.trim() == COMPLETE_LINE) {
        "stood-down"
    } else {
        "locked"
    };
    let sid = unsafe { libc::getsid(0) };
    let body = format!("outcome={outcome}\nsid={sid}\n");
    let _ = std::fs::write(std::path::Path::new(&dir).join("helper-result.txt"), body);
    0
}

fn sim_parent() -> i32 {
    let dir = std::env::var("MACOS_LOCK_KIT_SIGNAL_DIR").unwrap_or_default();
    let exe =
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("macos-lock-kit"));
    let mut child = match Command::new(exe)
        .env("MACOS_LOCK_KIT_SIGNAL_SIM", "helper")
        .env("MACOS_LOCK_KIT_SIGNAL_DIR", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = std::fs::write(
                std::path::Path::new(&dir).join("parent-error.txt"),
                error.to_string(),
            );
            return 2;
        }
    };
    let Some(stdin) = child.stdin.take() else {
        return 2;
    };
    let fd = stdin.into_raw_fd();
    install(fd);
    let sid = unsafe { libc::getsid(0) };
    let ready = format!("pid={}\nsid={sid}\n", std::process::id());
    let _ = std::fs::write(std::path::Path::new(&dir).join("ready"), ready);
    for _ in 0..400 {
        if tripped() {
            let _ = child.wait();
            return 0;
        }
        thread::sleep(Duration::from_millis(25));
    }
    if let Some(fd) = take_fd() {
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        let _ = writeln!(file, "{COMPLETE_LINE}");
    }
    let _ = child.wait();
    3
}
