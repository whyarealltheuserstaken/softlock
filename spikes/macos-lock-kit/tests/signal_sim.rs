//! A signal to the parent must not kill the helper, and must not unlock.
//!
//! This spawns the kit in a simulation. The helper writes `locked` when its
//! stdin closes without `complete`. It does not call a lock path.

#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn each_signal_makes_the_helper_lock_without_locking_the_session() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        let dir = std::env::temp_dir().join(format!(
            "macos-lock-kit-signal-{}-{}",
            signal,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let mut parent = Command::new(env!("CARGO_BIN_EXE_macos-lock-kit"))
            .env("MACOS_LOCK_KIT_SIGNAL_SIM", "parent")
            .env("MACOS_LOCK_KIT_SIGNAL_DIR", &dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sim parent");
        let ready = wait_text(&dir.join("ready"), Duration::from_secs(5));
        let ready_pid = field(&ready, "pid");
        assert_eq!(ready_pid, parent.id().to_string(), "{ready}");
        let parent_sid = field(&ready, "sid");
        let killed = unsafe { libc::kill(parent.id() as i32, signal) };
        assert_eq!(killed, 0, "kill {signal}");
        let status = parent.wait().expect("wait parent");
        assert!(status.success(), "signal {signal} parent status {status}");
        let helper = wait_text(&dir.join("helper-result.txt"), Duration::from_secs(5));
        assert!(
            helper.contains("outcome=locked\n"),
            "signal {signal} helper did not lock: {helper}"
        );
        assert!(!helper.contains("stood-down"));
        let helper_sid = field(&helper, "sid");
        assert_ne!(
            helper_sid, parent_sid,
            "helper must leave the parent's session"
        );
        assert_ne!(helper_sid, "0");
        assert_ne!(helper_sid, "-1");
        let _ = fs::remove_dir_all(&dir);
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
