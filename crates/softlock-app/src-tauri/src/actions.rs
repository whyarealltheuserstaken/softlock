//! What the settings app is allowed to ask the service, and what it may show.
//!
//! Quit is local. It has no public-pipe request, so it cannot stop the service.
//! A failed read never counts as Unlocked.

use std::path::Path;

use softlock_core::{LockSource, MIN_PIN_LEN, RequestKind, StateName};

/// The default global hotkey. Ctrl, Alt, and Shift, plus L.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Hotkey {
    #[cfg_attr(not(test), allow(dead_code))]
    pub control: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub alt: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub shift: bool,
    pub virtual_key: u16,
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn default_hotkey() -> Hotkey {
    Hotkey {
        control: true,
        alt: true,
        shift: true,
        virtual_key: u16::from(b'L'),
    }
}

/// `None` means the action stays in the app and is not a pipe request.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn lock_request(source: LockSource) -> RequestKind {
    RequestKind::Lock { source, note: None }
}

/// Tray "Lock for real". The service asks the Lock Host for the workstation
/// lock and falls back to disconnecting the console session.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn hard_lock_request() -> RequestKind {
    RequestKind::HardLock
}

/// Sentence shown after the owner signs back in. The product name comes from
/// the name constant. The time is the fall-back's UTC instant in local time.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn fallback_notice(fallback: &softlock_core::Fallback) -> String {
    fallback_notice_at(fallback, local_offset_secs())
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn fallback_notice_at(fallback: &softlock_core::Fallback, offset_secs: i32) -> String {
    format!(
        "{} switched to the Windows lock at {} because {}",
        softlock_core::PRODUCT_DISPLAY_NAME,
        format_local_clock(fallback.at_utc_ms, offset_secs),
        fallback_reason(fallback.reason)
    )
}

/// `YYYY-MM-DD HH:MM:SS` for `utc_ms` shifted by `offset_secs`.
pub fn format_local_clock(utc_ms: i64, offset_secs: i32) -> String {
    let adjusted = utc_ms.saturating_add(i64::from(offset_secs).saturating_mul(1000));
    if adjusted < 0 {
        return "unknown time".to_string();
    }
    let secs = (adjusted / 1000) as u64;
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month as u32, day)
}

fn local_offset_secs() -> i32 {
    #[cfg(windows)]
    {
        unsafe {
            let mut info = windows::Win32::System::Time::TIME_ZONE_INFORMATION::default();
            let kind = windows::Win32::System::Time::GetTimeZoneInformation(&mut info);
            // TIME_ZONE_ID_DAYLIGHT is 2. The windows crate does not name it.
            let bias = if kind == 2 {
                info.Bias + info.DaylightBias
            } else {
                info.Bias + info.StandardBias
            };
            -bias * 60
        }
    }
    #[cfg(not(windows))]
    {
        0
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn fallback_reason(reason: softlock_core::FailReason) -> &'static str {
    use softlock_core::FailReason;
    match reason {
        FailReason::ArmStep { .. } => "an arming step failed",
        FailReason::HeartbeatMissed => "the lock host stopped answering",
        FailReason::HostExited => "the lock host exited",
        FailReason::GuardUnhealthy => "the input guard failed",
        FailReason::OverlayUnhealthy => "the lock screen failed",
        FailReason::ArmTimeout => "arming took too long",
        FailReason::RevertFailed => "settings could not be restored",
        FailReason::ServiceStop => "the service was asked to stop",
        FailReason::HardLock => "lock for real",
        FailReason::SessionActive => "a lock was still in progress",
        FailReason::Repair => "settings still need repair",
    }
}

#[cfg(test)]
pub fn quit_request() -> Option<RequestKind> {
    None
}

/// Field error code for the settings PIN. The screen maps this to its string.
pub fn pin_field_error(pin: &str) -> Option<&'static str> {
    if pin.chars().count() < MIN_PIN_LEN {
        Some("too-short")
    } else {
        None
    }
}

/// Unlocked is shown only when the service answered and named that state.
pub fn shows_unlocked(service_running: bool, state: Option<StateName>) -> bool {
    service_running && matches!(state, Some(StateName::Unlocked))
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn autostart_value_name() -> &'static str {
    softlock_core::PRODUCT_DISPLAY_NAME
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn autostart_command(exe: &Path) -> String {
    let text = exe.display().to_string().replace('"', "\\\"");
    format!("\"{text}\"")
}

/// `None` when the registry write succeeded. Any other code is surfaced.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn autostart_failure_notice(code: u32) -> Option<&'static str> {
    if code == 0 {
        None
    } else {
        Some("autostart failed")
    }
}

/// Tray and window copy. The product name comes from the name constant.
pub fn status_line(service_running: bool, state: Option<StateName>) -> String {
    let name = softlock_core::PRODUCT_DISPLAY_NAME;
    if shows_unlocked(service_running, state) {
        format!("{name} is off")
    } else {
        format!("{name} is on")
    }
}

pub fn service_line(service_running: bool) -> &'static str {
    if service_running {
        "Service running"
    } else {
        "Service not running"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hotkey_is_ctrl_alt_shift_l() {
        let hotkey = default_hotkey();
        assert!(hotkey.control);
        assert!(hotkey.alt);
        assert!(hotkey.shift);
        assert_eq!(hotkey.virtual_key, u16::from(b'L'));
    }

    #[test]
    fn quit_does_not_build_a_service_request() {
        assert!(quit_request().is_none());
        let lock = lock_request(LockSource::Tray);
        assert!(matches!(
            lock,
            RequestKind::Lock {
                source: LockSource::Tray,
                ..
            }
        ));
        let lock = lock_request(LockSource::App);
        assert!(matches!(
            lock,
            RequestKind::Lock {
                source: LockSource::App,
                ..
            }
        ));
        let lock = lock_request(LockSource::Hotkey);
        assert!(matches!(
            lock,
            RequestKind::Lock {
                source: LockSource::Hotkey,
                ..
            }
        ));
    }

    #[test]
    fn a_too_short_pin_is_a_field_error() {
        assert_eq!(pin_field_error(""), Some("too-short"));
        assert_eq!(pin_field_error("abc"), Some("too-short"));
        assert!(pin_field_error("1357").is_none());
        assert_eq!(MIN_PIN_LEN, 4);
    }

    #[test]
    fn a_transport_error_does_not_end_unlocked() {
        assert!(!shows_unlocked(false, Some(StateName::Unlocked)));
        assert!(!shows_unlocked(false, None));
        assert!(!shows_unlocked(true, Some(StateName::Locked)));
        assert!(!shows_unlocked(true, None));
        assert!(shows_unlocked(true, Some(StateName::Unlocked)));
        assert_eq!(
            status_line(false, Some(StateName::Unlocked)),
            format!("{} is on", softlock_core::PRODUCT_DISPLAY_NAME)
        );
        assert_eq!(service_line(false), "Service not running");
    }

    #[test]
    fn sign_in_run_value_uses_the_product_name() {
        assert_eq!(autostart_value_name(), softlock_core::PRODUCT_DISPLAY_NAME);
        let command = autostart_command(Path::new(r"C:\Program Files\app.exe"));
        assert_eq!(command, "\"C:\\Program Files\\app.exe\"");
        assert!(!command.contains("stop"));
        assert_eq!(autostart_failure_notice(0), None);
        assert_eq!(autostart_failure_notice(5), Some("autostart failed"));
        assert!(matches!(hard_lock_request(), RequestKind::HardLock));
    }

    #[test]
    fn a_fake_fall_back_event_names_the_time_and_the_reason() {
        use softlock_core::{FailReason, Fallback};
        let fallback = Fallback {
            reason: FailReason::HeartbeatMissed,
            at_ms: 1_500,
            at_utc_ms: 1_795_176_000_000,
        };
        let notice = fallback_notice_at(&fallback, 0);
        let name = softlock_core::PRODUCT_DISPLAY_NAME;
        assert_eq!(
            notice,
            format!(
                "{name} switched to the Windows lock at 2026-11-20 12:00:00 because the lock host stopped answering"
            )
        );
        assert!(!notice.contains("1500 ms"));
        let local = fallback_notice_at(&fallback, 2 * 60 * 60);
        assert!(local.contains("2026-11-20 14:00:00"));
        assert!(!notice.contains("Unlocked"));
    }
}
