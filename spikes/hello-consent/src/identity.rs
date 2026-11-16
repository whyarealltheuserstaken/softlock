//! Foreground identity for the system credential UI.
//!
//! A window qualifies only when its image path is one of the fixed system
//! binaries and the signature check already succeeded. Each process id is
//! inspected once per dialog.

/// Image paths under the Windows directory. This is a fixed list of system
/// credential UI binaries, not a user-editable list.
pub const CREDENTIAL_UI_RELATIVE_PATHS: &[&str] = &[
    r"System32\CredentialUIBroker.exe",
    r"SystemApps\Microsoft.CredDialogHost_cw5n1h2txyewy\CredDialogHost.exe",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureCheck {
    Trusted,
    NotTrusted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageFact {
    pub path: String,
    pub signature: SignatureCheck,
}

pub fn foreground_is_credential_ui(
    windows_dir: &str,
    image_path: &str,
    signature: SignatureCheck,
) -> bool {
    signature == SignatureCheck::Trusted && path_is_system_credential_ui(windows_dir, image_path)
}

pub fn path_is_system_credential_ui(windows_dir: &str, image_path: &str) -> bool {
    let Some(image) = lexical_normal(image_path) else {
        return false;
    };
    CREDENTIAL_UI_RELATIVE_PATHS.iter().any(|relative| {
        expected_path(windows_dir, relative)
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(&image))
    })
}

fn expected_path(windows_dir: &str, relative: &str) -> Option<String> {
    let root = lexical_normal(windows_dir)?;
    lexical_normal(&format!("{root}\\{relative}"))
}

/// Lexical `..` and `.` cleanup. Returns nothing if the path escapes its root.
fn lexical_normal(path: &str) -> Option<String> {
    let path = path.trim();
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    if path.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(['\\', '/']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if parts.len() <= 1 {
                return None;
            }
            parts.pop();
            continue;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\\"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogGate {
    windows_dir: String,
    cache: Vec<(u32, bool)>,
}

impl DialogGate {
    pub fn new(windows_dir: impl Into<String>) -> Self {
        Self {
            windows_dir: windows_dir.into(),
            cache: Vec::new(),
        }
    }

    /// Whether `pid` is an accepted credential UI process.
    /// `inspect` runs at most once per pid until [`Self::end_dialog`].
    pub fn foreground_allowed(
        &mut self,
        pid: Option<u32>,
        inspect: impl FnOnce(u32) -> ImageFact,
    ) -> bool {
        let Some(pid) = pid else {
            return false;
        };
        if let Some((_, allowed)) = self.cache.iter().find(|(cached, _)| *cached == pid) {
            return *allowed;
        }
        let fact = inspect(pid);
        let allowed = foreground_is_credential_ui(&self.windows_dir, &fact.path, fact.signature);
        self.cache.push((pid, allowed));
        allowed
    }

    pub fn end_dialog(&mut self) {
        self.cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS: &str = r"C:\Windows";
    const BROKER: &str = r"C:\Windows\System32\CredentialUIBroker.exe";
    const HOST: &str =
        r"C:\Windows\SystemApps\Microsoft.CredDialogHost_cw5n1h2txyewy\CredDialogHost.exe";

    fn trusted(path: &str) -> bool {
        foreground_is_credential_ui(WINDOWS, path, SignatureCheck::Trusted)
    }

    #[test]
    fn a_trusted_system_credential_binary_matches() {
        assert!(trusted(BROKER));
        assert!(trusted(HOST));
        assert!(trusted(&BROKER.to_ascii_lowercase()));
        assert!(trusted(r"C:/Windows/System32/CredentialUIBroker.exe"));
        assert!(trusted(r"\\?\C:\Windows\System32\CredentialUIBroker.exe"));
        assert!(trusted(
            r"C:\Windows\System32\..\System32\CredentialUIBroker.exe"
        ));
    }

    #[test]
    fn the_signature_must_be_trusted() {
        assert!(!foreground_is_credential_ui(
            WINDOWS,
            BROKER,
            SignatureCheck::NotTrusted
        ));
    }

    #[test]
    fn a_copy_or_a_different_system_binary_does_not_match() {
        assert!(!trusted(r"C:\Temp\CredentialUIBroker.exe"));
        assert!(!trusted(r"C:\Windows\System32\notepad.exe"));
        assert!(!trusted(r"C:\Windows\System32\consent.exe"));
        assert!(!trusted(
            r"C:\Windows\System32\en-US\CredentialUIBroker.exe"
        ));
        assert!(!trusted(
            r"C:\Windows\SystemApps\Microsoft.Other_cw5n1h2txyewy\CredDialogHost.exe"
        ));
        assert!(!trusted(
            r"C:\Windows\System32\CredentialUIBroker.exe\..\..\..\Windows\System32\cmd.exe"
        ));
        assert!(!trusted(""));
        assert!(!foreground_is_credential_ui(
            "",
            BROKER,
            SignatureCheck::Trusted
        ));
    }

    #[test]
    fn each_process_is_inspected_once_per_dialog() {
        let mut gate = DialogGate::new(WINDOWS);
        let mut calls = 0;

        let first = gate.foreground_allowed(Some(10), |_| {
            calls += 1;
            ImageFact {
                path: r"C:\Windows\explorer.exe".into(),
                signature: SignatureCheck::Trusted,
            }
        });
        assert!(!first);
        let again = gate.foreground_allowed(Some(10), |_| {
            calls += 1;
            panic!("cached pid was inspected again");
        });
        assert!(!again);
        assert_eq!(calls, 1);

        let broker = gate.foreground_allowed(Some(20), |_| {
            calls += 1;
            ImageFact {
                path: BROKER.into(),
                signature: SignatureCheck::Trusted,
            }
        });
        assert!(broker);
        let broker_again = gate.foreground_allowed(Some(20), |_| {
            calls += 1;
            panic!("dialog pid was inspected again");
        });
        assert!(broker_again);
        assert_eq!(calls, 2);

        assert!(!gate.foreground_allowed(None, |_| {
            calls += 1;
            panic!("missing pid was inspected");
        }));

        let untrusted = gate.foreground_allowed(Some(21), |_| {
            calls += 1;
            ImageFact {
                path: BROKER.into(),
                signature: SignatureCheck::NotTrusted,
            }
        });
        assert!(!untrusted);

        gate.end_dialog();
        let rechecked = gate.foreground_allowed(Some(20), |_| {
            calls += 1;
            ImageFact {
                path: BROKER.into(),
                signature: SignatureCheck::Trusted,
            }
        });
        assert!(rechecked);
        assert_eq!(calls, 4);
    }

    #[test]
    fn pin_entry_follows_identity_chords_and_the_sixty_second_window() {
        use std::time::Duration;

        use crate::passthrough::{Action, InputOrigin, ModifierKeys, on_key, vk};
        use crate::{PASSTHROUGH_TIMEOUT, PassthroughWindow};

        let mut gate = DialogGate::new(WINDOWS);
        let mut clock = PassthroughWindow::new(PASSTHROUGH_TIMEOUT);
        let mut mods = ModifierKeys::new();

        let notepad = gate.foreground_allowed(Some(4), |_| ImageFact {
            path: r"C:\Windows\System32\notepad.exe".into(),
            signature: SignatureCheck::Trusted,
        });
        let allow = clock.poll(Duration::ZERO, true, notepad);
        assert_eq!(
            on_key(&mut mods, allow, InputOrigin::Physical, vk::DIGIT_1, true),
            Action::Swallow
        );

        let broker = gate.foreground_allowed(Some(9), |_| ImageFact {
            path: BROKER.into(),
            signature: SignatureCheck::Trusted,
        });
        let allow = clock.poll(Duration::from_secs(1), true, broker);
        assert!(allow);
        assert_eq!(
            on_key(&mut mods, allow, InputOrigin::Physical, vk::DIGIT_1, true),
            Action::Pass
        );
        assert_eq!(
            on_key(&mut mods, allow, InputOrigin::Physical, vk::LWIN, true),
            Action::Swallow
        );
        assert_eq!(
            on_key(&mut mods, allow, InputOrigin::Physical, vk::LMENU, true),
            Action::Swallow,
            "Win is still held, so Alt stays swallowed"
        );

        let mut mods = ModifierKeys::new();
        let allow = clock.poll(Duration::from_secs(61), true, broker);
        assert!(!allow, "60 s from the first accepted foreground");
        assert_eq!(
            on_key(&mut mods, allow, InputOrigin::Physical, vk::DIGIT_1, true),
            Action::Swallow
        );

        gate.end_dialog();
        let allow = clock.poll(Duration::from_secs(62), false, false);
        assert!(!allow);
        assert_eq!(
            on_key(
                &mut ModifierKeys::new(),
                allow,
                InputOrigin::Injected,
                vk::A,
                true
            ),
            Action::Pass
        );
    }
}
