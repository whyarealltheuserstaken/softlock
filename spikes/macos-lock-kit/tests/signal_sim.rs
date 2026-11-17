//! A signal to the parent must not kill the helper, and must not unlock.
//!
//! The parent process is the kit's signal simulation. It arms the helper with
//! `arm_helper` and the dry-run action `none`, so this never calls a lock
//! path. The helper writes `locked` when its stdin closes without `complete`.

#![cfg(unix)]

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn each_signal_makes_the_helper_lock_without_locking_the_session() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        let helper = signal_parent("parent", signal_name(signal), |parent, ready| {
            let killed = unsafe { libc::kill(parent.id() as i32, signal) };
            assert_eq!(killed, 0, "kill {signal}");
            let _ = ready;
        });
        assert!(
            helper.contains("outcome=locked\n"),
            "signal {signal} helper did not lock: {helper}"
        );
        assert!(helper.contains("action=none\n"), "{helper}");
        assert!(!helper.contains("stood-down"));
    }
}

#[test]
fn a_group_interrupt_leaves_the_helper_alive_and_locking() {
    let helper = signal_parent("parent", "group", |parent, ready| {
        let _ = parent;
        let pgid: i32 = field(&ready, "pgid").parse().expect("pgid");
        let killed = unsafe { libc::kill(-pgid, libc::SIGINT) };
        assert_eq!(killed, 0, "kill process group {pgid}");
    });
    assert!(helper.contains("outcome=locked\n"), "{helper}");
    assert!(helper.contains("action=none\n"), "{helper}");
}

#[test]
fn a_session_hangup_leaves_the_helper_alive_and_locking() {
    let helper = signal_parent("parent", "session", |parent, ready| {
        let _ = parent;
        let sid: i32 = field(&ready, "sid").parse().expect("sid");
        let killed = unsafe { libc::kill(-sid, libc::SIGHUP) };
        assert_eq!(killed, 0, "kill session {sid}");
    });
    assert!(helper.contains("outcome=locked\n"), "{helper}");
    assert!(helper.contains("action=none\n"), "{helper}");
}

#[test]
fn complete_stands_the_dry_run_helper_down() {
    let dir = std::env::temp_dir().join(format!(
        "macos-lock-kit-signal-complete-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("dir");
    let mut parent = spawn_parent(&dir, "complete");
    let ready = wait_text(&dir.join("ready"), Duration::from_secs(5));
    let status = parent.wait().expect("wait parent");
    assert!(status.success(), "{status}");
    let helper = wait_text(&dir.join("helper-result.txt"), Duration::from_secs(5));
    assert!(helper.contains("outcome=stood-down\n"), "{helper}");
    assert!(helper.contains("action=none\n"), "{helper}");
    assert_ne!(field(&helper, "sid"), field(&ready, "sid"));
    assert_ne!(field(&helper, "sid"), "0");
    let _ = fs::remove_dir_all(&dir);
}

fn signal_parent(role: &str, label: &str, poke: impl FnOnce(&Child, String)) -> String {
    let dir = std::env::temp_dir().join(format!(
        "macos-lock-kit-signal-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("dir");
    let mut parent = spawn_parent(&dir, role);
    let ready = wait_text(&dir.join("ready"), Duration::from_secs(5));
    assert_eq!(field(&ready, "pid"), parent.id().to_string(), "{ready}");
    poke(&parent, ready.clone());
    let status = parent.wait().expect("wait parent");
    assert!(status.success(), "{label} parent status {status}");
    let helper = wait_text(&dir.join("helper-result.txt"), Duration::from_secs(5));
    assert_ne!(
        field(&helper, "sid"),
        field(&ready, "sid"),
        "helper must leave the parent's session: {helper}"
    );
    assert_ne!(field(&helper, "sid"), "0");
    assert_ne!(field(&helper, "sid"), "-1");
    let _ = fs::remove_dir_all(&dir);
    helper
}

fn spawn_parent(dir: &Path, role: &str) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_macos-lock-kit"));
    command
        .env("MACOS_LOCK_KIT_SIGNAL_SIM", role)
        .env("MACOS_LOCK_KIT_SIGNAL_DIR", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn().expect("sim parent")
}

fn signal_name(signal: i32) -> &'static str {
    if signal == libc::SIGINT {
        "int"
    } else if signal == libc::SIGTERM {
        "term"
    } else {
        "hup"
    }
}

fn field(text: &str, key: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .unwrap_or("")
        .trim()
        .to_string()
}

fn wait_text(path: &Path, budget: Duration) -> String {
    let deadline = Instant::now() + budget;
    loop {
        if let Ok(text) = fs::read_to_string(path)
            && !text.is_empty()
        {
            return text;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {}", path.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
