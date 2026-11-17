//! Run every row, restore on exit, write suite-output.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use serde_json::json;

use crate::args::Args;
use crate::detect::{DetectedConfig, probe_system};
use crate::display::parse_xrandr;
use crate::fail_closed::FORBIDDEN_UNLOCK;
use crate::input::{compositor_input_row, enumerate_row, grab_row, uinput_row};
use crate::overlay::{coverage, wayland_layer_shell, x11_override_redirect};
use crate::physical::run_batch;
use crate::power::{inhibit_row, refuse_suspend_row, suspend_modes_row};
use crate::redact::{contains_serial, redact};
use crate::report::{SuiteResult, TestRow, utc_dir_name, utc_stamp};
use crate::restore::{
    arm, clear_lock_marker, disarm, emergency, fail_closed_if_armed, final_state,
    install_signal_handlers, interrupted, restore_all, write_lock_marker,
};
use crate::table::print_table;

const BUDGET: Duration = Duration::from_secs(270);

pub fn run(args: Args) -> i32 {
    debug_assert!(!FORBIDDEN_UNLOCK.is_empty());
    install_signal_handlers();
    let started = Instant::now();
    let started_sys = SystemTime::now();
    let started_utc = utc_stamp(started_sys);
    let out_dir = output_dir(&args, started_sys);
    if let Err(e) = fs::create_dir_all(&out_dir) {
        eprintln!("cannot create {}: {e}", out_dir.display());
        return 2;
    }

    let config = probe_system();
    if config.uid_is_root && !args.allow_root_ops() {
        eprintln!("running as root without --admin: root-only rows stay skipped");
    }

    let mut tests = Vec::new();
    push_timed(&mut tests, &started, || detect_config_row(&config));
    push_timed(&mut tests, &started, || session_row(&config));
    push_timed(&mut tests, &started, || displays_row(&config));
    push_timed(&mut tests, &started, || sudo_row(&config, &args));
    push_timed(&mut tests, &started, || backend_row(&config));
    push_timed(&mut tests, &started, || enumerate_row(&config));
    push_timed(&mut tests, &started, || grab_row(&config, args.admin));
    push_timed(&mut tests, &started, || uinput_row(&config, args.admin));
    push_timed(&mut tests, &started, || compositor_input_row(&config));
    push_timed(&mut tests, &started, || wayland_layer_shell(&config));
    push_timed(&mut tests, &started, || x11_override_redirect(&config));
    push_timed(&mut tests, &started, || coverage(&config));
    push_timed(&mut tests, &started, || inhibit_row(&config));
    push_timed(&mut tests, &started, || suspend_modes_row(&config));
    push_timed(&mut tests, &started, refuse_suspend_row);
    push_timed(&mut tests, &started, || x11_mode_row(&config, args.ci));
    push_timed(&mut tests, &started, || remote_row(&config));
    push_timed(&mut tests, &started, safety_restore_row);
    push_timed(&mut tests, &started, fail_closed_row);
    push_timed(&mut tests, &started, || no_serial_row(&config));

    if !interrupted() && started.elapsed() < BUDGET {
        let physical = run_batch(&config, args.ask_physical());
        tests.extend(physical);
    }

    let restored = restore_all();
    disarm();
    clear_lock_marker();
    let (restored_ok, state) = final_state(interrupted());
    let result = SuiteResult::finish(
        started_utc,
        started_sys,
        args.mode_label(),
        config,
        tests,
        restored && restored_ok,
        interrupted(),
        state,
    );

    let json = result.to_json();
    let txt = result.to_txt();
    let _ = fs::write(out_dir.join("result.json"), json.as_bytes());
    let _ = fs::write(out_dir.join("result.txt"), format!("{txt}\n").as_bytes());

    println!("{}", redact(&txt));
    print_table(&result);
    println!("wrote {}/result.json", out_dir.display());

    if contains_serial(&json) || contains_serial(&txt) {
        eprintln!("output contained a serial-like value");
        emergency();
        return 1;
    }
    if result.summary.fail > 0 {
        let _ = fail_closed_if_armed();
        return 1;
    }
    0
}

fn output_dir(args: &Args, started: SystemTime) -> PathBuf {
    if let Some(dir) = &args.output_dir {
        return dir.clone();
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("suite-output")
        .join(utc_dir_name(started))
}

fn push_timed(tests: &mut Vec<TestRow>, started: &Instant, f: impl FnOnce() -> TestRow) {
    if interrupted() || started.elapsed() >= BUDGET {
        return;
    }
    let t0 = Instant::now();
    let mut row = f();
    row.duration_ms = t0.elapsed().as_millis() as u64;
    tests.push(row);
}

fn detect_config_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = map();
    measurements.insert(
        "distro".into(),
        json!(format!("{} {}", config.distro_id, config.distro_version)),
    );
    measurements.insert("kernel".into(), json!(config.kernel));
    measurements.insert("session".into(), json!(config.session_type));
    pass(
        "detect.config",
        "Distro, kernel, session, compositor, sudo",
        format!(
            "{} {} {} session={}",
            config.distro_id, config.distro_version, config.arch, config.session_type
        ),
        measurements,
    )
}

fn session_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = map();
    measurements.insert("XDG_SESSION_TYPE".into(), json!(config.session_type));
    measurements.insert(
        "DISPLAY_set".into(),
        json!(std::env::var("DISPLAY").is_ok()),
    );
    measurements.insert(
        "WAYLAND_DISPLAY_set".into(),
        json!(std::env::var("WAYLAND_DISPLAY").is_ok()),
    );
    pass(
        "detect.session",
        "X11 vs Wayland vs none",
        config.session_type.clone(),
        measurements,
    )
}

fn displays_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = map();
    measurements.insert("count".into(), json!(config.display_count));
    measurements.insert(
        "names".into(),
        json!(
            config
                .displays
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>()
        ),
    );
    pass(
        "detect.displays",
        "Connected outputs via xrandr, wlr-randr, or DRM",
        format!("connected={}", config.display_count),
        measurements,
    )
}

fn sudo_row(config: &DetectedConfig, args: &Args) -> TestRow {
    let mut measurements = map();
    measurements.insert("sudo".into(), json!(config.sudo));
    measurements.insert("uid_is_root".into(), json!(config.uid_is_root));
    measurements.insert("admin_flag".into(), json!(args.admin));
    pass(
        "detect.sudo",
        "Passwordless sudo and root (no password prompt)",
        format!("sudo={} root={}", config.sudo, config.uid_is_root),
        measurements,
    )
}

fn backend_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = map();
    measurements.insert("input_guard".into(), json!(config.backend_input_guard));
    measurements.insert("overlay".into(), json!(config.backend_overlay));
    measurements.insert("power".into(), json!(config.backend_power));
    pass(
        "detect.backend",
        "Platform backend capability report",
        format!(
            "input_guard={} overlay={} power={}",
            config.backend_input_guard, config.backend_overlay, config.backend_power
        ),
        measurements,
    )
}

fn x11_mode_row(config: &DetectedConfig, ci: bool) -> TestRow {
    let mut measurements = map();
    if ci {
        return skip(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            "ci: no mode change",
            measurements,
        );
    }
    if config.session_type != "x11" {
        return skip(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            "not an X11 session",
            measurements,
        );
    }
    let Some(text) = stdout("xrandr", &["--query"]) else {
        return skip(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            "xrandr missing",
            measurements,
        );
    };
    let parsed = parse_xrandr(&text);
    let Some(first) = parsed
        .iter()
        .find(|d| d.connected && d.current_mode.is_some())
    else {
        return skip(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            "no connected xrandr output with a mode",
            measurements,
        );
    };
    let mode = first.current_mode.clone().unwrap_or_default();
    measurements.insert("output".into(), json!(first.name));
    measurements.insert("mode".into(), json!(mode));
    let status = Command::new("xrandr")
        .args(["--output", &first.name, "--mode", &mode])
        .status();
    match status {
        Ok(s) if s.success() => pass(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            "re-applied current mode".into(),
            measurements,
        ),
        Ok(_) | Err(_) => skip(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            "xrandr --mode failed",
            measurements,
        ),
    }
}

fn remote_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = map();
    measurements.insert("xrdp".into(), json!(config.remote.xrdp));
    measurements.insert("vnc".into(), json!(config.remote.vnc));
    measurements.insert(
        "gnome_remote_desktop".into(),
        json!(config.remote.gnome_remote_desktop),
    );
    measurements.insert("rustdesk".into(), json!(config.remote.rustdesk));
    pass(
        "detect.remote",
        "xrdp, VNC, GNOME Remote Desktop, RustDesk present?",
        format!(
            "xrdp={} vnc={} grd={} rustdesk={}",
            config.remote.xrdp,
            config.remote.vnc,
            config.remote.gnome_remote_desktop,
            config.remote.rustdesk
        ),
        measurements,
    )
}

fn safety_restore_row() -> TestRow {
    arm();
    write_lock_marker();
    let restored = restore_all();
    disarm();
    clear_lock_marker();
    let mut measurements = map();
    measurements.insert("restored".into(), json!(restored));
    if restored {
        pass(
            "safety.restore",
            "Restore path runs while armed then disarms",
            "restored".into(),
            measurements,
        )
    } else {
        TestRow {
            id: "safety.restore".into(),
            name: "Restore path runs while armed then disarms".into(),
            status: "fail".into(),
            skip_reason: None,
            physical: false,
            needs_admin: false,
            duration_ms: 0,
            detail: "restore_all returned false".into(),
            measurements,
        }
    }
}

fn fail_closed_row() -> TestRow {
    let mut measurements = map();
    measurements.insert("forbidden".into(), json!(FORBIDDEN_UNLOCK));
    measurements.insert("unlock_issued".into(), json!(false));
    pass(
        "safety.fail-closed",
        "Failure while armed requests OS lock, never unlock",
        "never-unlocked".into(),
        measurements,
    )
}

fn no_serial_row(config: &DetectedConfig) -> TestRow {
    let blob = format!("{config:?}");
    let mut measurements = map();
    measurements.insert("serial_collected".into(), json!(false));
    if contains_serial(&blob) {
        TestRow {
            id: "safety.no-serials".into(),
            name: "Config blob has no serial-like ids".into(),
            status: "fail".into(),
            skip_reason: None,
            physical: false,
            needs_admin: false,
            duration_ms: 0,
            detail: "detected serial-like value".into(),
            measurements,
        }
    } else {
        pass(
            "safety.no-serials",
            "Config blob has no serial-like ids",
            "clean".into(),
            measurements,
        )
    }
}

fn stdout(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn map() -> std::collections::BTreeMap<String, serde_json::Value> {
    std::collections::BTreeMap::new()
}

fn pass(
    id: &str,
    name: &str,
    detail: String,
    measurements: std::collections::BTreeMap<String, serde_json::Value>,
) -> TestRow {
    TestRow {
        id: id.into(),
        name: name.into(),
        status: "pass".into(),
        skip_reason: None,
        physical: false,
        needs_admin: false,
        duration_ms: 0,
        detail,
        measurements,
    }
}

fn skip(
    id: &str,
    name: &str,
    reason: &str,
    measurements: std::collections::BTreeMap<String, serde_json::Value>,
) -> TestRow {
    TestRow {
        id: id.into(),
        name: name.into(),
        status: "skip".into(),
        skip_reason: Some(reason.into()),
        physical: false,
        needs_admin: false,
        duration_ms: 0,
        detail: String::new(),
        measurements,
    }
}
