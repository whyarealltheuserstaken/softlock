//! Read-only interpretation of `fdesetup status` and `fdesetup isactive`.
//!
//! The text is whatever those two verbs printed for the console user. This
//! module does not run them and does not enable or disable FileVault. A
//! permission error is not Off.

use softlock_core::DiskEncryption;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileVaultRead {
    On,
    Off,
    PermissionDenied,
    Unreadable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandText {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandText {
    pub fn new(exit_code: i32, stdout: impl Into<String>, stderr: impl Into<String>) -> Self {
        Self {
            exit_code,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    fn combined(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }
}

pub fn interpret_status(output: &CommandText) -> FileVaultRead {
    interpret(output, "FileVault is On.", "FileVault is Off.")
}

pub fn interpret_isactive(output: &CommandText) -> FileVaultRead {
    let stdout = output.stdout.trim();
    if output.exit_code == 0 && stdout.eq_ignore_ascii_case("true") {
        return FileVaultRead::On;
    }
    if output.exit_code == 0 && stdout.eq_ignore_ascii_case("false") {
        return FileVaultRead::Off;
    }
    if permission_denied(&output.combined()) {
        FileVaultRead::PermissionDenied
    } else {
        FileVaultRead::Unreadable
    }
}

/// Prefer a successful `isactive` reading. Fall back to `status`.
///
/// Disagreeing answers become [`DiskEncryption::Unknown`]. A permission error
/// never becomes Off.
pub fn console_filevault(status: FileVaultRead, isactive: FileVaultRead) -> DiskEncryption {
    match (isactive, status) {
        (FileVaultRead::On, FileVaultRead::Off) | (FileVaultRead::Off, FileVaultRead::On) => {
            DiskEncryption::Unknown
        }
        (FileVaultRead::On, _) => DiskEncryption::On,
        (FileVaultRead::Off, _) => DiskEncryption::Off,
        (_, FileVaultRead::On) => DiskEncryption::On,
        (_, FileVaultRead::Off) => DiskEncryption::Off,
        _ => DiskEncryption::Unknown,
    }
}

pub fn disk_label(value: DiskEncryption) -> &'static str {
    match value {
        DiskEncryption::On => "on",
        DiskEncryption::Off => "off",
        DiskEncryption::Unknown => "unknown",
        DiskEncryption::Unsupported => "unsupported",
    }
}

fn interpret(output: &CommandText, on_token: &str, off_token: &str) -> FileVaultRead {
    if output.exit_code == 0
        && output.stdout.contains(on_token)
        && !output.stdout.contains(off_token)
    {
        return FileVaultRead::On;
    }
    if output.exit_code == 0
        && output.stdout.contains(off_token)
        && !output.stdout.contains(on_token)
    {
        return FileVaultRead::Off;
    }
    if permission_denied(&output.combined()) {
        FileVaultRead::PermissionDenied
    } else {
        FileVaultRead::Unreadable
    }
}

fn permission_denied(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("permission")
        || lower.contains("not permitted")
        || lower.contains("must be root")
        || lower.contains("need to be root")
        || lower.contains("operation not permitted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_and_isactive_map_the_documented_tokens() {
        let on = interpret_status(&CommandText::new(0, "FileVault is On.\n", ""));
        let off = interpret_status(&CommandText::new(0, "FileVault is Off.\n", ""));
        assert_eq!(on, FileVaultRead::On);
        assert_eq!(off, FileVaultRead::Off);
        assert_eq!(
            interpret_isactive(&CommandText::new(0, "true\n", "")),
            FileVaultRead::On
        );
        assert_eq!(
            interpret_isactive(&CommandText::new(0, "false\n", "")),
            FileVaultRead::Off
        );
        assert_eq!(
            console_filevault(FileVaultRead::Unreadable, FileVaultRead::On),
            DiskEncryption::On
        );
        assert_eq!(
            console_filevault(FileVaultRead::Off, FileVaultRead::Unreadable),
            DiskEncryption::Off
        );
    }

    #[test]
    fn a_permission_error_is_not_off() {
        let denied = CommandText::new(1, "", "Error: You need to be root to run this tool.\n");
        assert_eq!(interpret_status(&denied), FileVaultRead::PermissionDenied);
        assert_eq!(interpret_isactive(&denied), FileVaultRead::PermissionDenied);
        assert_eq!(
            console_filevault(
                FileVaultRead::PermissionDenied,
                FileVaultRead::PermissionDenied
            ),
            DiskEncryption::Unknown
        );
        let noisy = CommandText::new(1, "FileVault is Off.\n", "permission denied\n");
        assert_eq!(interpret_status(&noisy), FileVaultRead::PermissionDenied);
        assert_eq!(
            console_filevault(FileVaultRead::PermissionDenied, FileVaultRead::Unreadable),
            DiskEncryption::Unknown
        );
        let nonzero = CommandText::new(1, "FileVault is Off.\n", "");
        assert_eq!(interpret_status(&nonzero), FileVaultRead::Unreadable);
        assert_ne!(
            console_filevault(interpret_status(&nonzero), FileVaultRead::Unreadable),
            DiskEncryption::Off
        );
    }

    #[test]
    fn disagreement_and_garbage_stay_unknown() {
        assert_eq!(
            console_filevault(FileVaultRead::On, FileVaultRead::Off),
            DiskEncryption::Unknown
        );
        assert_eq!(
            console_filevault(FileVaultRead::Off, FileVaultRead::On),
            DiskEncryption::Unknown
        );
        let garbage = CommandText::new(0, "maybe\n", "");
        assert_eq!(interpret_status(&garbage), FileVaultRead::Unreadable);
        assert_eq!(interpret_isactive(&garbage), FileVaultRead::Unreadable);
        assert_eq!(
            console_filevault(FileVaultRead::Unreadable, FileVaultRead::Unreadable),
            DiskEncryption::Unknown
        );
    }
}
