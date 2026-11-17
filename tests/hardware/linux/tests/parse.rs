use linux_hardware_suite::args::Args;
use linux_hardware_suite::detect::{DetectedConfig, RemoteTools};
use linux_hardware_suite::detect::{SessionKind, parse_os_release, parse_session_type};
use linux_hardware_suite::display::{parse_drm_status_map, parse_xrandr};
use linux_hardware_suite::fail_closed::{FORBIDDEN_UNLOCK, FailClosed, on_failure};
use linux_hardware_suite::power::{parse_mem_sleep, parse_power_state};
use linux_hardware_suite::prompt::{Answer, parse_answer};
use linux_hardware_suite::redact::{contains_serial, redact};
use linux_hardware_suite::report::{SCHEMA, SuiteResult, TestRow};

use std::collections::BTreeMap;
use std::time::SystemTime;

#[test]
fn args_ci_admin_output_dir() {
    let a = Args::parse(["--ci", "--admin", "--output-dir", "/tmp/out"]).unwrap();
    assert!(a.ci && a.admin);
    assert_eq!(a.mode_label(), "ci+admin");
    assert_eq!(a.output_dir.unwrap().as_os_str(), "/tmp/out");
    assert!(Args::parse(["--nope"]).is_err());
}

#[test]
fn os_release_and_session() {
    let (id, ver) = parse_os_release("ID='debian'\nVERSION_ID='12'\n");
    assert_eq!((id, ver), ("debian".into(), "12".into()));
    assert_eq!(
        parse_session_type(Some("tty"), None, None),
        SessionKind::None
    );
}

#[test]
fn drm_and_xrandr_and_suspend() {
    let drm = parse_drm_status_map(&[("card1-DP-1".into(), "connected".into())]);
    assert_eq!(drm[0].name, "card1-DP-1");
    assert_eq!(parse_xrandr("Screen 0:\n").len(), 0);
    assert_eq!(parse_power_state("mem"), vec!["mem"]);
    assert_eq!(parse_mem_sleep("[s2idle] deep").current, "s2idle");
}

#[test]
fn prompt_and_fail_closed() {
    assert_eq!(parse_answer("\n").unwrap(), Answer::Go);
    assert_eq!(parse_answer("skip").unwrap(), Answer::Skip);
    assert_eq!(on_failure(true), FailClosed::OsLock);
    assert_eq!(FORBIDDEN_UNLOCK, "unlock-session");
}

#[test]
fn schema_is_shared_v1() {
    assert_eq!(SCHEMA, "softlock-hardware-suite/1");
    let config = DetectedConfig {
        os: "linux".into(),
        distro_id: "ubuntu".into(),
        distro_version: "24.04".into(),
        kernel: "6.8.0-generic".into(),
        arch: "x86_64".into(),
        session_type: "none".into(),
        compositor: "none".into(),
        desktop: "none".into(),
        display_count: 0,
        displays: vec![],
        suspend_states: vec!["mem".into(), "disk".into()],
        mem_sleep: Some("s2idle".into()),
        sudo: "none".into(),
        uid_is_root: false,
        logind: true,
        dbus_system: true,
        evdev_nodes: 0,
        uinput: "present".into(),
        backend_input_guard: "unsupported".into(),
        backend_overlay: "unsupported".into(),
        backend_power: "unsupported".into(),
        remote: RemoteTools {
            xrdp: false,
            vnc: false,
            gnome_remote_desktop: false,
            rustdesk: false,
        },
    };
    let tests = vec![TestRow {
        id: "detect.config".into(),
        name: "cfg".into(),
        status: "skip".into(),
        skip_reason: Some("ci".into()),
        physical: false,
        needs_admin: false,
        duration_ms: 1,
        detail: String::new(),
        measurements: BTreeMap::new(),
    }];
    let result = SuiteResult::finish(
        "0".into(),
        SystemTime::UNIX_EPOCH,
        "ci".into(),
        config,
        tests,
        true,
        false,
        "never-unlocked".into(),
    );
    let json = result.to_json();
    assert!(json.contains("\"status\": \"skip\""));
    assert!(json.contains("never-unlocked"));
    assert!(!contains_serial(&json));
    let txt = result.to_txt();
    assert!(txt.contains("pass=0 fail=0 skip=1"));
    assert!(txt.lines().count() < 20);
    assert_eq!(redact("ok"), "ok");
}
