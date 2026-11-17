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
}
