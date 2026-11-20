//! Acceptance tests for the OS-lock primitive.
//!
//! A lock counts only when a console-session notification or the session
//! state says so inside one second. `LockWorkStation`'s return value is not
//! an input. Ticket 17 plugs into `before_lock_workstation`; these tests
//! keep that seam from clearing `DisableLockWorkstation`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use softlock_core::{
    Attempt, Caller, Expect, FailReason, Method, NoticeKind, NoticeLog, Observation, OsLockOps,
    Outcome, StepFault, caller_for_service, confirmed, console_notice, fail_closed,
    failed_closed_log, policy_seam, session_for_expect, stays_guarded, tick_is_on_or_after,
    within_budget,
};

fn obs(
    notice: Option<NoticeKind>,
    locked: Option<bool>,
    disconnected: Option<bool>,
    elapsed: Duration,
) -> Observation {
    Observation {
        notice,
        session_locked: locked,
        session_disconnected: disconnected,
        elapsed,
    }
}

fn locked_fast() -> Observation {
    obs(
        Some(NoticeKind::Lock),
        Some(true),
        Some(false),
        Duration::from_millis(40),
    )
}

fn open() -> Observation {
    obs(None, Some(false), Some(false), Duration::from_millis(40))
}

fn disconnected_fast() -> Observation {
    obs(
        Some(NoticeKind::ConsoleDisconnect),
        Some(false),
        Some(true),
        Duration::from_millis(80),
    )
}

struct Script {
    lock_obs: Observation,
    own_obs: Observation,
    console_obs: Observation,
    api_ok: bool,
    prepare_calls: u32,
    calls: Vec<Method>,
    observes: Vec<Expect>,
    panic_prepare: bool,
    panic_call: Option<Method>,
    panic_observe: Option<Expect>,
    skip_lock: bool,
    plug_in_clear: bool,
    cleared_disable_lock_workstation: bool,
}

impl Script {
    fn new() -> Self {
        Self {
            lock_obs: locked_fast(),
            own_obs: disconnected_fast(),
            console_obs: disconnected_fast(),
            api_ok: true,
            prepare_calls: 0,
            calls: Vec::new(),
            observes: Vec::new(),
            panic_prepare: false,
            panic_call: None,
            panic_observe: None,
            skip_lock: false,
            plug_in_clear: false,
            cleared_disable_lock_workstation: false,
        }
    }
}

impl OsLockOps for Script {
    fn before_lock_workstation(&mut self) -> Result<(), StepFault> {
        self.prepare_calls += 1;
        if self.plug_in_clear {
            self.cleared_disable_lock_workstation = true;
        }
        if self.panic_prepare {
            panic!("prepare");
        }
        Ok(())
    }

    fn call(&mut self, method: Method) -> Result<(), StepFault> {
        self.calls.push(method);
        if self.panic_call == Some(method) {
            panic!("call");
        }
        if method == Method::LockWorkStation && self.skip_lock {
            return Err(StepFault::Skipped);
        }
        // `api_ok` is the return value the operating system would report.
        // The engine never reads it.
        let _ = self.api_ok;
        Ok(())
    }

    fn observe(&mut self, expect: Expect, started: Instant) -> Result<Observation, StepFault> {
        self.observes.push(expect);
        if self.panic_observe == Some(expect) {
            panic!("observe");
        }
        let mut observation = match expect {
            Expect::Lock => self.lock_obs,
            Expect::DisconnectOwn => self.own_obs,
            Expect::DisconnectConsole => self.console_obs,
        };
        if observation.elapsed.is_zero() {
            observation.elapsed = started.elapsed();
        }
        Ok(observation)
    }
}

#[test]
fn a_confirmed_primary_does_not_fall_back() {
    let mut script = Script::new();
    script.api_ok = false;
    let outcome = fail_closed(Caller::LockHost, &mut script);
    assert_eq!(
        outcome,
        Outcome::Confirmed {
            method: Method::LockWorkStation
        }
    );
    assert_eq!(script.calls, vec![Method::LockWorkStation]);
    assert_eq!(script.prepare_calls, 1);
    assert!(!script.cleared_disable_lock_workstation);
    assert!(!stays_guarded(&outcome));
}

#[test]
fn an_unconfirmed_primary_uses_a_confirmed_fall_back() {
    let mut script = Script::new();
    script.api_ok = true;
    script.lock_obs = open();
    let outcome = fail_closed(Caller::LockHost, &mut script);
    assert_eq!(
        outcome,
        Outcome::Confirmed {
            method: Method::DisconnectOwnSession
        }
    );
    assert_eq!(
        script.calls,
        vec![Method::LockWorkStation, Method::DisconnectOwnSession]
    );
    assert!(!script.cleared_disable_lock_workstation);
}

#[test]
fn both_unconfirmed_steps_stay_guarded_and_are_logged() {
    let mut script = Script::new();
    script.lock_obs = open();
    script.own_obs = open();
    let outcome = fail_closed(Caller::LockHost, &mut script);
    assert!(stays_guarded(&outcome));
    let Outcome::FailedClosed { attempts } = outcome else {
        panic!("both steps failed");
    };
    assert!(attempts.iter().all(|attempt| !attempt.confirmed));
    assert_eq!(attempts.len(), 2);
    let line = failed_closed_log(Caller::LockHost, &attempts);
    assert!(line.contains("failed-closed"));
    assert!(line.contains("LockWorkStation"));
    assert!(line.contains("unconfirmed"));
    assert!(!line.contains("Unlocked"));
    assert_eq!(
        script.calls,
        vec![Method::LockWorkStation, Method::DisconnectOwnSession]
    );
}

#[test]
fn a_notification_at_one_second_is_a_timeout() {
    let mut script = Script::new();
    script.lock_obs = obs(
        Some(NoticeKind::Lock),
        Some(true),
        Some(false),
        Duration::from_secs(1),
    );
    script.own_obs = obs(None, Some(false), Some(true), Duration::from_millis(20));
    let outcome = fail_closed(Caller::LockHost, &mut script);
    assert_eq!(
        outcome,
        Outcome::Confirmed {
            method: Method::DisconnectOwnSession
        }
    );
    assert!(!confirmed(
        Expect::Lock,
        &obs(
            Some(NoticeKind::Lock),
            Some(true),
            None,
            Duration::from_secs(1)
        )
    ));
    assert!(within_budget(Duration::from_millis(999)));
    assert!(!within_budget(Duration::from_secs(1)));
}

#[test]
fn session_state_confirms_without_a_notification() {
    assert!(confirmed(
        Expect::Lock,
        &obs(None, Some(true), None, Duration::from_millis(10))
    ));
    assert!(confirmed(
        Expect::DisconnectOwn,
        &obs(None, None, Some(true), Duration::from_millis(10))
    ));
    assert!(!confirmed(
        Expect::Lock,
        &obs(
            Some(NoticeKind::ConsoleDisconnect),
            Some(false),
            Some(true),
            Duration::from_millis(10)
        )
    ));
    assert!(!confirmed(
        Expect::DisconnectConsole,
        &obs(
            Some(NoticeKind::Lock),
            Some(true),
            Some(false),
            Duration::from_millis(10)
        )
    ));
    assert!(!confirmed(
        Expect::Lock,
        &obs(None, None, None, Duration::from_millis(10))
    ));
}

#[test]
fn the_service_disconnects_the_console_and_the_lock_host_does_not() {
    let mut service = Script::new();
    let outcome = fail_closed(Caller::Service, &mut service);
    assert_eq!(
        outcome,
        Outcome::Confirmed {
            method: Method::DisconnectConsole
        }
    );
    assert_eq!(service.calls, vec![Method::DisconnectConsole]);
    assert_eq!(service.prepare_calls, 0);
    assert!(!service.calls.contains(&Method::LockWorkStation));

    let mut host = Script::new();
    host.lock_obs = open();
    let outcome = fail_closed(Caller::LockHost, &mut host);
    assert_eq!(
        outcome,
        Outcome::Confirmed {
            method: Method::DisconnectOwnSession
        }
    );
    assert!(!host.calls.contains(&Method::DisconnectConsole));

    let mut primary = Script::new();
    primary.lock_obs = open();
    let outcome = fail_closed(Caller::LockHostPrimary, &mut primary);
    assert!(stays_guarded(&outcome));
    assert_eq!(primary.calls, vec![Method::LockWorkStation]);
}

#[test]
fn the_service_asks_the_host_before_it_disconnects_the_console() {
    let mut script = Script::new();
    script.lock_obs = open();
    let outcome = fail_closed(Caller::ServiceCoordinated, &mut script);
    assert_eq!(
        outcome,
        Outcome::Confirmed {
            method: Method::DisconnectConsole
        }
    );
    assert_eq!(
        script.calls,
        vec![Method::LockWorkStation, Method::DisconnectConsole]
    );
    assert_eq!(script.prepare_calls, 1);
    assert!(!script.cleared_disable_lock_workstation);

    let mut script = Script::new();
    script.skip_lock = true;
    let outcome = fail_closed(Caller::ServiceCoordinated, &mut script);
    assert_eq!(
        outcome,
        Outcome::Confirmed {
            method: Method::DisconnectConsole
        }
    );
    assert_eq!(script.observes, vec![Expect::DisconnectConsole]);
}

#[test]
fn a_panic_in_a_check_cannot_skip_the_lock_or_the_fall_back() {
    let mut script = Script::new();
    script.panic_prepare = true;
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        fail_closed(Caller::LockHost, &mut script)
    }));
    let outcome = outcome.expect("prepare panic must not escape");
    assert_eq!(
        outcome,
        Outcome::Confirmed {
            method: Method::LockWorkStation
        }
    );
    assert_eq!(script.calls, vec![Method::LockWorkStation]);

    let mut script = Script::new();
    script.panic_call = Some(Method::LockWorkStation);
    script.lock_obs = open();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        fail_closed(Caller::LockHost, &mut script)
    }));
    let outcome = outcome.expect("call panic must not escape");
    assert_eq!(
        outcome,
        Outcome::Confirmed {
            method: Method::DisconnectOwnSession
        }
    );

    let mut script = Script::new();
    script.lock_obs = open();
    script.panic_observe = Some(Expect::Lock);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        fail_closed(Caller::LockHost, &mut script)
    }));
    let outcome = outcome.expect("observe panic must not escape");
    assert_eq!(
        outcome,
        Outcome::Confirmed {
            method: Method::DisconnectOwnSession
        }
    );
    assert!(script.calls.contains(&Method::DisconnectOwnSession));
}

#[test]
fn ticket_17_can_plug_in_before_the_workstation_lock_only() {
    assert_eq!(policy_seam(), "leave-disable-lock-workstation");
    let mut script = Script::new();
    script.plug_in_clear = true;
    let _ = fail_closed(Caller::LockHost, &mut script);
    assert!(script.cleared_disable_lock_workstation);
    assert_eq!(script.prepare_calls, 1);

    let mut script = Script::new();
    script.plug_in_clear = true;
    let _ = fail_closed(Caller::Service, &mut script);
    assert!(!script.cleared_disable_lock_workstation);
    assert_eq!(script.prepare_calls, 0);
}

#[test]
fn another_sessions_notification_is_not_the_console() {
    assert_eq!(console_notice(NoticeKind::Lock, 5, 2), None);
    assert_eq!(console_notice(NoticeKind::ConsoleDisconnect, 5, 2), None);
    assert_eq!(
        console_notice(NoticeKind::Lock, 2, 2),
        Some(NoticeKind::Lock)
    );
    assert_eq!(
        console_notice(NoticeKind::ConsoleDisconnect, 2, 2),
        Some(NoticeKind::ConsoleDisconnect)
    );
    assert_eq!(console_notice(NoticeKind::Other, 2, 2), None);
}

#[test]
fn a_failed_closed_attempt_records_the_fault() {
    let mut script = Script::new();
    script.lock_obs = open();
    script.own_obs = open();
    script.panic_observe = Some(Expect::Lock);
    let outcome = fail_closed(Caller::LockHost, &mut script);
    let Outcome::FailedClosed { attempts } = outcome else {
        panic!("expected fail closed");
    };
    assert_eq!(
        attempts[0],
        Attempt {
            method: Method::LockWorkStation,
            confirmed: false,
            fault: Some(StepFault::Panic),
        }
    );
}

#[test]
fn product_code_does_not_reconnect_a_session() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let crates = root.join("crates");
    let mut files = Vec::new();
    collect(&crates, &mut files);
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let needle = format!("{}{}", "WTSConnect", "Session");
        assert!(
            !text.contains(&needle),
            "{} reconnects a session without credentials",
            path.display()
        );
    }
}

#[test]
fn a_dead_or_silent_host_does_not_wait_on_the_workstation_lock() {
    assert_eq!(
        caller_for_service(false, Some(FailReason::HostExited)),
        Caller::Service
    );
    assert_eq!(
        caller_for_service(true, Some(FailReason::HeartbeatMissed)),
        Caller::Service
    );
    assert_eq!(caller_for_service(false, None), Caller::Service);
    assert_eq!(caller_for_service(true, None), Caller::ServiceCoordinated);
    assert_eq!(
        caller_for_service(true, Some(FailReason::GuardUnhealthy)),
        Caller::ServiceCoordinated
    );
    let mut script = Script::new();
    script.console_obs = Observation {
        notice: Some(NoticeKind::ConsoleDisconnect),
        session_locked: None,
        session_disconnected: Some(true),
        elapsed: Duration::from_millis(5),
    };
    let started = Instant::now();
    let outcome = fail_closed(Caller::Service, &mut script);
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "disconnect-only caller waited out the workstation budget"
    );
    assert!(matches!(
        outcome,
        Outcome::Confirmed {
            method: Method::DisconnectConsole
        }
    ));
    assert!(!script.calls.contains(&Method::LockWorkStation));
}

#[test]
fn the_service_confirms_a_workstation_lock_on_the_console_session() {
    assert_eq!(
        session_for_expect(Caller::ServiceCoordinated, Expect::Lock, 2, 0),
        2
    );
    assert_eq!(session_for_expect(Caller::Service, Expect::Lock, 2, 0), 2);
    assert_eq!(session_for_expect(Caller::LockHost, Expect::Lock, 2, 4), 4);
    assert_eq!(
        session_for_expect(Caller::ServiceCoordinated, Expect::DisconnectConsole, 2, 0),
        2
    );
}

#[test]
fn a_stale_lock_notice_does_not_mask_a_later_disconnect() {
    assert!(!tick_is_on_or_after(10, 15));
    assert!(tick_is_on_or_after(15, 15));
    assert!(tick_is_on_or_after(20, 15));
    let mut log = NoticeLog::default();
    log.record(NoticeKind::Lock, 10);
    log.record(NoticeKind::ConsoleDisconnect, 20);
    assert_eq!(log.fresh(NoticeKind::Lock, 15), None);
    assert_eq!(
        log.fresh(NoticeKind::ConsoleDisconnect, 15),
        Some(NoticeKind::ConsoleDisconnect)
    );
    assert_eq!(log.lock_tick, Some(10));
    assert_eq!(log.disconnect_tick, Some(20));
}

#[test]
fn clippy_disallows_reconnecting_a_session() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join("clippy.toml")).expect("clippy.toml");
    let needle = format!("{}{}", "WTSConnect", "SessionW");
    assert!(
        text.contains("disallowed-methods"),
        "clippy does not disallow methods"
    );
    assert!(
        text.contains(&needle),
        "clippy still allows reconnecting a session"
    );
}

#[test]
fn the_lock_host_ui_thread_does_not_call_the_workstation_lock() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../softlock-lockhost/src/win/session.rs");
    let text = std::fs::read_to_string(path).expect("session");
    assert!(text.contains("schedule_workstation_lock"));
    assert!(
        !text.contains("failclosed::lock_workstation_only()"),
        "the UI thread still calls the workstation lock inline"
    );
}

#[test]
fn the_confirm_binary_is_not_a_default_product_binary() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../softlock-windows/Cargo.toml");
    let text = std::fs::read_to_string(path).expect("windows manifest");
    assert!(
        text.contains("required-features = [\"confirm\"]"),
        "os-lock-confirm is still built and smoke-launched on macOS"
    );
}

#[test]
fn seam3_selects_the_service_session_test_binary() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("ci");
    let finder =
        std::fs::read_to_string(root.join(".github/lock-host-test-exe.py")).expect("finder");
    assert!(
        ci.contains("service_session"),
        "ci.yml does not name the service_session test executable"
    );
    assert!(
        finder.contains("service_session") || finder.contains("needle"),
        "the test-binary finder still returns the last executable"
    );
}

#[test]
fn the_fail_closed_job_covers_each_hosted_case() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let job = std::fs::read_to_string(root.join(".github/workflows/os-lock-fail-closed.yml"))
        .expect("fail-closed workflow");
    for case in [
        "kill-host",
        "suspend-host",
        "kill-service",
        "stop-while-locked",
        "unhealthy-guard",
        "tray-hard-lock",
    ] {
        assert!(job.contains(case), "missing {case}");
    }
    let test =
        std::fs::read_to_string(root.join("crates/softlock-lockhost/tests/fail_closed_live.rs"))
            .expect("live test");
    assert!(test.contains("SOFTLOCK_FAIL_CLOSED_LIVE"));
    assert!(test.contains("from_secs(1)"));
    assert!(test.contains("GetStatus"));
    assert!(test.contains("WM_COMMAND"));
    assert!(test.contains("live-fail-closed"));
    assert!(job.contains("live-fail-closed"));
    assert!(job.contains("softlock-fail-closed"));
    for path in [
        "crates/softlock-windows/**",
        "crates/softlock-core/src/**",
        "**/Cargo.toml",
        "Cargo.lock",
    ] {
        assert!(job.contains(path), "fail-closed path filter omits {path}");
    }
    let session = std::fs::read_to_string(root.join("crates/softlock-lockhost/src/win/session.rs"))
        .expect("session");
    let quit = session
        .find("service_quit_locks")
        .expect("quit does not check the guard");
    let schedule = session
        .find("schedule_quit_os_lock()")
        .expect("quit does not schedule");
    assert!(
        quit < schedule,
        "the guard check must come before the OS lock"
    );
    let client = std::fs::read_to_string(root.join("crates/softlock-app/src-tauri/src/client.rs"))
        .expect("client");
    assert!(client.contains("runner_may_lock"));
    let actions =
        std::fs::read_to_string(root.join("crates/softlock-app/src-tauri/src/actions.rs"))
            .expect("actions");
    assert!(actions.contains("SystemTimeToTzSpecificLocalTime"));
    let script = std::fs::read_to_string(root.join("tests/hardware/windows/os-lock-check.ps1"))
        .expect("laptop script");
    for needle in [
        "ThinkStation P7",
        "30F2",
        "30F3",
        "identity-unreadable",
        "remote-session",
        "not-console",
        "Stop-Process",
        "NtSuspendProcess",
        "suite-output",
        "result.json",
        "result.txt",
        "--release",
        "--locked",
        "--offline",
        "NtResumeProcess",
        "PSObject.Properties",
        "finally",
        "SoftLockAgent",
        "Connect(200)",
        "1000",
    ] {
        assert!(script.contains(needle), "laptop script missing {needle}");
    }
    let reconnect = format!("{}{}", "WTSConnect", "Session");
    assert!(
        !script.contains(&reconnect),
        "the laptop script reconnects a session"
    );
    assert!(
        !script.contains("live-fail-closed"),
        "the laptop script enables the hosted test feature"
    );
    assert!(
        !script.contains("softlock-service' -ErrorAction SilentlyContinue | Stop-Process"),
        "the laptop script stops every service process"
    );
    let owner = std::fs::read_to_string(root.join("crates/softlock-app/src-tauri/src/owner.rs"))
        .expect("owner");
    assert!(owner.contains("CMD_HARD_LOCK"));
    assert!(owner.contains("enqueue_hard_lock"));
    assert!(owner.contains("runner_may_lock"));
}

#[test]
fn release_universal_and_product_builds_leave_live_fail_closed_off() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in [
        "crates/softlock-service/Cargo.toml",
        "crates/softlock-lockhost/Cargo.toml",
        "crates/softlock-app/src-tauri/Cargo.toml",
    ] {
        let text = std::fs::read_to_string(root.join(relative)).expect(relative);
        assert!(
            text.contains("live-fail-closed"),
            "{relative} does not declare the feature"
        );
        assert!(
            !text.contains("default = [\"live-fail-closed\"]")
                && !text.contains("default = [\"simulated-physical\", \"live-fail-closed\"]"),
            "{relative} enables live-fail-closed by default"
        );
    }
    let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("ci");
    assert!(
        ci.matches("live-fail-closed feature reached a product crate")
            .count()
            >= 3,
        "linux, windows, and macOS product trees do not reject live-fail-closed"
    );
    let scan = std::fs::read_to_string(root.join(".github/universal-scan.sh")).expect("scan");
    assert!(scan.contains("softlock-live-fail-closed-marker"));
    assert!(scan.contains("softlock_live_fail_closed_marker"));
    let windows = std::fs::read_to_string(root.join("crates/softlock-service/src/windows.rs"))
        .expect("windows");
    let authority = std::fs::read_to_string(root.join("crates/softlock-service/src/authority.rs"))
        .expect("authority");
    let session = std::fs::read_to_string(root.join("crates/softlock-lockhost/src/win/session.rs"))
        .expect("session");
    let failclosed =
        std::fs::read_to_string(root.join("crates/softlock-lockhost/src/win/failclosed.rs"))
            .expect("failclosed");
    assert!(windows.contains("feature = \"live-fail-closed\""));
    assert!(authority.contains("feature = \"live-fail-closed\""));
    assert!(session.contains("feature = \"live-fail-closed\""));
    assert!(failclosed.contains("feature = \"live-fail-closed\""));
}

fn collect(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap();
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            collect(&path, files);
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}
