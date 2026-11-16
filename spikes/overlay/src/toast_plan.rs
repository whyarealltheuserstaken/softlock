//! What toast suppression changes, and how the previous value comes back.

/// `ToastEnabled` while banners are suppressed.
pub const DISABLED_TOASTS: u32 = 0;

#[cfg_attr(not(windows), allow(dead_code))]
pub const TOAST_ENABLED_SUBKEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\PushNotifications";
#[cfg_attr(not(windows), allow(dead_code))]
pub const TOAST_ENABLED_VALUE: &str = "ToastEnabled";
#[cfg_attr(not(windows), allow(dead_code))]
pub const POLICY_SUBKEY: &str =
    r"Software\Policies\Microsoft\Windows\CurrentVersion\PushNotifications";
#[cfg_attr(not(windows), allow(dead_code))]
pub const POLICY_VALUE: &str = "NoToastApplicationNotification";
#[cfg_attr(not(windows), allow(dead_code))]
pub const SENTINEL_SUBKEY: &str = r"Software\softlock-spike-overlay";
#[cfg_attr(not(windows), allow(dead_code))]
pub const RUN_ONCE_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
#[cfg_attr(not(windows), allow(dead_code))]
pub const RUN_ONCE_VALUE: &str = "softlock-spike-overlay";

/// A toast policy value already exists, so this program must not change it.
pub fn suppression_blocked(policy_value_count: usize) -> Result<(), &'static str> {
    if policy_value_count == 0 {
        Ok(())
    } else {
        Err("school IT controls this, skip")
    }
}

/// RunOnce command that restores toasts after a kill or a reboot.
/// The path is the copied exe, so this works without cargo.
pub fn run_once_command(exe: &str) -> String {
    format!("\"{exe}\" restore-toasts")
}

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
    }

    #[test]
    fn an_existing_toast_policy_is_left_alone() {
        assert_eq!(suppression_blocked(1), Err("school IT controls this, skip"));
        assert_eq!(suppression_blocked(0), Ok(()));
        assert_eq!(
            run_once_command(r"C:\Users\you\softlock-test\spike-overlay.exe"),
            "\"C:\\Users\\you\\softlock-test\\spike-overlay.exe\" restore-toasts"
        );
    }
}
