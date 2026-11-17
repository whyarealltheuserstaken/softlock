//! Ctrl-C must unblock a prompt without waiting for Enter.

use std::os::unix::io::FromRawFd;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use linux_hardware_suite::prompt::{PromptRead, read_line_interruptible};
use linux_hardware_suite::restore::{install_signal_handlers, interrupted};

const STOP: i32 = 7;

fn child_wait_on_pty() -> ! {
    install_signal_handlers();
    let decision = read_line_interruptible(libc::STDIN_FILENO, interrupted);
    let code = match decision {
        PromptRead::Stop => STOP,
        PromptRead::Skip(reason) if reason == "no-input" => 8,
        PromptRead::Go => 9,
        PromptRead::Skip(_) => 10,
    };
    std::process::exit(code);
}

fn open_pty() -> (i32, i32) {
    let mut master = 0;
    let mut slave = 0;
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty: {}", std::io::Error::last_os_error());
    (master, slave)
}

fn parent_sends_sigint() {
    let (master, slave) = open_pty();
    let exe = std::env::current_exe().expect("test exe");
    let slave_file = unsafe { std::fs::File::from_raw_fd(slave) };
    let mut child = Command::new(exe)
        .arg("--child")
        .stdin(Stdio::from(slave_file))
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn child");
    let started = Instant::now();
    thread::sleep(Duration::from_millis(250));
    let rc = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
    assert_eq!(rc, 0, "kill: {}", std::io::Error::last_os_error());
    let status = child.wait().expect("wait child");
    unsafe {
        libc::close(master);
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "Ctrl-C waited for Enter: {elapsed:?} status={status:?}"
    );
    assert_eq!(
        status.code(),
        Some(STOP),
        "child should stop on SIGINT, got {status:?}"
    );
}

fn main() {
    if std::env::args().any(|arg| arg == "--child") {
        child_wait_on_pty();
    }
    parent_sends_sigint();
    println!("ok");
}
