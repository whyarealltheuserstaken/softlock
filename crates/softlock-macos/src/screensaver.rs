//! Whether leaving the screen saver can require the account password immediately.
//!
//! The numbers are the `askForPassword` and `askForPasswordDelay` values read
//! from the screen-saver preferences. This module does not write them. A
//! missing value, a delay other than zero, or a disagreement between the user
//! domain and the current-host domain cannot guarantee a password.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasswordAfterScreensaver {
    pub ask: Option<i64>,
    pub delay_seconds: Option<i64>,
}

impl PasswordAfterScreensaver {
    pub const fn can_require_immediate_password(self) -> bool {
        matches!(self.ask, Some(1)) && matches!(self.delay_seconds, Some(0))
    }
}

/// Prefer the current-host value. If the two domains disagree, the setting
/// is not guaranteed.
pub fn resolve_preference(current_host: Option<i64>, user_domain: Option<i64>) -> Option<i64> {
    match (current_host, user_domain) {
        (Some(host), Some(user)) if host != user => None,
        (Some(host), _) => Some(host),
        (None, user) => user,
    }
}

pub fn password_after_screensaver(
    ask_host: Option<i64>,
    ask_user: Option<i64>,
    delay_host: Option<i64>,
    delay_user: Option<i64>,
) -> PasswordAfterScreensaver {
    PasswordAfterScreensaver {
        ask: resolve_preference(ask_host, ask_user),
        delay_seconds: resolve_preference(delay_host, delay_user),
    }
}

/// Parse one `defaults read` value. Empty and non-numeric text are missing.
pub fn parse_defaults_value(text: &str) -> Option<i64> {
    let token = text.trim();
    if token.is_empty() {
        return None;
    }
    if let Ok(value) = token.parse::<i64>() {
        return Some(value);
    }
    let (whole, fraction) = token.split_once('.')?;
    if fraction.chars().all(|ch| ch == '0') && !fraction.is_empty() {
        return whole.parse().ok();
    }
    None
}

pub fn preference_label(value: Option<i64>) -> String {
    match value {
        Some(number) => number.to_string(),
        None => "missing".to_string(),
    }
}

/// `sysadminctl -screenLock status`, parsed from the text it printed.
///
/// This does not run the tool. A prompt for an admin password, or text with
/// no delay phrase, is [`ScreenLockRead::Unavailable`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenLockRead {
    Immediate,
    Off,
    Delayed(i64),
    Unavailable,
}

impl ScreenLockRead {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::Off => "off",
            Self::Delayed(_) => "delayed",
            Self::Unavailable => "unavailable",
        }
    }

    pub const fn is_immediate(self) -> bool {
        match self {
            Self::Immediate => true,
            Self::Delayed(seconds) => seconds == 0,
            Self::Off | Self::Unavailable => false,
        }
    }

    pub const fn says_not_immediate(self) -> bool {
        match self {
            Self::Off => true,
            Self::Delayed(seconds) => seconds != 0,
            Self::Immediate | Self::Unavailable => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockSetting {
    Immediate,
    NotImmediate,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockSettingReport {
    pub setting: LockSetting,
    pub disagreement: bool,
}

pub fn interpret_screen_lock_status(stdout: &str, stderr: &str) -> ScreenLockRead {
    let text = format!("{stdout}\n{stderr}");
    if asks_for_admin(&text) {
        return ScreenLockRead::Unavailable;
    }
    phrase(&text).unwrap_or(ScreenLockRead::Unavailable)
}

/// One definite reading is enough. Two definite readings that differ are
/// unknown, and unknown refuses to arm.
pub fn combine_lock_setting(
    defaults: PasswordAfterScreensaver,
    screen: ScreenLockRead,
) -> LockSettingReport {
    let defaults_immediate = defaults.can_require_immediate_password();
    let defaults_negative =
        defaults.ask.is_some() && defaults.delay_seconds.is_some() && !defaults_immediate;
    let screen_immediate = screen.is_immediate();
    let screen_negative = screen.says_not_immediate();
    if (defaults_immediate || defaults_negative) && (screen_immediate || screen_negative) {
        if defaults_immediate == screen_immediate {
            return report(defaults_immediate, false);
        }
        return LockSettingReport {
            setting: LockSetting::Unknown,
            disagreement: true,
        };
    }
    if screen_immediate || screen_negative {
        return report(screen_immediate, false);
    }
    if defaults_immediate || defaults_negative {
        return report(defaults_immediate, false);
    }
    LockSettingReport {
        setting: LockSetting::Unknown,
        disagreement: false,
    }
}

fn report(immediate: bool, disagreement: bool) -> LockSettingReport {
    LockSettingReport {
        setting: if immediate {
            LockSetting::Immediate
        } else {
            LockSetting::NotImmediate
        },
        disagreement,
    }
}

fn asks_for_admin(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("password:")
        || lower.contains("authenticate")
        || lower.contains("administrator")
        || lower.contains("not permitted")
        || lower.contains("requires admin")
}

fn phrase(text: &str) -> Option<ScreenLockRead> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("delay is immediate") || lower.contains("screenlock is immediate") {
        return Some(ScreenLockRead::Immediate);
    }
    if lower.contains("delay is off")
        || lower.contains("screenlock is off")
        || lower.contains("screen lock is off")
        || lower.contains("screenlock disabled")
    {
        return Some(ScreenLockRead::Off);
    }
    let marker = "delay is ";
    let rest = lower.split(marker).nth(1)?.trim_start();
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let seconds = digits.parse::<i64>().ok()?;
    Some(if seconds == 0 {
        ScreenLockRead::Immediate
    } else {
        ScreenLockRead::Delayed(seconds)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_password_requires_the_ask_flag_and_a_zero_delay() {
        let ready = password_after_screensaver(Some(1), None, Some(0), None);
        assert!(ready.can_require_immediate_password());
        let off = password_after_screensaver(Some(0), None, Some(0), None);
        assert!(!off.can_require_immediate_password());
        let delayed = password_after_screensaver(Some(1), None, Some(5), None);
        assert!(!delayed.can_require_immediate_password());
        let missing = password_after_screensaver(None, None, None, None);
        assert!(!missing.can_require_immediate_password());
        assert_eq!(preference_label(None), "missing");
    }

    #[test]
    fn a_disagreement_between_domains_is_not_guaranteed() {
        assert_eq!(resolve_preference(Some(1), Some(0)), None);
        assert_eq!(resolve_preference(Some(0), Some(0)), Some(0));
        assert_eq!(resolve_preference(None, Some(1)), Some(1));
        let mixed = password_after_screensaver(Some(1), Some(0), Some(0), Some(0));
        assert!(!mixed.can_require_immediate_password());
        let delay = password_after_screensaver(Some(1), Some(1), Some(0), Some(5));
        assert!(!delay.can_require_immediate_password());
    }

    #[test]
    fn defaults_text_parses_integers_and_a_zero_fraction() {
        assert_eq!(parse_defaults_value("1\n"), Some(1));
        assert_eq!(parse_defaults_value(" 0 "), Some(0));
        assert_eq!(parse_defaults_value("0.0"), Some(0));
        assert_eq!(parse_defaults_value("5.0"), Some(5));
        assert_eq!(parse_defaults_value("1.5"), None);
        assert_eq!(parse_defaults_value(""), None);
        assert_eq!(parse_defaults_value("does not exist"), None);
    }

    #[test]
    fn screen_lock_status_parses_immediate_delay_and_an_admin_prompt() {
        assert_eq!(
            interpret_screen_lock_status("screenLock delay is immediate\n", ""),
            ScreenLockRead::Immediate
        );
        assert_eq!(
            interpret_screen_lock_status("", "screenLock delay is 5 seconds\n"),
            ScreenLockRead::Delayed(5)
        );
        assert_eq!(
            interpret_screen_lock_status("screenLock delay is off\n", ""),
            ScreenLockRead::Off
        );
        assert_eq!(
            interpret_screen_lock_status("", "Password:"),
            ScreenLockRead::Unavailable
        );
        assert_eq!(
            interpret_screen_lock_status("", "Operation not permitted\n"),
            ScreenLockRead::Unavailable
        );
        assert_eq!(
            interpret_screen_lock_status("", ""),
            ScreenLockRead::Unavailable
        );
    }

    #[test]
    fn a_disagreement_or_two_missing_readings_refuse_to_arm() {
        let defaults_on = password_after_screensaver(Some(1), None, Some(0), None);
        let defaults_off = password_after_screensaver(Some(0), None, Some(0), None);
        let defaults_missing = password_after_screensaver(None, None, None, None);
        assert_eq!(
            combine_lock_setting(defaults_on, ScreenLockRead::Immediate).setting,
            LockSetting::Immediate
        );
        assert!(!combine_lock_setting(defaults_on, ScreenLockRead::Immediate).disagreement);
        let clash = combine_lock_setting(defaults_on, ScreenLockRead::Off);
        assert_eq!(clash.setting, LockSetting::Unknown);
        assert!(clash.disagreement);
        assert_eq!(
            combine_lock_setting(defaults_missing, ScreenLockRead::Immediate).setting,
            LockSetting::Immediate
        );
        assert_eq!(
            combine_lock_setting(defaults_missing, ScreenLockRead::Unavailable).setting,
            LockSetting::Unknown
        );
        assert_eq!(
            combine_lock_setting(defaults_off, ScreenLockRead::Unavailable).setting,
            LockSetting::NotImmediate
        );
        assert_eq!(
            combine_lock_setting(defaults_on, ScreenLockRead::Unavailable).setting,
            LockSetting::Immediate
        );
    }
}
