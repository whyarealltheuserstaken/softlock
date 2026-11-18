//! Registry values this suite may change, and how a previous dword comes back.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restore {
    Write(u32),
    Delete,
}

/// `None` means the value was absent, so restore deletes it.
pub fn restore_dword(previous: Option<u32>) -> Restore {
    restore_saved(previous.is_some(), previous.unwrap_or(0))
}

/// `present` is false when this suite created the value. The stored number is
/// then a placeholder, including 0, and must not be written back.
pub fn restore_saved(present: bool, data: u32) -> Restore {
    if present {
        Restore::Write(data)
    } else {
        Restore::Delete
    }
}

pub fn hkcu_value_allowed(name: &str) -> bool {
    matches!(
        name,
        "DisableTaskMgr"
            | "NoLogoff"
            | "DisableChangePassword"
            | "DisableLockWorkstation"
            | "NoClose"
            | "ToastEnabled"
    )
}

/// The only machine-hive value the suite writes, and only with `--admin`.
pub fn hklm_value_allowed(name: &str) -> bool {
    name == "HideFastUserSwitching"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_puts_back_a_dword_or_deletes_one_we_created() {
        assert_eq!(restore_dword(Some(1)), Restore::Write(1));
        assert_eq!(restore_dword(Some(0)), Restore::Write(0));
        assert_eq!(restore_dword(None), Restore::Delete);
    }

    #[test]
    fn an_absent_disable_task_mgr_is_deleted_not_written_as_zero() {
        assert_eq!(restore_saved(false, 0), Restore::Delete);
        assert_eq!(restore_saved(true, 0), Restore::Write(0));
    }

    #[test]
    fn hkcu_list_is_the_cad_values_plus_toasts() {
        assert!(hkcu_value_allowed("DisableTaskMgr"));
        assert!(hkcu_value_allowed("NoLogoff"));
        assert!(hkcu_value_allowed("DisableChangePassword"));
        assert!(hkcu_value_allowed("DisableLockWorkstation"));
        assert!(hkcu_value_allowed("NoClose"));
        assert!(hkcu_value_allowed("ToastEnabled"));
        assert!(!hkcu_value_allowed("HideFastUserSwitching"));
        assert!(!hklm_value_allowed("DisableTaskMgr"));
        assert!(hklm_value_allowed("HideFastUserSwitching"));
    }
}
