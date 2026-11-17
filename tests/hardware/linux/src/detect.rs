//! Distro, kernel, session, compositor, sudo. No hardware ids.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::display::{DisplayInfo, parse_drm_dir, parse_wlr_randr, parse_xrandr};
use crate::power::{MemSleep, parse_mem_sleep, parse_power_state};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RemoteTools {
    pub xrdp: bool,
    pub vnc: bool,
    pub gnome_remote_desktop: bool,
    pub rustdesk: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DetectedConfig {
    pub os: String,
    pub distro_id: String,
    pub distro_version: String,
    pub kernel: String,
    pub arch: String,
    pub session_type: String,
    pub compositor: String,
    pub desktop: String,
    pub display_count: u32,
    pub displays: Vec<DisplayInfo>,
    pub suspend_states: Vec<String>,
    pub mem_sleep: Option<String>,
    pub sudo: String,
    pub uid_is_root: bool,
    pub logind: bool,
    pub dbus_system: bool,
    pub evdev_nodes: u32,
    pub uinput: String,
    pub backend_input_guard: String,
    pub backend_overlay: String,
    pub backend_power: String,
    pub remote: RemoteTools,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionKind {
    Wayland,
    X11,
    None,
    Unknown,
}

impl SessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wayland => "wayland",
            Self::X11 => "x11",
            Self::None => "none",
            Self::Unknown => "unknown",
        }
    }
}

pub fn parse_os_release(text: &str) -> (String, String) {
    let mut id = String::new();
    let mut version = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("ID=") {
            id = unquote(rest);
        } else if let Some(rest) = line.strip_prefix("VERSION_ID=") {
            version = unquote(rest);
        }
    }
    if id.is_empty() {
        id = "unknown".to_string();
    }
    if version.is_empty() {
        version = "unknown".to_string();
    }
    (id, version)
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

pub fn parse_session_type(
    xdg_session: Option<&str>,
    display: Option<&str>,
    wayland: Option<&str>,
) -> SessionKind {
    match xdg_session.map(str::trim).unwrap_or("") {
        "wayland" => return SessionKind::Wayland,
        "x11" => return SessionKind::X11,
        "tty" | "unspecified" | "mir" => {}
        "" => {}
        _ => {
            if wayland.filter(|v| !v.is_empty()).is_some() {
                return SessionKind::Wayland;
            }
            if display.filter(|v| !v.is_empty()).is_some() {
                return SessionKind::X11;
            }
            return SessionKind::Unknown;
        }
    }
    let wayland = wayland.filter(|v| !v.is_empty());
    let display = display.filter(|v| !v.is_empty());
    match (wayland, display) {
        (Some(_), _) => SessionKind::Wayland,
        (None, Some(_)) => SessionKind::X11,
        (None, None) => SessionKind::None,
    }
}

pub fn parse_compositor(
    session: SessionKind,
    xdg_desktop: Option<&str>,
    wayland_desktop: Option<&str>,
    xdg_current: Option<&str>,
) -> String {
    let joined = [
        xdg_current.unwrap_or(""),
        xdg_desktop.unwrap_or(""),
        wayland_desktop.unwrap_or(""),
    ]
    .join(":")
    .to_ascii_lowercase();
    let token = |name: &str| {
        joined.split([':', '-', '_']).any(|part| part == name) || joined.contains(name)
    };
    if session == SessionKind::X11 {
        if token("kwin") {
            return "kwin-x11".to_string();
        }
        if token("gnome") || token("ubuntu") {
            return "mutter-x11".to_string();
        }
        if token("xfce") {
            return "xfwm4".to_string();
        }
        return "xorg".to_string();
    }
    if session != SessionKind::Wayland {
        return "none".to_string();
    }
    if token("sway") {
        return "sway".to_string();
    }
    if token("hyprland") {
        return "hyprland".to_string();
    }
    if token("kwin") || token("kde") || token("plasma") {
        return "kwin".to_string();
    }
    if token("gnome") || token("mutter") || token("ubuntu") {
        return "mutter".to_string();
    }
    if token("wlroots") || token("wayfire") || token("river") {
        return "wlroots".to_string();
    }
    "wayland".to_string()
}

pub fn parse_sudo_status(code: Option<i32>, ran: bool) -> String {
    if !ran {
        return "none".to_string();
    }
    match code {
        Some(0) => "passwordless".to_string(),
        Some(1) | Some(127) => "needs-password-or-missing".to_string(),
        Some(_) => "unavailable".to_string(),
        None => "unavailable".to_string(),
    }
}

pub fn probe_system() -> DetectedConfig {
    let os_release = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let (distro_id, distro_version) = parse_os_release(&os_release);
    let kernel = uname_field("-r");
    let arch = uname_field("-m");
    let session = parse_session_type(
        env_opt("XDG_SESSION_TYPE").as_deref(),
        env_opt("DISPLAY").as_deref(),
        env_opt("WAYLAND_DISPLAY").as_deref(),
    );
    let compositor = parse_compositor(
        session,
        env_opt("XDG_SESSION_DESKTOP").as_deref(),
        env_opt("WAYLAND_DISPLAY").as_deref(),
        env_opt("XDG_CURRENT_DESKTOP").as_deref(),
    );
    let desktop = env_opt("XDG_CURRENT_DESKTOP").unwrap_or_else(|| "none".to_string());
    let displays = collect_displays(session);
    let connected = displays.iter().filter(|d| d.connected).count() as u32;
    let suspend_states = fs::read_to_string("/sys/power/state")
        .map(|t| parse_power_state(&t))
        .unwrap_or_default();
    let mem_sleep = fs::read_to_string("/sys/power/mem_sleep")
        .ok()
        .map(|t| parse_mem_sleep(&t))
        .map(|m: MemSleep| m.current);
    let sudo = parse_sudo_status(sudo_n_code(), true);
    let uid_is_root = unsafe { libc::geteuid() == 0 };
    let logind = command_ok("busctl", &["status", "org.freedesktop.login1"]);
    let dbus_system = Path::new("/run/dbus/system_bus_socket").exists()
        || Path::new("/var/run/dbus/system_bus_socket").exists();
    let evdev_nodes = count_evdev();
    let uinput = uinput_state();
    let report = softlock_linux::capability_report();
    DetectedConfig {
        os: "linux".to_string(),
        distro_id,
        distro_version,
        kernel,
        arch,
        session_type: session.as_str().to_string(),
        compositor,
        desktop,
        display_count: connected,
        displays,
        suspend_states,
        mem_sleep,
        sudo,
        uid_is_root,
        logind,
        dbus_system,
        evdev_nodes,
        uinput,
        backend_input_guard: format!("{:?}", report.input_guard).to_ascii_lowercase(),
        backend_overlay: format!("{:?}", report.overlay).to_ascii_lowercase(),
        backend_power: format!("{:?}", report.power).to_ascii_lowercase(),
        remote: probe_remote(),
    }
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn uname_field(flag: &str) -> String {
    let out = Command::new("uname").arg(flag).output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => "unknown".to_string(),
    }
}

fn sudo_n_code() -> Option<i32> {
    let mut cmd = Command::new("sudo");
    cmd.args(["-n", "true"]);
    match cmd.output() {
        Ok(o) => o.status.code(),
        Err(_) => None,
    }
}

fn command_ok(bin: &str, args: &[&str]) -> bool {
    Command::new(bin)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn which_exists(name: &str) -> bool {
    env_opt("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|p| !p.is_empty())
        .any(|dir| Path::new(dir).join(name).is_file())
}

fn probe_remote() -> RemoteTools {
    RemoteTools {
        xrdp: which_exists("xrdp") || Path::new("/usr/sbin/xrdp").exists(),
        vnc: ["Xvnc", "x0vncserver", "vncserver", "tigervncserver"]
            .into_iter()
            .any(which_exists),
        gnome_remote_desktop: which_exists("grdctl")
            || Path::new("/usr/libexec/gnome-remote-desktop-daemon").exists(),
        rustdesk: which_exists("rustdesk"),
    }
}

fn count_evdev() -> u32 {
    let Ok(entries) = fs::read_dir("/dev/input") else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
        .count() as u32
}

fn uinput_state() -> String {
    let path = Path::new("/dev/uinput");
    if !path.exists() {
        return "missing".to_string();
    }
    match fs::OpenOptions::new().read(true).write(true).open(path) {
        Ok(_) => "writable".to_string(),
        Err(_) => "present".to_string(),
    }
}

fn collect_displays(session: SessionKind) -> Vec<DisplayInfo> {
    if session == SessionKind::X11
        && let Some(text) = command_stdout("xrandr", &["--query"])
    {
        let parsed = parse_xrandr(&text);
        if !parsed.is_empty() {
            return parsed;
        }
    }
    if session == SessionKind::Wayland
        && let Some(text) = command_stdout("wlr-randr", &[])
    {
        let parsed = parse_wlr_randr(&text);
        if !parsed.is_empty() {
            return parsed;
        }
    }
    parse_drm_dir("/sys/class/drm")
}

fn command_stdout(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        SessionKind, parse_compositor, parse_os_release, parse_session_type, parse_sudo_status,
    };

    #[test]
    fn os_release_quotes() {
        let text = "NAME=\"Ubuntu\"\nID=ubuntu\nVERSION_ID=\"24.04\"\n";
        assert_eq!(
            parse_os_release(text),
            ("ubuntu".to_string(), "24.04".to_string())
        );
    }

    #[test]
    fn session_prefers_xdg() {
        assert_eq!(
            parse_session_type(Some("wayland"), Some(":0"), Some("wayland-0")),
            SessionKind::Wayland
        );
        assert_eq!(
            parse_session_type(Some("x11"), Some(":0"), None),
            SessionKind::X11
        );
        assert_eq!(
            parse_session_type(Some("tty"), None, None),
            SessionKind::None
        );
        assert_eq!(
            parse_session_type(None, None, Some("wayland-1")),
            SessionKind::Wayland
        );
        assert_eq!(parse_session_type(None, Some(":1"), None), SessionKind::X11);
    }

    #[test]
    fn compositor_tokens() {
        assert_eq!(
            parse_compositor(SessionKind::Wayland, None, None, Some("ubuntu:GNOME")),
            "mutter"
        );
        assert_eq!(
            parse_compositor(SessionKind::Wayland, Some("sway"), None, None),
            "sway"
        );
        assert_eq!(
            parse_compositor(SessionKind::X11, None, None, Some("XFCE")),
            "xfwm4"
        );
        assert_eq!(
            parse_compositor(SessionKind::None, None, None, None),
            "none"
        );
    }

    #[test]
    fn sudo_codes() {
        assert_eq!(parse_sudo_status(Some(0), true), "passwordless");
        assert_eq!(
            parse_sudo_status(Some(1), true),
            "needs-password-or-missing"
        );
        assert_eq!(parse_sudo_status(None, false), "none");
    }
}
