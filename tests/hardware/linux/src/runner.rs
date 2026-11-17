//! Run every row, restore on exit, write suite-output.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use serde_json::json;

use crate::args::Args;
use crate::detect::{DetectedConfig, probe_system};
use crate::display::parse_xrandr;
use crate::exec;
use crate::input::{compositor_input_row, enumerate_row, grab_row, uinput_row};
use crate::overlay::{coverage, wayland_layer_shell, x11_override_redirect};
use crate::physical::run_batch;
use crate::power::{inhibit_row, refuse_suspend_row, suspend_modes_row};
use crate::redact::{contains_serial, redact};
use crate::report::{SuiteResult, TestRow, empty_measurements, utc_dir_name, utc_stamp};
use crate::restore::{
    arm, disarm, fail_closed_if_armed, final_state, install_signal_handlers, interrupted, is_armed,
    prove_fail_closed, prove_xrandr_restore, register_xrandr, restore_all, suite_exit_code,
};
use crate::table::print_table;

const BUDGET: Duration = Duration::from_secs(270);

pub fn run(args: Args) -> i32 {
    install_signal_handlers();
    let started = Instant::now();
    let started_sys = SystemTime::now();
    let started_utc = utc_stamp(started_sys);
    let out_dir = output_dir(&args, started_sys);
    if let Err(err) = fs::create_dir_all(&out_dir) {
        eprintln!("cannot create {}: {err}", out_dir.display());
        return 2;
    }

    let config = probe_system(args.admin);
    let mut tests = Vec::new();
    push(
        &mut tests,
        &started,
        "detect.config",
        "Detect config",
        false,
        false,
        || detect_config_row(&config),
    );
    push(
        &mut tests,
        &started,
        "detect.session",
        "Session",
        false,
        false,
        || session_row(&config),
    );
    push(
        &mut tests,
        &started,
        "detect.displays",
        "Displays",
        false,
        false,
        || displays_row(&config),
    );
    push(
        &mut tests,
        &started,
        "detect.sudo",
        "sudo",
        false,
        false,
        || sudo_row(&config, &args),
    );
    push(
        &mut tests,
        &started,
        "detect.backend",
        "Backend",
        false,
        false,
        || backend_row(&config),
    );
    push(
        &mut tests,
        &started,
        "input.evdev.enumerate",
        "evdev nodes",
        false,
        false,
        || enumerate_row(&config),
    );

    if interrupted() || started.elapsed() >= BUDGET {
        let reason = if interrupted() {
            "interrupted"
        } else {
            "budget"
        };
        tests.push(TestRow::skip(
            "input.evdev.grab",
            "EVIOCGRAB a node we own",
            reason,
            false,
            true,
            reason,
            empty_measurements(),
        ));
        tests.push(TestRow::skip(
            "input.uinput.create",
            "Create and destroy a virtual uinput device",
            reason,
            false,
            true,
            reason,
            empty_measurements(),
        ));
    } else {
        arm();
        let grab = grab_row(&config, args.admin);
        let created = uinput_row(&config, args.admin);
        if grab.status != "fail" && created.status != "fail" {
            disarm();
        }
        tests.push(grab);
        tests.push(created);
    }

    push(
        &mut tests,
        &started,
        "input.compositor.session",
        "Compositor input",
        false,
        false,
        || compositor_input_row(&config),
    );
    push(
        &mut tests,
        &started,
        "overlay.wayland.layer-shell",
        "Wayland overlay",
        false,
        false,
        || wayland_layer_shell(&config),
    );
    push(
        &mut tests,
        &started,
        "overlay.x11.redirect",
        "X11 overlay",
        false,
        false,
        || x11_override_redirect(&config),
    );
    push(
        &mut tests,
        &started,
        "overlay.coverage",
        "Coverage",
        false,
        false,
        || coverage(&config),
    );
    push(
        &mut tests,
        &started,
        "power.inhibit.logind",
        "Inhibit",
        false,
        false,
        || inhibit_row(&config, args.ci),
    );
    push(
        &mut tests,
        &started,
        "power.suspend.modes",
        "Suspend modes",
        false,
        false,
        || suspend_modes_row(&config),
    );
    push(
        &mut tests,
        &started,
        "power.suspend.not-issued",
        "Suspend not issued",
        false,
        false,
        refuse_suspend_row,
    );
    push(
        &mut tests,
        &started,
        "safety.restore",
        "Restore",
        false,
        false,
        safety_restore_row,
    );
    push(
        &mut tests,
        &started,
        "safety.fail-closed",
        "Fail closed",
        false,
        false,
        fail_closed_row,
    );
    push(
        &mut tests,
        &started,
        "display.x11.mode-restore",
        "xrandr mode",
        false,
        false,
        || x11_mode_row(&config, args.ci),
    );
    push(
        &mut tests,
        &started,
        "detect.remote",
        "Remote tools",
        false,
        false,
        || remote_row(&config),
    );
    push(
        &mut tests,
        &started,
        "safety.no-serials",
        "No serials",
        false,
        false,
        || no_serial_row(&config),
    );

    if interrupted() || started.elapsed() >= BUDGET {
        let reason = if interrupted() {
            "interrupted"
        } else {
            "budget"
        };
        for (id, name) in PHYSICAL {
            tests.push(TestRow::skip(
                id,
                name,
                reason,
                true,
                false,
                reason,
                empty_measurements(),
            ));
        }
    } else {
        tests.extend(run_batch(&config, args.ask_physical()));
    }

    let failed = tests.iter().any(|row| row.status == "fail");
    if failed && !is_armed() {
        arm();
    }
    if failed && is_armed() {
        let _ = fail_closed_if_armed();
    } else {
        disarm();
    }
    let restored = restore_all();
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
    let serial = contains_serial(&json) || contains_serial(&txt);
    if serial {
        eprintln!("output contained a serial-like value; not writing it");
        if !is_armed() {
            arm();
        }
        let _ = fail_closed_if_armed();
        return suite_exit_code(interrupted(), result.summary.fail, true);
    }
    let _ = fs::write(out_dir.join("result.json"), json.as_bytes());
    let _ = fs::write(out_dir.join("result.txt"), format!("{txt}\n").as_bytes());
    println!("{}", redact(&txt));
    print_table(&result);
    println!("wrote {}/result.json", out_dir.display());
    suite_exit_code(interrupted(), result.summary.fail, false)
}

const PHYSICAL: &[(&str, &str)] = &[
    ("physical.lid", "Lid close/open"),
    ("physical.display-hotplug", "Plug/unplug a display"),
    ("physical.power-key", "Power key"),
    ("physical.password-unlock", "Password unlock after OS lock"),
];

fn output_dir(args: &Args, started: SystemTime) -> PathBuf {
    if let Some(dir) = &args.output_dir {
        return dir.clone();
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("suite-output")
        .join(utc_dir_name(started))
}

fn push(
    tests: &mut Vec<TestRow>,
    started: &Instant,
    id: &str,
    name: &str,
    physical: bool,
    needs_admin: bool,
    f: impl FnOnce() -> TestRow,
) {
    if interrupted() {
        tests.push(TestRow::skip(
            id,
            name,
            "interrupted",
            physical,
            needs_admin,
            "interrupted",
            empty_measurements(),
        ));
        return;
    }
    if started.elapsed() >= BUDGET {
        tests.push(TestRow::skip(
            id,
            name,
            "budget",
            physical,
            needs_admin,
            "budget",
            empty_measurements(),
        ));
        return;
    }
    let t0 = Instant::now();
    let mut row = f();
    row.duration_ms = t0.elapsed().as_millis() as u64;
    tests.push(row);
}

fn detect_config_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert(
        "distro".into(),
        json!(format!("{} {}", config.distro_id, config.distro_version)),
    );
    measurements.insert("kernel".into(), json!(config.kernel));
    measurements.insert("session".into(), json!(config.session_type));
    TestRow::pass(
        "detect.config",
        "Distro, kernel, session, compositor",
        false,
        false,
        format!(
            "{} {} {} session={}",
            config.distro_id, config.distro_version, config.arch, config.session_type
        ),
        measurements,
    )
}

fn session_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("XDG_SESSION_TYPE".into(), json!(config.session_type));
    measurements.insert(
        "DISPLAY_set".into(),
        json!(std::env::var("DISPLAY").is_ok()),
    );
    measurements.insert(
        "WAYLAND_DISPLAY_set".into(),
        json!(std::env::var("WAYLAND_DISPLAY").is_ok()),
    );
    TestRow::pass(
        "detect.session",
        "X11 vs Wayland vs none",
        false,
        false,
        config.session_type.clone(),
        measurements,
    )
}

fn displays_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = empty_measurements();
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
    TestRow::pass(
        "detect.displays",
        "Connected outputs via xrandr, wlr-randr, or DRM",
        false,
        false,
        format!("connected={}", config.display_count),
        measurements,
    )
}

fn sudo_row(config: &DetectedConfig, args: &Args) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("sudo".into(), json!(config.sudo));
    measurements.insert("uid_is_root".into(), json!(config.uid_is_root));
    if !args.admin {
        return TestRow::skip(
            "detect.sudo",
            "Passwordless sudo",
            "needs-admin",
            false,
            true,
            "sudo not probed without --admin",
            measurements,
        );
    }
    TestRow::pass(
        "detect.sudo",
        "Passwordless sudo",
        false,
        true,
        format!("sudo={} root={}", config.sudo, config.uid_is_root),
        measurements,
    )
}

fn backend_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("input_guard".into(), json!(config.backend_input_guard));
    measurements.insert("overlay".into(), json!(config.backend_overlay));
    measurements.insert("power".into(), json!(config.backend_power));
    TestRow::pass(
        "detect.backend",
        "Platform backend capability report",
        false,
        false,
        format!(
            "input_guard={} overlay={} power={}",
            config.backend_input_guard, config.backend_overlay, config.backend_power
        ),
        measurements,
    )
}

fn x11_mode_row(config: &DetectedConfig, ci: bool) -> TestRow {
    let measurements = empty_measurements();
    if ci {
        return TestRow::skip(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            "ci",
            false,
            false,
            "ci: no mode change",
            measurements,
        );
    }
    if config.session_type != "x11" {
        return TestRow::skip(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            "not-applicable",
            false,
            false,
            "not an X11 session",
            measurements,
        );
    }
    let queried = exec::run_cmd("xrandr", &["--query"]);
    if queried.code != Some(0) {
        return TestRow::skip(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            "not-applicable",
            false,
            false,
            "xrandr missing",
            measurements,
        );
    }
    let parsed = parse_xrandr(&queried.stdout);
    let Some(first) = parsed
        .iter()
        .find(|d| d.connected && d.current_mode.is_some())
    else {
        return TestRow::skip(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            "not-applicable",
            false,
            false,
            "no connected xrandr output with a mode",
            measurements,
        );
    };
    let mode = first.current_mode.clone().unwrap_or_default();
    let rate = first.current_rate.clone().unwrap_or_else(|| "60".into());
    register_xrandr(first.name.clone(), mode.clone(), rate.clone());
    let status = exec::run_cmd(
        "xrandr",
        &["--output", &first.name, "--mode", &mode, "--rate", &rate],
    );
    let mut measurements = measurements;
    measurements.insert("output".into(), json!(first.name));
    measurements.insert("mode".into(), json!(mode));
    measurements.insert("rate".into(), json!(rate));
    if status.code == Some(0) {
        TestRow::pass(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            false,
            false,
            "registered current mode and rate",
            measurements,
        )
    } else {
        TestRow::fail(
            "display.x11.mode-restore",
            "xrandr mode change then restore",
            false,
            false,
            "xrandr --mode --rate failed",
            measurements,
        )
    }
}

fn remote_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("xrdp".into(), json!(config.remote.xrdp));
    measurements.insert("vnc".into(), json!(config.remote.vnc));
    measurements.insert(
        "gnome_remote_desktop".into(),
        json!(config.remote.gnome_remote_desktop),
    );
    measurements.insert("rustdesk".into(), json!(config.remote.rustdesk));
    TestRow::pass(
        "detect.remote",
        "xrdp, VNC, GNOME Remote Desktop, RustDesk present?",
        false,
        false,
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
    match prove_xrandr_restore() {
        Ok(()) => TestRow::pass(
            "safety.restore",
            "Registered xrandr mode is restored with --rate",
            false,
            false,
            "stub saw xrandr --mode --rate",
            empty_measurements(),
        ),
        Err(err) => TestRow::fail(
            "safety.restore",
            "Registered xrandr mode is restored with --rate",
            false,
            false,
            err,
            empty_measurements(),
        ),
    }
}

fn fail_closed_row() -> TestRow {
    match prove_fail_closed() {
        Ok(()) => TestRow::pass(
            "safety.fail-closed",
            "Armed failure requests lock-session",
            false,
            false,
            "stub saw lock-session",
            empty_measurements(),
        ),
        Err(err) => TestRow::fail(
            "safety.fail-closed",
            "Armed failure requests lock-session",
            false,
            false,
            err,
            empty_measurements(),
        ),
    }
}

fn no_serial_row(config: &DetectedConfig) -> TestRow {
    let blob = format!("{config:?}");
    let mut measurements = empty_measurements();
    measurements.insert("serial_collected".into(), json!(false));
    if contains_serial(&blob) {
        TestRow::fail(
            "safety.no-serials",
            "Config blob has no serial-like ids",
            false,
            false,
            "detected serial-like value",
            measurements,
        )
    } else {
        TestRow::pass(
            "safety.no-serials",
            "Config blob has no serial-like ids",
            false,
            false,
            "clean",
            measurements,
        )
    }
}
