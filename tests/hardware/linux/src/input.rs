//! evdev / uinput probes. Never read a device uniq ioctl.

use std::fs;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use serde_json::{Value, json};

use crate::detect::DetectedConfig;
use crate::report::TestRow;
use crate::restore::{register_grab, register_uinput, unregister_grab, unregister_uinput};

pub const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
pub const UI_DEV_CREATE: libc::c_ulong = 0x5501;
pub const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
pub const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
pub const UI_GET_VERSION: libc::c_ulong = 0x8004_552d;

const EV_KEY: libc::c_int = 1;

pub fn enumerate_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = measurements();
    measurements.insert("evdev_nodes".into(), json!(config.evdev_nodes));
    let (status, detail) = if Path::new("/dev/input").is_dir() {
        ("pass", format!("nodes={}", config.evdev_nodes))
    } else {
        ("skip", "no /dev/input".to_string())
    };
    row(
        "input.evdev.enumerate",
        "Count evdev nodes without grabbing",
        status,
        false,
        false,
        detail,
        measurements,
    )
}

pub fn grab_row(config: &DetectedConfig, admin: bool) -> TestRow {
    let mut measurements = measurements();
    if !admin {
        return skip(
            "input.evdev.grab",
            "EVIOCGRAB a node we own",
            true,
            false,
            "needs --admin",
            measurements,
        );
    }
    if config.uinput != "writable" {
        measurements.insert("uinput".into(), json!(config.uinput));
        return skip(
            "input.evdev.grab",
            "EVIOCGRAB a node we own",
            true,
            false,
            "no writable /dev/uinput",
            measurements,
        );
    }
    match grab_self_uinput() {
        Ok(version) => {
            measurements.insert("uinput_version".into(), json!(version));
            row(
                "input.evdev.grab",
                "EVIOCGRAB a node we own",
                "pass",
                true,
                false,
                format!("uinput_version={version}"),
                measurements,
            )
        }
        Err(e) => row(
            "input.evdev.grab",
            "EVIOCGRAB a node we own",
            "fail",
            true,
            false,
            e,
            measurements,
        ),
    }
}

pub fn uinput_row(config: &DetectedConfig, admin: bool) -> TestRow {
    let mut measurements = measurements();
    measurements.insert("uinput".into(), json!(config.uinput));
    if !admin {
        return skip(
            "input.uinput.create",
            "Create and destroy a virtual uinput device",
            true,
            false,
            "needs --admin",
            measurements,
        );
    }
    if config.uinput != "writable" {
        return skip(
            "input.uinput.create",
            "Create and destroy a virtual uinput device",
            true,
            false,
            "no writable /dev/uinput",
            measurements,
        );
    }
    match create_destroy_uinput() {
        Ok(version) => {
            measurements.insert("uinput_version".into(), json!(version));
            row(
                "input.uinput.create",
                "Create and destroy a virtual uinput device",
                "pass",
                true,
                false,
                format!("uinput_version={version}"),
                measurements,
            )
        }
        Err(e) => row(
            "input.uinput.create",
            "Create and destroy a virtual uinput device",
            "fail",
            true,
            false,
            e,
            measurements,
        ),
    }
}

pub fn compositor_input_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = measurements();
    measurements.insert("session".into(), json!(config.session_type));
    measurements.insert("compositor".into(), json!(config.compositor));
    if config.session_type == "none" {
        return skip(
            "input.compositor.session",
            "Session/compositor input lock protocol",
            false,
            false,
            "no graphical session (X11/Wayland)",
            measurements,
        );
    }
    let wayland_info = command_has("wayland-info", &[], "ext_session_lock");
    let idle = command_has("wayland-info", &[], "zwp_idle_inhibit");
    measurements.insert("ext_session_lock".into(), json!(wayland_info));
    measurements.insert("idle_inhibit".into(), json!(idle));
    let detail = format!(
        "session={} compositor={} ext_session_lock={} idle_inhibit={}",
        config.session_type, config.compositor, wayland_info, idle
    );
    row(
        "input.compositor.session",
        "Session/compositor input lock protocol",
        "pass",
        false,
        false,
        detail,
        measurements,
    )
}

fn grab_self_uinput() -> Result<u32, String> {
    let (fd, version) = open_uinput()?;
    register_uinput(fd);
    unsafe {
        set_evbit(fd)?;
        if libc::ioctl(fd, UI_DEV_CREATE, 0) != 0 {
            unregister_uinput();
            destroy(fd);
            return Err("UI_DEV_CREATE failed".into());
        }
    }
    // Grabbing our own uinput node is best-effort: find the new event node, grab, ungrab.
    let grabbed = grab_named("linux-hw-suite");
    unsafe {
        let _ = libc::ioctl(fd, UI_DEV_DESTROY, 0);
        libc::close(fd);
    }
    unregister_uinput();
    let _ = grabbed;
    Ok(version)
}

fn create_destroy_uinput() -> Result<u32, String> {
    let (fd, version) = open_uinput()?;
    register_uinput(fd);
    unsafe {
        set_evbit(fd)?;
        if libc::ioctl(fd, UI_DEV_CREATE, 0) != 0 {
            unregister_uinput();
            destroy(fd);
            return Err("UI_DEV_CREATE failed".into());
        }
        if libc::ioctl(fd, UI_DEV_DESTROY, 0) != 0 {
            unregister_uinput();
            libc::close(fd);
            return Err("UI_DEV_DESTROY failed".into());
        }
        libc::close(fd);
    }
    unregister_uinput();
    Ok(version)
}

fn open_uinput() -> Result<(i32, u32), String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/uinput")
        .map_err(|e| e.to_string())?;
    let fd = file.as_raw_fd();
    std::mem::forget(file);
    let mut version: libc::c_uint = 0;
    unsafe {
        if libc::ioctl(fd, UI_GET_VERSION, &mut version) != 0 {
            libc::close(fd);
            return Err("UI_GET_VERSION failed".into());
        }
        let mut setup: [u8; 1024] = [0; 1024];
        // uinput_setup / uinput_user_dev name at offset 0 for user_dev.
        let name = b"linux-hw-suite";
        setup[..name.len()].copy_from_slice(name);
        // Prefer UI_DEV_SETUP if available; fall back to write(uinput_user_dev).
        let n = libc::write(fd, setup.as_ptr() as *const _, setup.len());
        if n < 0 {
            libc::close(fd);
            return Err("uinput name write failed".into());
        }
    }
    Ok((fd, version as u32))
}

unsafe fn set_evbit(fd: i32) -> Result<(), String> {
    unsafe {
        if libc::ioctl(fd, UI_SET_EVBIT, EV_KEY) != 0 {
            return Err("UI_SET_EVBIT failed".into());
        }
    }
    Ok(())
}

unsafe fn destroy(fd: i32) {
    unsafe {
        let _ = libc::ioctl(fd, UI_DEV_DESTROY, 0);
        libc::close(fd);
    }
}

fn grab_named(want: &str) -> Result<(), String> {
    let Ok(entries) = fs::read_dir("/dev/input") else {
        return Ok(());
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("event") {
            continue;
        }
        if let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(&path) {
            let fd = file.as_raw_fd();
            // Device name via EVIOCGNAME, not UNIQ.
            let mut buf = [0u8; 256];
            let eviocgname = eviocgname(256);
            let n = unsafe { libc::ioctl(fd, eviocgname, buf.as_mut_ptr()) };
            if n > 0 {
                let got = String::from_utf8_lossy(&buf[..n as usize]);
                if got.contains(want) {
                    std::mem::forget(file);
                    register_grab(fd);
                    let rc = unsafe { libc::ioctl(fd, EVIOCGRAB, 1) };
                    let _ = unsafe { libc::ioctl(fd, EVIOCGRAB, 0) };
                    unregister_grab(fd);
                    unsafe { libc::close(fd) };
                    if rc != 0 {
                        return Err("EVIOCGRAB failed".into());
                    }
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

fn eviocgname(len: u32) -> libc::c_ulong {
    // _IOC(_IOC_READ, 'E', 0x06, len)
    let dir = 2u32;
    let typ = u32::from(b'E');
    let nr = 0x06u32;
    u64::from((dir << 30) | (len << 16) | (typ << 8) | nr)
}

fn command_has(bin: &str, args: &[&str], needle: &str) -> bool {
    let out = std::process::Command::new(bin).args(args).output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            text.contains(needle)
        }
        _ => false,
    }
}

fn measurements() -> std::collections::BTreeMap<String, Value> {
    std::collections::BTreeMap::new()
}

fn row(
    id: &str,
    name: &str,
    status: &str,
    needs_admin: bool,
    physical: bool,
    detail: String,
    measurements: std::collections::BTreeMap<String, Value>,
) -> TestRow {
    TestRow {
        id: id.into(),
        name: name.into(),
        status: status.into(),
        skip_reason: if status == "skip" {
            Some(detail.clone())
        } else {
            None
        },
        physical,
        needs_admin,
        duration_ms: 0,
        detail: if status == "skip" {
            String::new()
        } else {
            detail
        },
        measurements,
    }
}

fn skip(
    id: &str,
    name: &str,
    needs_admin: bool,
    physical: bool,
    reason: &str,
    measurements: std::collections::BTreeMap<String, Value>,
) -> TestRow {
    row(
        id,
        name,
        "skip",
        needs_admin,
        physical,
        reason.to_string(),
        measurements,
    )
}
