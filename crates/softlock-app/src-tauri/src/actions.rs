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
/// the name constant. `at_ms` is the service clock on the fall-back event.
pub fn fallback_notice(fallback: &softlock_core::Fallback) -> String {
    format!(
        "{} switched to the Windows lock at {} ms because {}",
        softlock_core::PRODUCT_DISPLAY_NAME,
        fallback.at_ms,
        fallback_reason(fallback.reason)
    )
}

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
        let notice = fallback_notice(&Fallback {
            reason: FailReason::HeartbeatMissed,
            at_ms: 1_500,
        });
        let name = softlock_core::PRODUCT_DISPLAY_NAME;
        assert_eq!(
            notice,
            format!(
                "{name} switched to the Windows lock at 1500 ms because the lock host stopped answering"
            )
        );
        assert!(!notice.contains("Unlocked"));
    }
}
