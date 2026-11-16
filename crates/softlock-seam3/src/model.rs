//! Snapshot and diff types. No operating-system calls.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RegistryRoot {
    CurrentUser,
    LocalMachine,
    Users,
}

impl RegistryRoot {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CurrentUser => "HKCU",
            Self::LocalMachine => "HKLM",
            Self::Users => "HKU",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegistryPath {
    pub root: RegistryRoot,
    pub key: String,
    pub value: String,
}

impl RegistryPath {
    pub fn new(root: RegistryRoot, key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            root,
            key: key.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryData {
    Dword(u32),
    Qword(u64),
    String(String),
    ExpandString(String),
    MultiString(Vec<String>),
    Binary(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RegistrySnapshot {
    pub values: BTreeMap<RegistryPath, RegistryData>,
}

impl RegistrySnapshot {
    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryChange {
    pub path: RegistryPath,
    pub before: Option<RegistryData>,
    pub after: Option<RegistryData>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RegistryDiff {
    pub changes: Vec<RegistryChange>,
}

impl RegistryDiff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

pub fn diff_registry(before: &RegistrySnapshot, after: &RegistrySnapshot) -> RegistryDiff {
    let mut paths = BTreeMap::new();
    for path in before.values.keys().chain(after.values.keys()) {
        paths.insert(path.clone(), ());
    }
    let mut changes = Vec::new();
    for path in paths.into_keys() {
        let left = before.values.get(&path).cloned();
        let right = after.values.get(&path).cloned();
        if left != right {
            changes.push(RegistryChange {
                path,
                before: left,
                after: right,
            });
        }
    }
    RegistryDiff { changes }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DeviceSnapshot {
    /// Instance id to enabled. Disabled means the device problem is disabled.
    pub devices: BTreeMap<String, bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceChange {
    pub instance_id: String,
    pub before: Option<bool>,
    pub after: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DeviceDiff {
    pub changes: Vec<DeviceChange>,
}

impl DeviceDiff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

pub fn diff_devices(before: &DeviceSnapshot, after: &DeviceSnapshot) -> DeviceDiff {
    let mut ids = BTreeMap::new();
    for id in before.devices.keys().chain(after.devices.keys()) {
        ids.insert(id.clone(), ());
    }
    let mut changes = Vec::new();
    for instance_id in ids.into_keys() {
        let left = before.devices.get(&instance_id).copied();
        let right = after.devices.get(&instance_id).copied();
        if left != right {
            changes.push(DeviceChange {
                instance_id,
                before: left,
                after: right,
            });
        }
    }
    DeviceDiff { changes }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PowerRequestKind {
    Display,
    System,
    AwayMode,
    Execution,
    PerfBoost,
    ActiveLockScreen,
}

impl PowerRequestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Display => "DISPLAY",
            Self::System => "SYSTEM",
            Self::AwayMode => "AWAYMODE",
            Self::Execution => "EXECUTION",
            Self::PerfBoost => "PERFBOOST",
            Self::ActiveLockScreen => "ACTIVELOCKSCREEN",
        }
    }

    pub fn parse(header: &str) -> Option<Self> {
        match header.trim().trim_end_matches(':') {
            "DISPLAY" => Some(Self::Display),
            "SYSTEM" => Some(Self::System),
            "AWAYMODE" => Some(Self::AwayMode),
            "EXECUTION" => Some(Self::Execution),
            "PERFBOOST" => Some(Self::PerfBoost),
            "ACTIVELOCKSCREEN" => Some(Self::ActiveLockScreen),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PowerRequest {
    pub kind: PowerRequestKind,
    pub requester: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PowerSnapshot {
    pub requests: Vec<PowerRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PowerDiff {
    pub added: Vec<PowerRequest>,
    pub removed: Vec<PowerRequest>,
}

impl PowerDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

pub fn diff_power(before: &PowerSnapshot, after: &PowerSnapshot) -> PowerDiff {
    let mut left = counts(&before.requests);
    let mut right = counts(&after.requests);
    let mut keys = BTreeMap::new();
    for key in left.keys().chain(right.keys()) {
        keys.insert(key.clone(), ());
    }
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for key in keys.into_keys() {
        let before_count = left.remove(&key).unwrap_or(0);
        let after_count = right.remove(&key).unwrap_or(0);
        if after_count > before_count {
            added.extend(std::iter::repeat_n(key.clone(), after_count - before_count));
        } else if before_count > after_count {
            removed.extend(std::iter::repeat_n(key, before_count - after_count));
        }
    }
    PowerDiff { added, removed }
}

fn counts(requests: &[PowerRequest]) -> BTreeMap<PowerRequest, usize> {
    let mut map = BTreeMap::new();
    for request in requests {
        *map.entry(request.clone()).or_insert(0) += 1;
    }
    map
}
