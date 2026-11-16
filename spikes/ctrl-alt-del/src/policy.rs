use serde::{Deserialize, Serialize};

/// User and machine policy key under `Software\Microsoft\Windows\CurrentVersion\Policies\System`.
pub const SYSTEM_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\System";

/// User Explorer policy key.
pub const EXPLORER_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";

/// Where a policy value lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

impl Hive {
    pub fn label(self) -> &'static str {
        match self {
            Self::CurrentUser => "HKCU",
            Self::LocalMachine => "HKLM",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "HKCU" => Some(Self::CurrentUser),
            "HKLM" => Some(Self::LocalMachine),
            _ => None,
        }
    }
}

/// `HKCU\...` or `HKLM\...` for one policy key.
pub fn split_key_id(key_id: &str) -> Option<(Hive, &str)> {
    let (label, path) = key_id.split_once('\\')?;
    Some((Hive::from_label(label)?, path))
}

/// One Ctrl+Alt+Del policy value from the spike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PolicyId {
    DisableTaskMgr,
    NoLogoff,
    DisableChangePassword,
    HideFastUserSwitching,
    DisableLockWorkstation,
    NoClose,
}

/// Registry location and the button it is meant to hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub id: PolicyId,
    pub value_name: &'static str,
    pub screen_label: &'static str,
    pub hive: Hive,
    pub key_path: &'static str,
}

impl PolicyId {
    pub const ALL: [Self; 6] = [
        Self::DisableTaskMgr,
        Self::NoLogoff,
        Self::DisableChangePassword,
        Self::HideFastUserSwitching,
        Self::DisableLockWorkstation,
        Self::NoClose,
    ];

    pub fn name(self) -> &'static str {
        self.info().value_name
    }

    pub fn info(self) -> &'static Policy {
        &POLICIES[self.index()]
    }

    pub fn parse(name: &str) -> Result<Self, super::Error> {
        Self::ALL
            .into_iter()
            .find(|policy| policy.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                super::Error::new(format!(
                    "Unknown setting {name}. Use one of: {}, or all.",
                    Self::ALL
                        .iter()
                        .map(|policy| policy.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|policy| *policy == self)
            .expect("every policy is in ALL")
    }
}

impl std::fmt::Display for PolicyId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

const POLICIES: [Policy; 6] = [
    Policy {
        id: PolicyId::DisableTaskMgr,
        value_name: "DisableTaskMgr",
        screen_label: "Task Manager",
        hive: Hive::CurrentUser,
        key_path: SYSTEM_KEY,
    },
    Policy {
        id: PolicyId::NoLogoff,
        value_name: "NoLogoff",
        screen_label: "Sign out",
        hive: Hive::CurrentUser,
        key_path: EXPLORER_KEY,
    },
    Policy {
        id: PolicyId::DisableChangePassword,
        value_name: "DisableChangePassword",
        screen_label: "Change a password",
        hive: Hive::CurrentUser,
        key_path: SYSTEM_KEY,
    },
    Policy {
        id: PolicyId::HideFastUserSwitching,
        value_name: "HideFastUserSwitching",
        screen_label: "Switch user",
        hive: Hive::LocalMachine,
        key_path: SYSTEM_KEY,
    },
    Policy {
        id: PolicyId::DisableLockWorkstation,
        value_name: "DisableLockWorkstation",
        screen_label: "Lock",
        hive: Hive::CurrentUser,
        key_path: SYSTEM_KEY,
    },
    Policy {
        id: PolicyId::NoClose,
        value_name: "NoClose",
        screen_label: "Power",
        hive: Hive::CurrentUser,
        key_path: EXPLORER_KEY,
    },
];
