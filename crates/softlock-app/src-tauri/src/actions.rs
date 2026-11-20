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
    notice_for_offset(fallback, local_offset_secs(fallback.at_utc_ms))
}

fn notice_for_offset(fallback: &softlock_core::Fallback, offset: Option<i32>) -> String {
    match offset {
        Some(offset) => fallback_notice_at(fallback, offset),
        None => format!(
            "{} switched to the Windows lock at {} UTC because {}",
            softlock_core::PRODUCT_DISPLAY_NAME,
            format_local_clock(fallback.at_utc_ms, 0),
            fallback_reason(fallback.reason)
        ),
    }
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

/// Offset of the local zone at `utc_ms`, not the offset of the current minute.
/// Windows asks the system to convert that UTC instant, so daylight rules stay
/// in the operating system.
fn local_offset_secs(utc_ms: i64) -> Option<i32> {
    #[cfg(windows)]
    {
        system_local_offset_secs(utc_ms)
    }
    #[cfg(not(windows))]
    {
        let _ = utc_ms;
        None
    }
}

#[cfg(windows)]
fn system_local_offset_secs(utc_ms: i64) -> Option<i32> {
    if utc_ms < 0 {
        return None;
    }
    let secs = utc_ms / 1000;
    let days = secs / 86_400;
    let tod = (secs % 86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    let utc = windows::Win32::Foundation::SYSTEMTIME {
        wYear: year as u16,
        wMonth: month as u16,
        wDay: day as u16,
        wHour: (tod / 3600) as u16,
        wMinute: ((tod % 3600) / 60) as u16,
        wSecond: (tod % 60) as u16,
        ..windows::Win32::Foundation::SYSTEMTIME::default()
    };
    let mut local = windows::Win32::Foundation::SYSTEMTIME::default();
    let converted = unsafe {
        windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime(None, &utc, &mut local)
    };
    if converted.is_err() {
        return None;
    }
    let utc_day = days_from_civil(year, month, day)?;
    let local_day = days_from_civil(
        i32::from(local.wYear),
        u32::from(local.wMonth),
        u32::from(local.wDay),
    )?;
    let utc_secs = utc_day * 86_400 + i64::from(tod);
    let local_secs = local_day * 86_400
        + i64::from(local.wHour) * 3_600
        + i64::from(local.wMinute) * 60
        + i64::from(local.wSecond);
    i32::try_from(local_secs - utc_secs).ok()
}

#[cfg(test)]
struct Transition {
    month: u16,
    week: u16,
    dow: u16,
    hour: u16,
    minute: u16,
}

#[cfg(test)]
struct ZoneRules {
    /// Minutes added to standard local time to get UTC. Windows `Bias + StandardBias`.
    standard_bias_min: i32,
    /// Extra minutes during daylight. Windows `DaylightBias`, usually -60.
    daylight_bias_min: i32,
    daylight_start: Transition,
    standard_start: Transition,
}

#[cfg(test)]
fn offset_at(utc_ms: i64, rules: &ZoneRules) -> i32 {
    let bias = if daylight_at(utc_ms, rules) {
        rules.standard_bias_min + rules.daylight_bias_min
    } else {
        rules.standard_bias_min
    };
    -bias * 60
}

#[cfg(test)]
fn daylight_at(utc_ms: i64, rules: &ZoneRules) -> bool {
    if rules.daylight_start.month == 0
        || rules.standard_start.month == 0
        || rules.daylight_bias_min == 0
    {
        return false;
    }
    let Some(year) = year_of_utc(utc_ms) else {
        return false;
    };
    let Some(start) = transition_utc_ms(year, &rules.daylight_start, rules.standard_bias_min)
    else {
        return false;
    };
    let Some(end) = transition_utc_ms(
        year,
        &rules.standard_start,
        rules.standard_bias_min + rules.daylight_bias_min,
    ) else {
        return false;
    };
    if start < end {
        utc_ms >= start && utc_ms < end
    } else {
        utc_ms >= start || utc_ms < end
    }
}

#[cfg(test)]
fn transition_utc_ms(year: i32, when: &Transition, bias_min: i32) -> Option<i64> {
    let day = nth_weekday(year, when.month, when.week, when.dow)?;
    let local_secs = days_from_civil(year, when.month as u32, day)? * 86_400
        + i64::from(when.hour) * 3_600
        + i64::from(when.minute) * 60;
    Some(local_secs.saturating_add(i64::from(bias_min) * 60) * 1_000)
}

#[cfg(test)]
fn nth_weekday(year: i32, month: u16, week: u16, dow: u16) -> Option<u32> {
    if month == 0 || month > 12 || week == 0 || week > 5 || dow > 6 {
        return None;
    }
    let first = day_of_week(year, month as u32, 1)?;
    let mut day = 1 + (i32::from(dow) + 7 - i32::from(first)).rem_euclid(7);
    day += i32::from(week.saturating_sub(1)) * 7;
    let limit = days_in_month(year, month as u32)?;
    if day > i32::try_from(limit).unwrap_or(i32::MAX) {
        day -= 7;
    }
    u32::try_from(day).ok()
}

#[cfg(test)]
fn days_in_month(year: i32, month: u32) -> Option<u32> {
    let length = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => return None,
    };
    Some(length)
}

#[cfg(test)]
fn is_leap(year: i32) -> bool {
    let year = year as i64;
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
fn day_of_week(year: i32, month: u32, day: u32) -> Option<u16> {
    let days = days_from_civil(year, month, day)?;
    Some(((days.rem_euclid(7) + 4) % 7) as u16)
}

#[cfg(test)]
fn year_of_utc(utc_ms: i64) -> Option<i32> {
    if utc_ms < 0 {
        return None;
    }
    let days = utc_ms / 86_400_000;
    Some(civil_from_days(days).0)
}

#[cfg(any(test, windows))]
fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if month == 0 || month > 12 || day == 0 {
        return None;
    }
    let year = i64::from(year);
    let month = i64::from(month);
    let day = i64::from(day);
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = (if month > 2 { month - 3 } else { month + 9 }) as u64;
    let doy = (153 * mp + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe as i64 - 719_468)
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
        let unknown = notice_for_offset(&fallback, None);
        assert!(unknown.contains("2026-11-20 12:00:00 UTC"));
        assert!(!unknown.contains("12:00:00 because"));
    }

    #[test]
    fn the_offset_follows_the_event_time() {
        let eastern = ZoneRules {
            standard_bias_min: 300,
            daylight_bias_min: -60,
            daylight_start: Transition {
                month: 3,
                week: 2,
                dow: 0,
                hour: 2,
                minute: 0,
            },
            standard_start: Transition {
                month: 11,
                week: 1,
                dow: 0,
                hour: 2,
                minute: 0,
            },
        };
        let january = 1_768_478_400_000i64;
        let july = 1_784_116_800_000i64;
        assert_eq!(civil_from_days(january / 86_400_000), (2026, 1, 15));
        assert_eq!(civil_from_days(july / 86_400_000), (2026, 7, 15));
        assert_eq!(offset_at(january, &eastern), -5 * 3_600);
        assert_eq!(offset_at(july, &eastern), -4 * 3_600);
        assert_ne!(offset_at(january, &eastern), offset_at(july, &eastern));
        let utc = ZoneRules {
            standard_bias_min: 0,
            daylight_bias_min: 0,
            daylight_start: Transition {
                month: 0,
                week: 0,
                dow: 0,
                hour: 0,
                minute: 0,
            },
            standard_start: Transition {
                month: 0,
                week: 0,
                dow: 0,
                hour: 0,
                minute: 0,
            },
        };
        assert_eq!(offset_at(january, &utc), 0);
        assert_eq!(offset_at(july, &utc), 0);
        assert_eq!(nth_weekday(2026, 3, 2, 0), Some(8));
        assert_eq!(nth_weekday(2026, 11, 1, 0), Some(1));
    }
}
