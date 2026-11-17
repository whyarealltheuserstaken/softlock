//! evdev / uinput probes. Never read a device uniq ioctl.

use std::fs;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::detect::DetectedConfig;
use crate::report::{TestRow, empty_measurements};
use crate::restore::{register_grab, register_uinput, unregister_grab, unregister_uinput};

pub const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
pub const UI_DEV_CREATE: libc::c_ulong = 0x5501;
pub const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
pub const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
pub const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
pub const UI_GET_VERSION: libc::c_ulong = 0x8004_552d;

pub const UINPUT_MAX_NAME_SIZE: usize = 80;
pub const ABS_CNT: usize = 0x40;
const EV_KEY: libc::c_int = 1;
const KEY_A: libc::c_int = 30;
const BUS_VIRTUAL: u16 = 0x06;
const DEVICE_NAME: &[u8] = b"linux-hw-suite";
const GRAB_WAIT: Duration = Duration::from_secs(2);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InputId {
    pub bustype: u16,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UinputSetup {
    pub id: InputId,
    pub name: [u8; UINPUT_MAX_NAME_SIZE],
    pub ff_effects_max: u32,
}

/// `struct uinput_user_dev` from the kernel. A short write is EINVAL.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct UinputUserDev {
    pub name: [u8; UINPUT_MAX_NAME_SIZE],
    pub id: InputId,
    pub ff_effects_max: u32,
    pub absmax: [i32; ABS_CNT],
    pub absmin: [i32; ABS_CNT],
    pub absfuzz: [i32; ABS_CNT],
    pub absflat: [i32; ABS_CNT],
}

pub const fn ioc_write(typ: u8, nr: u32, size: u32) -> u64 {
    (1u64 << 30) | ((size as u64) << 16) | ((typ as u64) << 8) | (nr as u64)
}

pub const UI_DEV_SETUP: libc::c_ulong =
    ioc_write(b'U', 3, std::mem::size_of::<UinputSetup>() as u32) as libc::c_ulong;

pub fn enumerate_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("evdev_nodes".into(), json!(config.evdev_nodes));
    if Path::new("/dev/input").is_dir() {
        TestRow::pass(
            "input.evdev.enumerate",
            "Count evdev nodes without grabbing",
            false,
            false,
            format!("nodes={}", config.evdev_nodes),
            measurements,
        )
    } else {
        TestRow::skip(
            "input.evdev.enumerate",
            "Count evdev nodes without grabbing",
            "not-applicable",
            false,
            false,
            "no /dev/input",
            measurements,
        )
    }
}

pub fn grab_row(config: &DetectedConfig, admin: bool) -> TestRow {
    let mut measurements = empty_measurements();
    if !admin {
        return TestRow::skip(
            "input.evdev.grab",
            "EVIOCGRAB a node we own",
            "needs-admin",
            false,
            true,
            "needs --admin",
            measurements,
        );
    }
    if config.uinput != "writable" {
        measurements.insert("uinput".into(), json!(config.uinput));
        return TestRow::skip(
            "input.evdev.grab",
            "EVIOCGRAB a node we own",
            "not-applicable",
            false,
            true,
            "no writable /dev/uinput",
            measurements,
        );
    }
    match grab_self_uinput() {
        Ok(version) => {
            measurements.insert("uinput_version".into(), json!(version));
            TestRow::pass(
                "input.evdev.grab",
                "EVIOCGRAB a node we own",
                false,
                true,
                format!("uinput_version={version}"),
                measurements,
            )
        }
        Err(err) => TestRow::fail(
            "input.evdev.grab",
            "EVIOCGRAB a node we own",
            false,
            true,
            err,
            measurements,
        ),
    }
}

pub fn uinput_row(config: &DetectedConfig, admin: bool) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("uinput".into(), json!(config.uinput));
    if !admin {
        return TestRow::skip(
            "input.uinput.create",
            "Create and destroy a virtual uinput device",
            "needs-admin",
            false,
            true,
            "needs --admin",
            measurements,
        );
    }
    if config.uinput != "writable" {
        return TestRow::skip(
            "input.uinput.create",
            "Create and destroy a virtual uinput device",
            "not-applicable",
            false,
            true,
            "no writable /dev/uinput",
            measurements,
        );
    }
    match create_destroy_uinput() {
        Ok(version) => {
            measurements.insert("uinput_version".into(), json!(version));
            TestRow::pass(
                "input.uinput.create",
                "Create and destroy a virtual uinput device",
                false,
                true,
                format!("uinput_version={version}"),
                measurements,
            )
        }
        Err(err) => TestRow::fail(
            "input.uinput.create",
            "Create and destroy a virtual uinput device",
            false,
            true,
            err,
            measurements,
        ),
    }
}

pub fn compositor_input_row(config: &DetectedConfig) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("session".into(), json!(config.session_type));
    measurements.insert("compositor".into(), json!(config.compositor));
    if config.session_type == "none" {
        return TestRow::skip(
            "input.compositor.session",
            "Session/compositor input lock protocol",
            "not-applicable",
            false,
            false,
            "no graphical session",
            measurements,
        );
    }
    let wayland_info = command_has("wayland-info", &[], "ext_session_lock");
    let idle = command_has("wayland-info", &[], "zwp_idle_inhibit");
    measurements.insert("ext_session_lock".into(), json!(wayland_info));
    measurements.insert("idle_inhibit".into(), json!(idle));
    TestRow::pass(
        "input.compositor.session",
        "Session/compositor input lock protocol",
        false,
        false,
        format!(
            "session={} compositor={} ext_session_lock={} idle_inhibit={}",
            config.session_type, config.compositor, wayland_info, idle
        ),
        measurements,
    )
}

fn grab_self_uinput() -> Result<u32, String> {
    let (fd, version) = open_uinput()?;
    register_uinput(fd);
    if let Err(err) = setup_device(fd) {
        destroy_and_unregister(fd);
        return Err(err);
    }
    let created = unsafe { libc::ioctl(fd, UI_DEV_CREATE, 0) };
    if created != 0 {
        let err = io_err("UI_DEV_CREATE");
        destroy_and_unregister(fd);
        return Err(err);
    }
    let grabbed = wait_for_grab(DEVICE_NAME, GRAB_WAIT);
    destroy_and_unregister(fd);
    grabbed?;
    Ok(version)
}

fn create_destroy_uinput() -> Result<u32, String> {
    let (fd, version) = open_uinput()?;
    register_uinput(fd);
    if let Err(err) = setup_device(fd) {
        destroy_and_unregister(fd);
        return Err(err);
    }
    let created = unsafe { libc::ioctl(fd, UI_DEV_CREATE, 0) };
    if created != 0 {
        let err = io_err("UI_DEV_CREATE");
        destroy_and_unregister(fd);
        return Err(err);
    }
    let destroyed = unsafe { libc::ioctl(fd, UI_DEV_DESTROY, 0) };
    unsafe { libc::close(fd) };
    unregister_uinput();
    if destroyed != 0 {
        return Err(io_err("UI_DEV_DESTROY"));
    }
    Ok(version)
}

fn open_uinput() -> Result<(i32, u32), String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/uinput")
        .map_err(|err| err.to_string())?;
    let fd = file.as_raw_fd();
    std::mem::forget(file);
    let mut version: libc::c_uint = 0;
    let rc = unsafe { libc::ioctl(fd, UI_GET_VERSION, &mut version) };
    if rc != 0 {
        unsafe { libc::close(fd) };
        return Err(io_err("UI_GET_VERSION"));
    }
    Ok((fd, version as u32))
}

fn setup_device(fd: i32) -> Result<(), String> {
    let ev = unsafe { libc::ioctl(fd, UI_SET_EVBIT, EV_KEY) };
    if ev != 0 {
        return Err(io_err("UI_SET_EVBIT"));
    }
    let key = unsafe { libc::ioctl(fd, UI_SET_KEYBIT, KEY_A) };
    if key != 0 {
        return Err(io_err("UI_SET_KEYBIT"));
    }
    let mut setup = UinputSetup {
        id: InputId {
            bustype: BUS_VIRTUAL,
            vendor: 0,
            product: 0,
            version: 1,
        },
        name: [0; UINPUT_MAX_NAME_SIZE],
        ff_effects_max: 0,
    };
    setup.name[..DEVICE_NAME.len()].copy_from_slice(DEVICE_NAME);
    let rc = unsafe { libc::ioctl(fd, UI_DEV_SETUP, &setup) };
    if rc != 0 {
        return Err(io_err("UI_DEV_SETUP"));
    }
    Ok(())
}

fn destroy_and_unregister(fd: i32) {
    unsafe {
        let _ = libc::ioctl(fd, UI_DEV_DESTROY, 0);
        libc::close(fd);
    }
    unregister_uinput();
}

fn wait_for_grab(want: &[u8], budget: Duration) -> Result<(), String> {
    let deadline = Instant::now() + budget;
    loop {
        match grab_named(want) {
            Grab::Held => return Ok(()),
            Grab::Missing => {
                if Instant::now() >= deadline {
                    return Err("event node did not appear in time".into());
                }
                thread::sleep(Duration::from_millis(50));
            }
            Grab::Failed(err) => return Err(err),
        }
    }
}

enum Grab {
    Held,
    Missing,
    Failed(String),
}

fn grab_named(want: &[u8]) -> Grab {
    let Ok(entries) = fs::read_dir("/dev/input") else {
        return Grab::Missing;
    };
    for entry in entries.filter_map(|item| item.ok()) {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("event") {
            continue;
        }
        let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(&path) else {
            continue;
        };
        let fd = file.as_raw_fd();
        let mut buf = [0u8; 256];
        let n = unsafe { libc::ioctl(fd, eviocgname(256), buf.as_mut_ptr()) };
        if n <= 0 || !buf[..n as usize].windows(want.len()).any(|w| w == want) {
            continue;
        }
        std::mem::forget(file);
        register_grab(fd);
        let rc = unsafe { libc::ioctl(fd, EVIOCGRAB, 1) };
        let _ = unsafe { libc::ioctl(fd, EVIOCGRAB, 0) };
        unregister_grab(fd);
        unsafe { libc::close(fd) };
        if rc != 0 {
            return Grab::Failed(io_err("EVIOCGRAB"));
        }
        return Grab::Held;
    }
    Grab::Missing
}

fn eviocgname(len: u32) -> libc::c_ulong {
    let dir = 2u32;
    let typ = u32::from(b'E');
    let nr = 0x06u32;
    u64::from((dir << 30) | (len << 16) | (typ << 8) | nr) as libc::c_ulong
}

fn io_err(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

fn command_has(bin: &str, args: &[&str], needle: &str) -> bool {
    let out = std::process::Command::new(bin).args(args).output();
    match out {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).contains(needle)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{UI_DEV_SETUP, UinputSetup, UinputUserDev, ioc_write};

    #[test]
    fn uinput_user_dev_matches_the_kernel_size() {
        assert_eq!(std::mem::size_of::<UinputUserDev>(), 1116);
        assert_eq!(std::mem::size_of::<UinputSetup>(), 92);
        assert_eq!(UI_DEV_SETUP, ioc_write(b'U', 3, 92) as libc::c_ulong);
        assert_ne!(std::mem::size_of::<UinputUserDev>(), 1024);
    }
}
