//! What toast suppression changes, and how the previous value comes back.

/// `ToastEnabled` while banners are suppressed.
pub const DISABLED_TOASTS: u32 = 0;

/// `NoToastApplicationNotification` while banners are suppressed.
pub const POLICY_SUPPRESSED: u32 = 1;

pub const TOAST_ENABLED_SUBKEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\PushNotifications";
pub const TOAST_ENABLED_VALUE: &str = "ToastEnabled";
pub const POLICY_SUBKEY: &str =
    r"Software\Policies\Microsoft\Windows\CurrentVersion\PushNotifications";
pub const POLICY_VALUE: &str = "NoToastApplicationNotification";
pub const SENTINEL_SUBKEY: &str = r"Software\softlock-spike-overlay";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restore {
    Write(u32),
    Delete,
}

/// `None` means the value was absent, so restore deletes it.
pub fn restore_of(previous: Option<u32>) -> Restore {
    match previous {
        Some(value) => Restore::Write(value),
        None => Restore::Delete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_puts_back_the_previous_dword_or_deletes_a_value_we_created() {
        assert_eq!(restore_of(Some(1)), Restore::Write(1));
        assert_eq!(restore_of(Some(0)), Restore::Write(0));
        assert_eq!(restore_of(None), Restore::Delete);
        assert_eq!(DISABLED_TOASTS, 0);
        assert_eq!(POLICY_SUPPRESSED, 1);
    }
}
