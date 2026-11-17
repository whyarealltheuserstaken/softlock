//! Ctrl-C, closing the terminal, and SIGTERM must end locked.
//!
//! `arm_helper` detaches the helper in `pre_exec`, before `exec`. `setsid`
//! and the ignores for SIGINT, SIGHUP, and SIGTERM run in the child, so a
//! signal aimed at the parent's process group does not kill the helper in
//! the window before its `main` starts. The parent's handler closes the
//! helper's stdin. End of input without `complete` is the helper's order to
//! lock. The parent then waits for `helper-result.txt` and records
//! `helper-end` only when the file carries this run's nonce. A stale file is
//! removed before the helper starts. If it cannot be removed, the helper is
//! not armed. An unread or wrong-nonce file is not `never-unlocked`.

use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, IntoRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::thread;
use std::time::Duration;

use crate::helper::{
    COMPLETE_LINE, HELPER_ENV, HELPER_NONCE_ENV, prepare_helper_slot, write_helper_result,
};

static INTERRUPT: AtomicBool = AtomicBool::new(false);
static HELPER_FD: AtomicI32 = AtomicI32::new(-1);

pub struct Armed {
    pub child: Child,
    pub nonce: String,
    finished: bool,
}

impl Armed {
    pub fn stand_down(&mut self) {
        if self.finished {
            return;
        }
        if !tripped()
            && let Some(fd) = take_fd()
        {
            let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
            let _ = writeln!(file, "{COMPLETE_LINE}");
        }
        let _ = self.child.wait();
        self.finished = true;
    }

    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        let _ = self.child.wait();
        self.finished = true;
    }
}

impl Drop for Armed {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Some(fd) = take_fd() {
            unsafe {
                libc::close(fd);
            }
        }
        let _ = self.child.wait();
    }
}

pub fn tripped() -> bool {
    INTERRUPT.load(Ordering::SeqCst)
}

pub fn detach_helper_session() {
    unsafe {
        detach_in_child();
    }
}

/// Spawn the fail-closed helper. `sim` selects the signal simulation, which
/// uses the dry-run action `none` and never calls a lock path.
pub fn arm_helper(action: &str, dir: &Path, sim: bool) -> Option<Armed> {
    let nonce = prepare_helper_slot(dir)?;
    let exe = std::env::current_exe().ok()?;
    let dir_text = dir.display().to_string();
    let mut command = Command::new(exe);
    command
        .args(["--helper", action, "--output-dir", &dir_text])
        .env(HELPER_ENV, "1")
        .env(HELPER_NONCE_ENV, &nonce)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if sim {
        command
            .env("MACOS_LOCK_KIT_SIGNAL_SIM", "helper")
            .env("MACOS_LOCK_KIT_SIGNAL_DIR", &dir_text);
    }
    unsafe {
        command.pre_exec(|| {
            detach_in_child();
            Ok(())
        });
    }
    let mut child = command.spawn().ok()?;
    let stdin = child.stdin.take()?;
    install(stdin.into_raw_fd());
    Some(Armed {
        child,
        nonce,
        finished: false,
    })
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

unsafe fn detach_in_child() {
    unsafe {
        libc::setsid();
    }
    ignore(libc::SIGINT);
    ignore(libc::SIGHUP);
    ignore(libc::SIGTERM);
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
        "parent" => sim_parent(false),
        "complete" => sim_parent(true),
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
    let action = helper_action_arg();
    let nonce = std::env::var(HELPER_NONCE_ENV).unwrap_or_default();
    let body = format!("outcome={outcome}\naction={action}\nsid={sid}\nnonce={nonce}\n");
    let _ = write_helper_result(std::path::Path::new(&dir), &body);
    0
}

fn helper_action_arg() -> String {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--helper" {
            return args.next().unwrap_or_default();
        }
    }
    String::new()
}

fn sim_parent(complete: bool) -> i32 {
    let dir = std::env::var("MACOS_LOCK_KIT_SIGNAL_DIR").unwrap_or_default();
    let path = std::path::Path::new(&dir);
    let Some(mut armed) = arm_helper("none", path, true) else {
        let _ = std::fs::write(path.join("parent-error.txt"), "helper did not start");
        return 2;
    };
    let sid = unsafe { libc::getsid(0) };
    let pgid = unsafe { libc::getpgrp() };
    let ready = format!("pid={}\nsid={sid}\npgid={pgid}\n", std::process::id());
    let _ = std::fs::write(path.join("ready"), ready);
    if complete {
        armed.stand_down();
        return 0;
    }
    for _ in 0..400 {
        if tripped() {
            armed.finish();
            return 0;
        }
        thread::sleep(Duration::from_millis(25));
    }
    armed.stand_down();
    3
}

#[cfg(test)]
mod tests {
    use super::arm_helper;

    #[test]
    fn a_result_path_that_cannot_be_removed_refuses_to_arm() {
        let dir = std::env::temp_dir().join(format!(
            "macos-lock-kit-blocked-result-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("helper-result.txt")).expect("blocked path");
        assert!(arm_helper("none", &dir, true).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
