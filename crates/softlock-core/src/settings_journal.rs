//! Crash-safe settings journal. No operating-system calls.
//!
//! The document is written and flushed before a setting changes. Restore
//! writes the previous value back, including when that value was absent.
//! The lockdown rows are an allow-list. A DWORD outside that list is not
//! written. Nothing here disables Windows Hello or the password sign-in.
//! [`CONFIG_FILE`] is the config file DACL, restored exactly from the
//! journaled SDDL.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::name::PRODUCT_DISPLAY_NAME;

/// Re-assert the locked value on this interval, and on a registry notification.
pub const REASSERT_AFTER: Duration = Duration::from_secs(5);

/// The one policy ticket 16 applied. Later rows use the same document.
pub const TASK_MANAGER: &str = "DisableTaskMgr";

/// The user-editable config file's DACL. The path is the entry's `path`.
pub const CONFIG_FILE: &str = "ConfigFileAcl";
/// The config directory DACL. The file can stay read-only and still be
/// deleted while the parent grants `FILE_DELETE_CHILD`.
pub const CONFIG_DIR: &str = "ConfigDirAcl";

pub const JOURNAL_VERSION: u32 = 1;

/// User and machine System policy key.
pub const SYSTEM_POLICY: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\System";

/// User Explorer policy key. Sign out and shutdown live here.
pub const EXPLORER_POLICY: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";

const LOCKED_DWORD: u32 = 1;

/// Where a lockdown value is written. User values use the locked user's SID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyHive {
    User,
    Machine,
}

/// One allow-listed policy. `locked` is the value while the session is locked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockdownPolicy {
    pub target: &'static str,
    pub locked: u32,
    pub hive: PolicyHive,
    pub subkey: &'static str,
}

/// Ctrl+Alt+Del and UAC rows. Task Manager stays from ticket 16.
pub const LOCKDOWN: &[LockdownPolicy] = &[
    LockdownPolicy {
        target: TASK_MANAGER,
        locked: 1,
        hive: PolicyHive::User,
        subkey: SYSTEM_POLICY,
    },
    LockdownPolicy {
        target: "NoLogoff",
        locked: 1,
        hive: PolicyHive::User,
        subkey: EXPLORER_POLICY,
    },
    LockdownPolicy {
        target: "DisableChangePassword",
        locked: 1,
        hive: PolicyHive::User,
        subkey: SYSTEM_POLICY,
    },
    LockdownPolicy {
        target: "HideFastUserSwitching",
        locked: 1,
        hive: PolicyHive::Machine,
        subkey: SYSTEM_POLICY,
    },
    LockdownPolicy {
        target: "DisableLockWorkstation",
        locked: 1,
        hive: PolicyHive::User,
        subkey: SYSTEM_POLICY,
    },
    LockdownPolicy {
        target: "NoClose",
        locked: 1,
        hive: PolicyHive::User,
        subkey: EXPLORER_POLICY,
    },
    LockdownPolicy {
        target: "ConsentPromptBehaviorAdmin",
        locked: 1,
        hive: PolicyHive::Machine,
        subkey: SYSTEM_POLICY,
    },
    LockdownPolicy {
        target: "ConsentPromptBehaviorUser",
        locked: 0,
        hive: PolicyHive::Machine,
        subkey: SYSTEM_POLICY,
    },
];

/// These change who can sign in. The lockdown list must not name them.
const SIGN_IN_BLOCKERS: &[&str] = &[
    "scforceoption",
    "DisabledCredentialProviders",
    "AllowDomainPINLogon",
    "DisablePasswordUnlock",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SettingValue {
    #[serde(rename = "absent")]
    Absent,
    #[serde(rename = "dword")]
    Dword { value: u32 },
    #[serde(rename = "sddl")]
    Sddl { value: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingEntry {
    pub target: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    pub previous: SettingValue,
    pub new_value: SettingValue,
    /// Owner SID before the lock. Restore writes it back with the DACL.
    /// Empty on a journal saved before the owner was recorded.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub previous_owner: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsJournal {
    pub v: u32,
    /// The user who was locked. Restore writes this hive and no other.
    #[serde(default)]
    pub sid: String,
    /// Flushed before `DisableLockWorkstation` is cleared for a real OS lock.
    #[serde(default)]
    pub workstation_clear: bool,
    pub says_locked: bool,
    /// Policies were restored after a confirmed OS lock. Startup stays
    /// `OsLocked` until the console is active again. SPEC section 7: the
    /// state is `OsLocked` until the owner signs back in, and that recovery
    /// never reports `Unlocked`.
    #[serde(default)]
    pub os_confirmed: bool,
    pub failures: u32,
    /// Monotonic millisecond clock when this backoff ends. Zero means none.
    #[serde(default)]
    pub backoff_until_tick: u64,
    /// Monotonic millisecond clock when the deadline was written.
    #[serde(default)]
    pub written_tick: u64,
    /// Unix milliseconds when the deadline was written. An earlier reading
    /// later is a clock rollback and restarts the delay.
    #[serde(default)]
    pub written_wall_ms: u64,
    /// Password failures still inside the Windows observation window.
    /// A PIN failure is not one of these. Unlock keeps the list; a successful
    /// password check clears it.
    #[serde(default)]
    pub password_attempts: Vec<crate::backoff::PasswordStamp>,
    pub entries: Vec<SettingEntry>,
    /// Boot time and the logon that was locked. Absent on a journal written
    /// before this field existed, which startup treats as unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<LockIdentity>,
}

/// The boot and the logon recorded when the journal was saved.
/// Compared at the next service start. The file that holds it is
/// SYSTEM and Administrators only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockIdentity {
    /// Unbiased interrupt time at lock, in 100 ns units. This counter does
    /// not move when the wall clock changes. A smaller reading at the next
    /// start is a new boot. Equality of `BootTime` is not used.
    pub boot: u64,
    /// Console session id at lock time.
    pub session: u32,
    /// That session's logon LUID (`AuthenticationId`).
    pub logon: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalFault {
    Malformed,
    Version,
    Target,
    Value,
    Duplicate,
    NotDurable,
    Sid,
    Link,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitFault<E> {
    Journal(JournalFault),
    Change(E),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartAction {
    Idle,
    Revert,
    FailClosed,
    /// Policies are already restored and the console is not active.
    /// Stay `OsLocked`. Do not revert again and do not report `Unlocked`.
    StayOsLocked,
}

/// What a missing session means. A disconnect drops the console and can drop
/// the session id without a new logon. That is not proof the lock ended.
/// SPEC section 7 and Decision 5: recovery never reports `Unlocked`.
/// `listed == None` is a failed query. `same_logon == None` is a failed
/// LUID read. The console being active while the old session is gone is a
/// new logon.
pub fn fact_for_session(
    listed: Option<bool>,
    console_active: bool,
    same_logon: Option<bool>,
) -> SessionFact {
    match listed {
        None => SessionFact::Unknown,
        Some(true) => match same_logon {
            None => SessionFact::Unknown,
            Some(true) => SessionFact::SameLogon,
            Some(false) => SessionFact::DifferentLogon,
        },
        Some(false) if console_active => SessionFact::SessionGone,
        Some(false) => SessionFact::Unknown,
    }
}

/// A journal whose policies were restored after a confirmed OS lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldStart {
    /// The console is active. The owner is back.
    OwnerReturned,
    /// The console is not active. Stay `OsLocked`.
    StayOsLocked,
}

/// The locked session is the console again, and it is the same logon.
/// A different console session, or the same session while it is still
/// disconnected, is not a sign-in.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn owner_reconnected(
    saved: LockIdentity,
    console_session: u32,
    console_active: bool,
    console_logon: Option<u64>,
) -> bool {
    console_active
        && console_session != u32::MAX
        && console_session == saved.session
        && console_logon.is_some_and(|logon| logon == saved.logon && logon != 0)
}

/// A running service that confirmed a disconnect uses this when the owner
/// is back. It is [`repair_action`] for `OsLocked`: the same revert the
/// ONLOGON repair task runs. A soft lock does not take this path.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn sign_in_release(state: ProductState, reconnected: bool) -> RepairAction {
    if reconnected
        && repair_action(state) == RepairAction::Revert
        && state == ProductState::OsLocked
    {
        RepairAction::Revert
    } else {
        RepairAction::Refuse
    }
}

/// A confirmed OS lock is released only when the logon actually changed.
/// The same logon on an active console is not, by itself, a new sign-in.
/// The running service uses [`owner_reconnected`] for that. SPEC section 7
/// keeps a restart `OsLocked` until then.
pub fn release_os_hold(fact: SessionFact) -> bool {
    matches!(
        fact,
        SessionFact::DifferentLogon | SessionFact::SessionGone | SessionFact::DifferentBoot
    )
}

pub fn held_start(doc: &SettingsJournal, console_active: bool) -> Option<HeldStart> {
    if !doc.os_confirmed || doc.says_locked() || !doc.entries.is_empty() {
        return None;
    }
    Some(if console_active {
        HeldStart::OwnerReturned
    } else {
        HeldStart::StayOsLocked
    })
}

/// Whether the Lock Host from this install is still the locked session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostPresence {
    Alive,
    Dead,
    /// A snapshot or token we could not read. A locked journal fails closed.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductState {
    Unlocked,
    Locked,
    Arming,
    Unlocking,
    FailingClosed,
    Recovering,
    OsLocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairAction {
    Revert,
    Refuse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirOwner {
    System,
    Administrators,
    User,
    Unreadable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirFacts {
    pub owner: DirOwner,
    pub acl_exact: bool,
    pub reparse: bool,
    pub planted_file: bool,
    pub hard_link: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirFault {
    Reparse,
    HardLink,
    Planted,
    UserOwned,
    Acl,
    Unreadable,
}

impl SettingsJournal {
    pub fn empty() -> Self {
        Self {
            v: JOURNAL_VERSION,
            sid: String::new(),
            workstation_clear: false,
            says_locked: false,
            os_confirmed: false,
            failures: 0,
            backoff_until_tick: 0,
            written_tick: 0,
            written_wall_ms: 0,
            password_attempts: Vec::new(),
            entries: Vec::new(),
            lock: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && !self.says_locked && !self.os_confirmed && self.failures == 0
    }

    pub fn entries(&self) -> &[SettingEntry] {
        &self.entries
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    pub fn says_locked(&self) -> bool {
        self.says_locked
    }
}

pub fn allowed_previous(value: &SettingValue) -> bool {
    match value {
        SettingValue::Absent => true,
        SettingValue::Dword { value: 0 | 1 } => true,
        SettingValue::Dword { .. } | SettingValue::Sddl { .. } => false,
    }
}

pub fn task_manager_document(
    sid: &str,
    previous: SettingValue,
    failures: u32,
) -> Result<SettingsJournal, JournalFault> {
    if !sid_is_safe(sid) {
        return Err(JournalFault::Sid);
    }
    if !allowed_previous(&previous) {
        return Err(JournalFault::Value);
    }
    Ok(SettingsJournal {
        v: JOURNAL_VERSION,
        sid: sid.to_string(),
        workstation_clear: false,
        says_locked: true,
        os_confirmed: false,
        failures,
        backoff_until_tick: 0,
        written_tick: 0,
        written_wall_ms: 0,
        password_attempts: Vec::new(),
        entries: vec![SettingEntry {
            target: TASK_MANAGER.to_string(),
            path: String::new(),
            previous,
            new_value: SettingValue::Dword {
                value: LOCKED_DWORD,
            },
            previous_owner: String::new(),
        }],
        lock: None,
    })
}

pub fn lockdown_policy(target: &str) -> Option<&'static LockdownPolicy> {
    LOCKDOWN.iter().find(|policy| policy.target == target)
}

pub fn lockdown_target(target: &str) -> bool {
    lockdown_policy(target).is_some()
}

/// A sign-in policy is never a lockdown row. Windows Hello and the password
/// stay available at the real lock screen.
pub fn blocks_owner_sign_in(target: &str) -> bool {
    SIGN_IN_BLOCKERS
        .iter()
        .any(|name| name.eq_ignore_ascii_case(target))
}

pub fn allowed_previous_for(target: &str, value: &SettingValue) -> bool {
    match value {
        SettingValue::Absent => lockdown_policy(target).is_some(),
        SettingValue::Dword { value } => allowed_dword(target, *value),
        SettingValue::Sddl { .. } => false,
    }
}

fn allowed_dword(target: &str, value: u32) -> bool {
    match target {
        "ConsentPromptBehaviorAdmin" => value <= 5,
        "ConsentPromptBehaviorUser" => matches!(value, 0 | 1 | 3),
        _ if lockdown_policy(target).is_some() => matches!(value, 0 | 1),
        _ => false,
    }
}

/// Every lockdown row, once. `previous` is what restore writes back.
pub fn lockdown_document(
    sid: &str,
    previous: &[(&str, SettingValue)],
    failures: u32,
) -> Result<SettingsJournal, JournalFault> {
    if !sid_is_safe(sid) {
        return Err(JournalFault::Sid);
    }
    if previous.len() != LOCKDOWN.len() {
        return Err(JournalFault::Target);
    }
    let mut entries = Vec::with_capacity(LOCKDOWN.len());
    for policy in LOCKDOWN {
        let Some((_, value)) = previous.iter().find(|(target, _)| *target == policy.target) else {
            return Err(JournalFault::Target);
        };
        if !allowed_previous_for(policy.target, value) {
            return Err(JournalFault::Value);
        }
        entries.push(SettingEntry {
            target: policy.target.to_string(),
            path: String::new(),
            previous: value.clone(),
            new_value: SettingValue::Dword {
                value: policy.locked,
            },
            previous_owner: String::new(),
        });
    }
    Ok(SettingsJournal {
        v: JOURNAL_VERSION,
        sid: sid.to_string(),
        workstation_clear: false,
        says_locked: true,
        os_confirmed: false,
        failures,
        backoff_until_tick: 0,
        written_tick: 0,
        written_wall_ms: 0,
        password_attempts: Vec::new(),
        entries,
        lock: None,
    })
}

/// Record that `DisableLockWorkstation` is about to be cleared. The previous
/// value stays so restore is exact. The save has to finish before the delete.
pub fn mark_workstation_clear(doc: &SettingsJournal) -> Result<SettingsJournal, JournalFault> {
    validate(doc)?;
    if !doc
        .entries
        .iter()
        .any(|entry| entry.target == "DisableLockWorkstation")
    {
        return Err(JournalFault::Target);
    }
    let mut next = doc.clone();
    next.workstation_clear = true;
    Ok(next)
}

/// `skip` is true only while a workstation lock has cleared that one value.
pub fn reassert_write(
    target: &str,
    locked: &SettingValue,
    current: &SettingValue,
    skip: bool,
) -> Option<SettingValue> {
    if skip && target == "DisableLockWorkstation" {
        return None;
    }
    if current == locked {
        None
    } else {
        Some(locked.clone())
    }
}

pub fn encode_document(doc: &SettingsJournal) -> Result<Vec<u8>, JournalFault> {
    validate(doc)?;
    serde_json::to_vec(doc).map_err(|_| JournalFault::Malformed)
}

pub fn parse_document(bytes: &[u8]) -> Result<SettingsJournal, JournalFault> {
    let doc: SettingsJournal =
        serde_json::from_slice(bytes).map_err(|_| JournalFault::Malformed)?;
    validate(&doc)?;
    Ok(doc)
}

fn validate(doc: &SettingsJournal) -> Result<(), JournalFault> {
    if doc.v != JOURNAL_VERSION {
        return Err(JournalFault::Version);
    }
    if doc.is_empty() {
        if doc.sid.is_empty() || sid_is_safe(&doc.sid) {
            return Ok(());
        }
        return Err(JournalFault::Sid);
    }
    if !sid_is_safe(&doc.sid) {
        return Err(JournalFault::Sid);
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in &doc.entries {
        if !seen.insert(entry.target.clone()) {
            return Err(JournalFault::Duplicate);
        }
        if entry.target == CONFIG_FILE || entry.target == CONFIG_DIR {
            let path_ok = if entry.target == CONFIG_FILE {
                config_path_allowed(&entry.path)
            } else {
                config_dir_allowed(&entry.path)
            };
            if !path_ok {
                return Err(JournalFault::Target);
            }
            let SettingValue::Sddl { value: previous } = &entry.previous else {
                return Err(JournalFault::Value);
            };
            let SettingValue::Sddl { value: locked } = &entry.new_value else {
                return Err(JournalFault::Value);
            };
            if !sddl_allowed(previous) || !sddl_allowed(locked) {
                return Err(JournalFault::Value);
            }
            if !owner_sid_ok(&entry.previous_owner) {
                return Err(JournalFault::Sid);
            }
            continue;
        }
        if lockdown_policy(&entry.target).is_none() {
            return Err(JournalFault::Target);
        }
        if !entry.path.is_empty() {
            return Err(JournalFault::Target);
        }
        if !allowed_previous_for(&entry.target, &entry.previous) {
            return Err(JournalFault::Value);
        }
        let locked = lockdown_policy(&entry.target)
            .expect("target was just checked")
            .locked;
        if entry.new_value != (SettingValue::Dword { value: locked }) {
            return Err(JournalFault::Value);
        }
    }
    if doc.workstation_clear
        && !doc
            .entries
            .iter()
            .any(|entry| entry.target == "DisableLockWorkstation")
    {
        return Err(JournalFault::Target);
    }
    if let Some(lock) = doc.lock
        && (lock.boot == 0 || lock.logon == 0 || lock.session == u32::MAX)
    {
        return Err(JournalFault::Value);
    }
    Ok(())
}

pub fn config_acl_entry(
    path: &str,
    previous: &str,
    locked: &str,
    owner: &str,
) -> Result<SettingEntry, JournalFault> {
    acl_entry(
        CONFIG_FILE,
        path,
        config_path_allowed(path),
        previous,
        locked,
        owner,
    )
}

pub fn config_dir_entry(
    path: &str,
    previous: &str,
    locked: &str,
    owner: &str,
) -> Result<SettingEntry, JournalFault> {
    acl_entry(
        CONFIG_DIR,
        path,
        config_dir_allowed(path),
        previous,
        locked,
        owner,
    )
}

fn acl_entry(
    target: &str,
    path: &str,
    path_ok: bool,
    previous: &str,
    locked: &str,
    owner: &str,
) -> Result<SettingEntry, JournalFault> {
    if !owner_sid_ok(owner) {
        return Err(JournalFault::Sid);
    }
    let entry = SettingEntry {
        target: target.to_string(),
        path: path.to_string(),
        previous: SettingValue::Sddl {
            value: previous.to_string(),
        },
        new_value: SettingValue::Sddl {
            value: locked.to_string(),
        },
        previous_owner: owner.to_string(),
    };
    if !path_ok {
        return Err(JournalFault::Target);
    }
    let SettingValue::Sddl { value: previous } = &entry.previous else {
        return Err(JournalFault::Value);
    };
    let SettingValue::Sddl { value: locked } = &entry.new_value else {
        return Err(JournalFault::Value);
    };
    if !sddl_allowed(previous) || !sddl_allowed(locked) {
        return Err(JournalFault::Value);
    }
    Ok(entry)
}

pub fn with_config_acl(
    mut doc: SettingsJournal,
    entry: SettingEntry,
) -> Result<SettingsJournal, JournalFault> {
    if entry.target != CONFIG_FILE && entry.target != CONFIG_DIR {
        return Err(JournalFault::Target);
    }
    doc.entries.push(entry);
    validate(&doc)?;
    Ok(doc)
}

/// Remove write for everyone except SYSTEM, including inherited allow ACEs.
/// The result is a protected DACL so the parent cannot grant that write back.
/// The caller journals the original.
pub fn dacl_without_owner_write(dacl: &str, console_sid: &str) -> Result<String, JournalFault> {
    if !sddl_allowed(dacl) || !sid_is_safe(console_sid) {
        return Err(JournalFault::Value);
    }
    let mut out = String::new();
    let mut rest = dacl;
    while let Some(start) = rest.find('(') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find(')') else {
            return Err(JournalFault::Value);
        };
        let ace = &rest[start..=start + end];
        let (rewritten, _) = rewrite_owner_ace(ace, console_sid)?;
        out.push_str(&explicit_ace(&rewritten));
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    // Without an Owner Rights ACE, Windows still grants the owner WRITE_DAC.
    if !out.contains(";;;OW)") && !out.to_ascii_uppercase().contains(";;;S-1-3-4)") {
        out.push_str("(A;;FR;;;OW)");
    }
    // A protected DACL must still name SYSTEM. An inherited-only copy is not
    // enough once the parent can no longer grant it.
    if !out.contains(";;;SY)") && !out.to_ascii_uppercase().contains(";;;S-1-5-18)") {
        out.push_str("(A;;FA;;;SY)");
    }
    let protected = if let Some(rest) = out.strip_prefix("D:") {
        if rest.starts_with('P') {
            out
        } else {
            format!("D:P{rest}")
        }
    } else {
        return Err(JournalFault::Value);
    };
    if !sddl_allowed(&protected) {
        return Err(JournalFault::Value);
    }
    Ok(protected)
}

/// The descriptor is the locked config, judged from the bytes that were read.
///
/// Access denied is not evidence. Held means the DACL is protected, Owner
/// Rights is present and limited to read, nobody but SYSTEM or Administrators
/// is granted write, `WRITE_DAC`, `WRITE_OWNER`, or `DELETE`, a deny does not
/// take those rights away from SYSTEM or Administrators, and the owner is
/// SYSTEM or Administrators. Windows may print `FR` as a hex mask and `SY`
/// as `S-1-5-18`; both forms count.
pub fn config_lock_held(dacl: &str, owner: &str) -> bool {
    if !sddl_allowed(dacl) || !dacl_is_protected(dacl) || !privileged_trustee(owner) {
        return false;
    }
    let mut privileged_can_repair = false;
    let mut owner_rights_limited = false;
    let mut rest = dacl;
    while let Some(start) = rest.find('(') {
        let Some(end) = rest[start..].find(')') else {
            return false;
        };
        let ace = &rest[start..=start + end];
        let Some(parts) = ace_parts(ace) else {
            return false;
        };
        let allow = parts[0] == "A";
        let deny = parts[0] == "D";
        let trustee = parts[5];
        let write = ace_grants_write(parts[2]);
        if is_owner_rights(trustee) {
            if !allow || write {
                return false;
            }
            owner_rights_limited = true;
        } else if privileged_trustee(trustee) {
            if deny && write {
                return false;
            }
            if allow && write {
                privileged_can_repair = true;
            }
        } else if allow && write {
            return false;
        }
        rest = &rest[start + end + 1..];
    }
    owner_rights_limited && privileged_can_repair
}

fn dacl_is_protected(dacl: &str) -> bool {
    let Some(rest) = dacl.strip_prefix("D:") else {
        return false;
    };
    rest.split('(').next().unwrap_or("").contains('P')
}

fn ace_parts(ace: &str) -> Option<Vec<&str>> {
    let inner = ace.strip_prefix('(')?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(';').collect();
    if parts.len() == 6 { Some(parts) } else { None }
}

fn is_owner_rights(trustee: &str) -> bool {
    trustee.eq_ignore_ascii_case("OW") || trustee.eq_ignore_ascii_case("S-1-3-4")
}

fn privileged_trustee(trustee: &str) -> bool {
    trustee.eq_ignore_ascii_case("SY")
        || trustee.eq_ignore_ascii_case("S-1-5-18")
        || trustee.eq_ignore_ascii_case("BA")
        || trustee.eq_ignore_ascii_case("S-1-5-32-544")
}

/// Bytes appended after `baseline`. A log that was replaced, or a snapshot
/// that does not prefix the current text, contributes nothing. An empty
/// suffix has no confirmation line.
pub fn fresh_log<'a>(baseline: &str, log: &'a str) -> &'a str {
    if baseline.len() > log.len() || !log.is_char_boundary(baseline.len()) {
        return "";
    }
    if log.as_bytes().get(..baseline.len()) != Some(baseline.as_bytes()) {
        return "";
    }
    &log[baseline.len()..]
}

/// `Unlocked` after auto-logon counts only when this wait's new log text
/// contains `os-lock result=confirmed` and the original value is back.
pub fn confirmed_during_wait(
    baseline: &str,
    log: &str,
    value: &str,
    expected: &str,
    journal_still_locked: bool,
) -> bool {
    !journal_still_locked
        && value == expected
        && fresh_log(baseline, log).contains("os-lock result=confirmed")
}

/// Drop the inherited-ACE flag. A protected DACL still evaluates an ACE
/// marked inherited, but the copy must stand on its own after the parent
/// stops granting the same access.
fn explicit_ace(ace: &str) -> String {
    let Some(inner) = ace
        .strip_prefix('(')
        .and_then(|text| text.strip_suffix(')'))
    else {
        return ace.to_string();
    };
    let parts: Vec<&str> = inner.split(';').collect();
    if parts.len() != 6 {
        return ace.to_string();
    }
    format!(
        "({};{};{};{};{};{})",
        parts[0],
        parts[1].replace("ID", ""),
        parts[2],
        parts[3],
        parts[4],
        parts[5]
    )
}

fn rewrite_owner_ace(ace: &str, _owner: &str) -> Result<(String, bool), JournalFault> {
    let inner = ace
        .strip_prefix('(')
        .and_then(|text| text.strip_suffix(')'))
        .ok_or(JournalFault::Value)?;
    let parts: Vec<&str> = inner.split(';').collect();
    if parts.len() != 6 {
        return Err(JournalFault::Value);
    }
    let write = ace_grants_write(parts[2]);
    let system = parts[5].eq_ignore_ascii_case("SY") || parts[5].eq_ignore_ascii_case("S-1-5-18");
    // A deny of write for SYSTEM or Administrators cannot be the locked
    // descriptor: reassert would read it and refuse to call it held.
    if parts[0] == "D" && write && privileged_trustee(parts[5]) {
        return Ok((String::new(), true));
    }
    // Everyone except SYSTEM loses write. That includes the console user,
    // Administrators (the console user is a member), Owner Rights, and
    // Creator Owner. SYSTEM keeps full control and puts the original back.
    if parts[0] == "A" && write && !system {
        return Ok((
            format!("(A;{};FR;{};{};{})", parts[1], parts[3], parts[4], parts[5]),
            true,
        ));
    }
    Ok((ace.to_string(), false))
}

fn ace_grants_write(rights: &str) -> bool {
    if rights.eq_ignore_ascii_case("FR") || rights.eq_ignore_ascii_case("GR") {
        return false;
    }
    if let Some(hex) = rights
        .strip_prefix("0x")
        .or_else(|| rights.strip_prefix("0X"))
    {
        let Ok(mask) = u32::from_str_radix(hex, 16) else {
            return true;
        };
        // FILE_WRITE_DATA, FILE_APPEND_DATA, FILE_WRITE_EA, FILE_WRITE_ATTRIBUTES,
        // DELETE, WRITE_DAC, WRITE_OWNER, and the generic write bit.
        // 0x40 is FILE_DELETE_CHILD, the parent right that deletes the file.
        return mask & (0x0000_0156 | 0x0001_0000 | 0x0004_0000 | 0x0008_0000 | 0x4000_0000) != 0;
    }
    let upper = rights.to_ascii_uppercase();
    upper.contains("FA")
        || upper.contains("GA")
        || upper.contains("GW")
        || upper.contains("WD")
        || upper.contains("AD")
        || upper.contains("DC")
        || upper.contains("WE")
        || upper.contains("WO")
        || upper.contains("SD")
        || upper.contains("DT")
}

pub fn sddl_allowed(text: &str) -> bool {
    if text.len() < 4 || text.len() > 4096 || !text.starts_with("D:") {
        return false;
    }
    text.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | ';' | '(' | ')' | ' '))
        && text.contains('(')
        && !text.contains("..")
}

pub fn config_path_allowed(path: &str) -> bool {
    if configured_file_allowed(path) {
        return true;
    }
    if path.len() < 8 || path.len() > 260 || path.contains("..") || path.contains('/') {
        return false;
    }
    let bytes = path.as_bytes();
    if !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return false;
    }
    if !path.chars().all(|c| c.is_ascii() && !c.is_control()) {
        return false;
    }
    let suffix = format!(
        "\\AppData\\Roaming\\{}\\{}",
        crate::name::SERVICE_NAME,
        crate::config::CONFIG_FILE_NAME
    );
    let Some(profile) = path.strip_suffix(&suffix) else {
        return false;
    };
    profile_root_allowed(profile)
}

/// The directory that contains [`config_path_allowed`]'s file.
pub fn config_dir_allowed(path: &str) -> bool {
    let file = format!("{path}\\{}", crate::config::CONFIG_FILE_NAME);
    config_path_allowed(&file)
}

/// The service's own config file when `SOFTLOCK_CONFIG_DIR` names it.
/// The roaming profile check still rejects every other path. A journal
/// cannot retarget the DACL at `C:\Windows` or at a path the service did
/// not open.
fn configured_file_allowed(path: &str) -> bool {
    let Ok(dir) = std::env::var("SOFTLOCK_CONFIG_DIR") else {
        return false;
    };
    configured_file_matches(path, &dir)
}

fn configured_file_matches(path: &str, dir: &str) -> bool {
    if dir.is_empty()
        || dir.contains("..")
        || dir.contains('"')
        || dir.contains('*')
        || dir.contains('/')
    {
        return false;
    }
    let dir = dir.trim_end_matches(['\\', '/']);
    let expected = format!("{dir}\\{}", crate::config::CONFIG_FILE_NAME);
    if !path.eq_ignore_ascii_case(&expected) {
        return false;
    }
    let bytes = path.as_bytes();
    let lower = path.to_ascii_lowercase();
    bytes.len() >= 8
        && bytes.len() <= 260
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'\\'
        && path.chars().all(|c| c.is_ascii() && !c.is_control())
        && !lower.contains("\\windows\\")
        && !lower.contains("\\program files")
}

fn profile_root_allowed(profile: &str) -> bool {
    let mut parts = profile.split('\\');
    let drive = parts.next();
    let users = parts.next();
    let account = parts.next();
    drive.is_some_and(|text| text.len() == 2 && text.as_bytes()[1] == b':')
        && users.is_some_and(|text| text.eq_ignore_ascii_case("Users"))
        && account.is_some_and(account_component)
        && parts.next().is_none()
}

fn account_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 64
        && name.chars().all(|c| {
            c.is_ascii()
                && !c.is_control()
                && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
}

/// The value a restart writes back.
///
/// A live value equal to the lockdown dword is the lock itself. The journaled
/// previous wins, so that dword is not stored as the original. Any other live
/// value is the user's value, including a value written after a stale journal
/// still said the original was absent. Reverting that stale absent would
/// delete the user's value.
pub fn original_previous<'a>(
    journaled: Option<&'a SettingValue>,
    current: &'a SettingValue,
    locked: &SettingValue,
) -> &'a SettingValue {
    if current == locked {
        journaled.unwrap_or(current)
    } else {
        current
    }
}

/// `Ok(None)` means the current value already matches and the write is skipped.
pub fn restore_action_for(
    target: &str,
    previous: &SettingValue,
    current: &SettingValue,
) -> Result<Option<SettingValue>, JournalFault> {
    if !allowed_previous_for(target, previous) {
        return Err(JournalFault::Value);
    }
    if current == previous {
        Ok(None)
    } else {
        Ok(Some(previous.clone()))
    }
}

/// `Ok(None)` means the current value already matches and the write is skipped.
pub fn restore_action(
    previous: &SettingValue,
    current: &SettingValue,
) -> Result<Option<SettingValue>, JournalFault> {
    if matches!(previous, SettingValue::Sddl { .. }) || matches!(current, SettingValue::Sddl { .. })
    {
        let SettingValue::Sddl {
            value: previous_text,
        } = previous
        else {
            return Err(JournalFault::Value);
        };
        let SettingValue::Sddl {
            value: current_text,
        } = current
        else {
            return Err(JournalFault::Value);
        };
        if !sddl_allowed(previous_text) || !sddl_allowed(current_text) {
            return Err(JournalFault::Value);
        }
        return if previous_text == current_text {
            Ok(None)
        } else {
            Ok(Some(previous.clone()))
        };
    }
    if !allowed_previous(previous) {
        return Err(JournalFault::Value);
    }
    if current == previous {
        Ok(None)
    } else {
        Ok(Some(previous.clone()))
    }
}

pub fn reassert_due(elapsed: Duration) -> bool {
    elapsed >= REASSERT_AFTER
}

/// Put `DisableLockWorkstation` back after an OS-lock attempt. The temporary
/// clear flag does not survive, including when a later disconnect is what
/// confirmed the lock.
pub fn workstation_value_after_attempt(
    doc: &SettingsJournal,
) -> Result<Option<(SettingsJournal, SettingValue)>, JournalFault> {
    validate(doc)?;
    let Some(policy) = lockdown_policy("DisableLockWorkstation") else {
        return Ok(None);
    };
    if !doc
        .entries
        .iter()
        .any(|entry| entry.target == policy.target)
    {
        return Ok(None);
    }
    let mut next = doc.clone();
    next.workstation_clear = false;
    Ok(Some((
        next,
        SettingValue::Dword {
            value: policy.locked,
        },
    )))
}

/// Save first. A failed save does not run `change`.
pub fn commit_then_change<E>(
    save: impl FnOnce() -> Result<(), JournalFault>,
    change: impl FnOnce() -> Result<(), E>,
) -> Result<(), CommitFault<E>> {
    save().map_err(CommitFault::Journal)?;
    change().map_err(CommitFault::Change)
}

/// What startup could prove about the logon that the journal recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionFact {
    /// Same boot, and the same logon still holds the recorded session id.
    SameLogon,
    /// Monotonic uptime went backwards. That is a reboot, not a clock change.
    DifferentBoot,
    /// That session id is not in the current session list.
    SessionGone,
    /// The session id is present and a different logon holds it.
    DifferentLogon,
    /// A query failed, or the journal has no identity. That is not proof
    /// the lock ended.
    Unknown,
}

/// Decide whether uptime proves a new boot.
///
/// `saved` and `now` are the same monotonic counter (unbiased interrupt
/// time). A smaller `now` is a reboot or a power loss. A wall-clock change,
/// including a large forward or backward jump, does not move this counter,
/// so it is not a reboot. `now == None` means the uptime query failed. That
/// is not proof of a new boot either: the session fact stands, and
/// [`SessionFact::Unknown`] fails closed.
///
/// SPEC Decision 5: only `NoActiveLock` reverts. A clock change is not that.
pub fn fact_after_boot(saved: u64, now: Option<u64>, session: SessionFact) -> SessionFact {
    if let Some(now) = now
        && now < saved
    {
        return SessionFact::DifferentBoot;
    }
    session
}

/// Startup with a locked journal.
///
/// A different boot, a missing session, or a different logon is
/// `NoActiveLock`: revert and stay `Unlocked`. The same logon, a dead Lock
/// Host, an active console, or any query error is still the lock. That
/// returns [`StartAction::FailClosed`], which asks for the OS lock and
/// does not end `Unlocked`. SPEC Decision 5 (`Recovering` transitions) and
/// section 7: policies stay applied until the OS lock is confirmed.
///
/// `console_active` and `host` do not override that. A dead host used to
/// revert. That unlocked a session whose logon had not ended.
pub fn recovery_action(
    doc: &SettingsJournal,
    fact: SessionFact,
    console_active: bool,
    host: HostPresence,
) -> StartAction {
    if doc.is_empty() {
        return StartAction::Idle;
    }
    if !doc.says_locked() {
        return StartAction::Revert;
    }
    let _ = (console_active, host);
    match fact {
        SessionFact::DifferentBoot | SessionFact::SessionGone | SessionFact::DifferentLogon => {
            StartAction::Revert
        }
        SessionFact::SameLogon | SessionFact::Unknown => StartAction::FailClosed,
    }
}

/// Service startup order. Journal recovery runs before the config file is
/// created or applied, so that apply cannot rebuild the journal from the
/// live lockdown values.
pub fn startup_steps() -> [StartupStep; 2] {
    [StartupStep::RecoverJournal, StartupStep::LoadConfig]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupStep {
    RecoverJournal,
    LoadConfig,
}

pub fn service_start_action(
    doc: &SettingsJournal,
    console_active: bool,
    host: HostPresence,
) -> StartAction {
    // No boot or logon comparison has been supplied. A locked journal is
    // therefore the same session, or unknown. Either one fails closed.
    recovery_action(doc, SessionFact::SameLogon, console_active, host)
}

/// The hive written at lock time. The console user is not a substitute.
pub fn restore_sid(doc: &SettingsJournal) -> Result<&str, JournalFault> {
    if !sid_is_safe(&doc.sid) {
        return Err(JournalFault::Sid);
    }
    Ok(doc.sid.as_str())
}

pub fn repair_action(state: ProductState) -> RepairAction {
    match state {
        ProductState::Unlocked | ProductState::OsLocked => RepairAction::Revert,
        ProductState::Locked
        | ProductState::Arming
        | ProductState::Unlocking
        | ProductState::FailingClosed
        | ProductState::Recovering => RepairAction::Refuse,
    }
}

pub fn admit_directory(facts: DirFacts) -> Result<(), DirFault> {
    if facts.reparse {
        return Err(DirFault::Reparse);
    }
    if facts.hard_link {
        return Err(DirFault::HardLink);
    }
    if facts.planted_file {
        return Err(DirFault::Planted);
    }
    match facts.owner {
        DirOwner::Unreadable => return Err(DirFault::Unreadable),
        DirOwner::User => return Err(DirFault::UserOwned),
        DirOwner::System | DirOwner::Administrators => {}
    }
    if !facts.acl_exact {
        return Err(DirFault::Acl);
    }
    Ok(())
}

/// The task may run only this service image, after `..` is rejected and the
/// path is one of the installed locations. A prefix or substring match is not enough.
pub fn executable_may_run_as_system(candidate: &Path, service_image: &Path) -> bool {
    match (lexical_exe(candidate), lexical_exe(service_image)) {
        (Some(candidate), Some(image)) => candidate == image && trusted_install(&image),
        _ => false,
    }
}

/// Same executable path. `..` and a non-exe are not the same as anything.
pub fn same_exe_path(left: &Path, right: &Path) -> bool {
    match (lexical_exe(left), lexical_exe(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn lexical_exe(path: &Path) -> Option<String> {
    let text = path.to_string_lossy().replace('/', "\\");
    if text.is_empty()
        || text.contains('"')
        || text.contains('\'')
        || text.contains("..")
        || text.contains('.') && text.split('\\').any(|part| part == ".")
    {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    let absolute =
        lower.starts_with('\\') || (lower.len() > 2 && lower.as_bytes().get(1) == Some(&b':'));
    if !absolute || !lower.ends_with(".exe") {
        return None;
    }
    Some(lower)
}

fn trusted_install(lower: &str) -> bool {
    let parts: Vec<&str> = lower.split('\\').filter(|part| !part.is_empty()).collect();
    if parts.iter().any(|part| *part == "." || *part == "..") {
        return false;
    }
    let body = if parts.first().is_some_and(|part| part.ends_with(':')) {
        &parts[1..]
    } else {
        &parts[..]
    };
    component_prefix(body, &["program files"])
        || component_prefix(body, &["program files (x86)"])
        || component_prefix(body, &["windows", "system32"])
        || component_prefix(body, &["windows", "syswow64"])
}

fn component_prefix(body: &[&str], prefix: &[&str]) -> bool {
    body.len() > prefix.len() && body[..prefix.len()] == prefix[..]
}

/// A locked user is a domain account (`S-1-5-21-` plus a rid). Service accounts are not.
/// The console user is the SID from `WTSQueryUserToken` when that call
/// works. When it fails, an interactive process is already that user, so its
/// token is the console user. A service account is never substituted.
/// The Lock Host is this install when the image and console session match
/// and its token is the console user or LocalSystem. The service launches
/// the host as LocalSystem in the console session. Any other token is a
/// different process that happens to share the executable name.
pub fn host_identity(
    path_matches: bool,
    session_matches: bool,
    host_sid: Option<&str>,
    console_sid: Option<&str>,
) -> HostPresence {
    if !path_matches {
        return HostPresence::Dead;
    }
    match (host_sid, console_sid) {
        (Some(host), Some(console))
            if session_matches && (host == console || host == "S-1-5-18") =>
        {
            HostPresence::Alive
        }
        (Some(_), Some(_)) => HostPresence::Dead,
        _ => HostPresence::Unknown,
    }
}

pub fn console_user_from<'a>(
    queried: Option<&'a str>,
    process: Option<&'a str>,
) -> Result<&'a str, JournalFault> {
    if let Some(sid) = queried {
        return if sid_is_safe(sid) {
            Ok(sid)
        } else {
            Err(JournalFault::Sid)
        };
    }
    match process {
        Some(sid) if sid_is_safe(sid) => Ok(sid),
        _ => Err(JournalFault::Sid),
    }
}

pub fn sid_is_safe(sid: &str) -> bool {
    let Some(rest) = sid.strip_prefix("S-1-5-21-") else {
        return false;
    };
    if sid.len() >= 96 {
        return false;
    }
    let parts: Vec<&str> = rest.split('-').collect();
    parts.len() >= 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Expand only `%SystemRoot%` and `%SystemDrive%`. Any other variable, `..`, or a
/// non-absolute result is refused.
pub fn expand_profile_image(
    raw: &str,
    system_root: &str,
    system_drive: &str,
) -> Result<String, JournalFault> {
    if raw.is_empty()
        || raw.contains('\0')
        || raw.contains("..")
        || raw.contains('/')
        || system_root.is_empty()
        || system_root.contains('%')
        || system_root.contains("..")
        || system_drive.len() != 2
        || !system_drive.as_bytes()[0].is_ascii_alphabetic()
        || system_drive.as_bytes()[1] != b':'
    {
        return Err(JournalFault::Value);
    }
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            return Err(JournalFault::Value);
        };
        let value = match &after[..end] {
            "SystemRoot" => system_root,
            "SystemDrive" => system_drive,
            _ => return Err(JournalFault::Value),
        };
        out.push_str(value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    hive_target_allowed(&out, false, false, false)?;
    Ok(out)
}

/// A reparse point on the profile or hive file, or a REG_LINK on the key path, blocks the load.
pub fn hive_target_allowed(
    expanded: &str,
    profile_reparse: bool,
    file_reparse: bool,
    key_is_link: bool,
) -> Result<(), JournalFault> {
    if profile_reparse || file_reparse || key_is_link {
        return Err(JournalFault::Link);
    }
    let bytes = expanded.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return Err(JournalFault::Value);
    }
    if expanded.contains("..") || expanded.contains('/') || expanded.contains('%') {
        return Err(JournalFault::Value);
    }
    Ok(())
}

/// The systemprofile parent may grant SYSTEM and Administrators only.
pub fn parent_dacl_trusted(text: &str) -> bool {
    let text = text
        .trim()
        .replace("S-1-5-18", "SY")
        .replace("S-1-5-32-544", "BA");
    let mut trusted = false;
    for ace in text.split_inclusive(')') {
        let ace = ace.trim();
        if !ace.contains(";;;") {
            continue;
        }
        if ace.contains("(D;") {
            return false;
        }
        let allow = ace.ends_with(";;;SY)") || ace.ends_with(";;;BA)") || inherit_only_creator(ace);
        if allow {
            trusted = true;
        } else {
            return false;
        }
    }
    trusted
}

fn inherit_only_creator(ace: &str) -> bool {
    let mut parts = ace.trim_matches(|c| c == '(' || c == ')').split(';');
    let _kind = parts.next();
    let flags = parts.next().unwrap_or("");
    let _rights = parts.next();
    let _object = parts.next();
    let _inherited = parts.next();
    let sid = parts.next().unwrap_or("");
    sid == "CO" && flag_codes(flags).any(|code| code == "IO")
}

fn flag_codes(flags: &str) -> impl Iterator<Item = &str> {
    let mut rest = flags;
    std::iter::from_fn(move || {
        if rest.len() < 2 {
            return None;
        }
        let (code, next) = rest.split_at(2);
        rest = next;
        Some(code)
    })
}

pub fn repair_entry_name() -> String {
    format!("Repair {PRODUCT_DISPLAY_NAME} settings")
}

pub fn fault_log(fault: JournalFault) -> &'static str {
    match fault {
        JournalFault::Malformed => "malformed",
        JournalFault::Version => "version",
        JournalFault::Target => "target",
        JournalFault::Value => "value",
        JournalFault::Duplicate => "duplicate",
        JournalFault::NotDurable => "not-durable",
        JournalFault::Sid => "sid",
        JournalFault::Link => "link",
    }
}

/// Protected DACL: full control for SYSTEM and Administrators, nothing else.
pub fn journal_dacl_exact(text: &str) -> bool {
    let text = text
        .trim()
        .replace("S-1-5-18", "SY")
        .replace("S-1-5-32-544", "BA");
    // Windows reads a protected DACL back as `D:PAI`. The auto-inherit
    // marker is not an ACE. The protected bit and the two allows are.
    let Some(body) = text.strip_prefix("D:") else {
        return false;
    };
    let Some(split) = body.find('(') else {
        return false;
    };
    // Exactly `P`, then the auto-inherit pairs `AI` and `AR` in either
    // order. `D:PR` is not that: `R` by itself is not `AR`.
    if !protected_auto_inherit_flags(&body[..split]) {
        return false;
    }
    let body = &body[split..];
    if body.contains("(D;") || body.contains("ID") {
        return false;
    }
    let mut system = false;
    let mut admins = false;
    let mut count = 0usize;
    for ace in body.split_inclusive(')') {
        let ace = ace.trim();
        if ace.is_empty() {
            continue;
        }
        count += 1;
        if ace == "(A;;FA;;;SY)" {
            system = true;
        } else if ace == "(A;;FA;;;BA)" {
            admins = true;
        } else {
            return false;
        }
    }
    count == 2 && system && admins
}

fn owner_sid_ok(owner: &str) -> bool {
    if owner.is_empty() {
        return true;
    }
    if owner.len() >= 96 {
        return false;
    }
    if !owner
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return false;
    }
    owner.starts_with("S-1-") || owner == "SY" || owner == "BA"
}

fn protected_auto_inherit_flags(flags: &str) -> bool {
    let Some(rest) = flags.strip_prefix('P') else {
        return false;
    };
    let mut rest = rest;
    let mut saw_ai = false;
    let mut saw_ar = false;
    loop {
        if !saw_ai && let Some(next) = rest.strip_prefix("AI") {
            saw_ai = true;
            rest = next;
            continue;
        }
        if !saw_ar && let Some(next) = rest.strip_prefix("AR") {
            saw_ar = true;
            rest = next;
            continue;
        }
        break;
    }
    rest.is_empty()
}

pub fn dir_fault_log(fault: DirFault) -> &'static str {
    match fault {
        DirFault::Reparse => "reparse",
        DirFault::HardLink => "hardlink",
        DirFault::Planted => "planted",
        DirFault::UserOwned => "user-owned",
        DirFault::Acl => "acl",
        DirFault::Unreadable => "unreadable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "S-1-5-21-1-2-3-1001";
    const BOB: &str = "S-1-5-21-9-8-7-2002";

    #[test]
    fn absent_zero_and_admin_one_restore_exactly() {
        let absent = restore_action(&SettingValue::Absent, &SettingValue::Dword { value: 1 })
            .expect("absent");
        let zero = restore_action(
            &SettingValue::Dword { value: 0 },
            &SettingValue::Dword { value: 1 },
        )
        .expect("zero");
        let admin = restore_action(
            &SettingValue::Dword { value: 1 },
            &SettingValue::Dword { value: 1 },
        )
        .expect("admin");
        assert_eq!(absent, Some(SettingValue::Absent));
        assert_eq!(zero, Some(SettingValue::Dword { value: 0 }));
        assert_eq!(admin, None);
        assert_ne!(absent, zero);
    }

    #[test]
    fn a_second_restore_does_not_change_the_value() {
        let previous = SettingValue::Dword { value: 0 };
        let mut current = SettingValue::Dword { value: 1 };
        let first = restore_action(&previous, &current).expect("first");
        current = first.expect("write");
        let second = restore_action(&previous, &current).expect("second");
        assert_eq!(second, None);
        assert_eq!(current, previous);
        let third = restore_action(&SettingValue::Absent, &SettingValue::Absent).expect("third");
        assert_eq!(third, None);
    }

    #[test]
    fn a_disallowed_dword_is_not_restored_or_applied() {
        let err = restore_action(
            &SettingValue::Dword { value: 2 },
            &SettingValue::Dword { value: 1 },
        );
        assert_eq!(err, Err(JournalFault::Value));
        assert_eq!(
            task_manager_document(ALICE, SettingValue::Dword { value: 7 }, 0),
            Err(JournalFault::Value)
        );
    }

    #[test]
    fn the_document_rejects_unknown_targets_and_types() {
        let ok = task_manager_document(ALICE, SettingValue::Absent, 3).expect("doc");
        let bytes = encode_document(&ok).expect("encode");
        assert_eq!(parse_document(&bytes).expect("parse"), ok);
        assert!(parse_document(br#"{"v":1,"says_locked":true,"failures":0,"entries":[{"target":"NotAPolicy","previous":{"kind":"absent"},"new_value":{"kind":"dword","value":1}}]}"#).is_err());
        let legacy = br#"{"v":1,"sid":"S-1-5-21-1-2-3-1001","says_locked":true,"failures":0,"entries":[{"target":"DisableTaskMgr","previous":{"kind":"absent"},"new_value":{"kind":"dword","value":1}}]}"#;
        let parsed = parse_document(legacy).expect("ticket 16 journal");
        assert!(!parsed.workstation_clear);
        assert_eq!(parsed.entries().len(), 1);
        assert_eq!(parsed.backoff_until_tick, 0);
        assert_eq!(parsed.written_tick, 0);
        assert_eq!(parsed.written_wall_ms, 0);
        assert!(parse_document(br#"{"v":1,"says_locked":true,"failures":0,"entries":[{"target":"DisableTaskMgr","previous":{"kind":"sz","value":"1"},"new_value":{"kind":"dword","value":1}}]}"#).is_err());
        assert!(
            parse_document(br#"{"v":2,"says_locked":false,"failures":0,"entries":[]}"#).is_err()
        );
        assert!(
            parse_document(br#"{"v":1,"says_locked":true,"failures":0,"entries":[],"extra":1}"#)
                .is_err()
        );
        assert!(parse_document(b"not-json").is_err());
    }

    #[test]
    fn duplicate_entries_are_rejected() {
        let mut doc = task_manager_document(ALICE, SettingValue::Absent, 0).expect("doc");
        doc.entries.push(doc.entries[0].clone());
        assert_eq!(encode_document(&doc), Err(JournalFault::Duplicate));
    }

    #[test]
    fn a_failed_save_does_not_change_the_setting() {
        let mut changed = false;
        let result = commit_then_change(
            || Err(JournalFault::NotDurable),
            || {
                changed = true;
                Ok::<(), &str>(())
            },
        );
        assert_eq!(result, Err(CommitFault::Journal(JournalFault::NotDurable)));
        assert!(!changed);
        let mut saved = false;
        let result = commit_then_change(
            || {
                saved = true;
                Ok(())
            },
            || Err::<(), _>("apply"),
        );
        assert!(saved);
        assert_eq!(result, Err(CommitFault::Change("apply")));
    }

    #[test]
    fn an_absent_value_is_deleted_after_a_restart_mid_lock() {
        let doc = task_manager_document(ALICE, SettingValue::Absent, 0).expect("journal");
        let live = SettingValue::Dword { value: 1 };
        // A proven different boot is the reboot case. The locked dword is
        // deleted. The same boot does not take this path.
        assert_eq!(
            recovery_action(&doc, SessionFact::DifferentBoot, true, HostPresence::Dead),
            StartAction::Revert
        );
        assert_eq!(startup_steps()[0], StartupStep::RecoverJournal);
        assert_eq!(startup_steps()[1], StartupStep::LoadConfig);
        let locked = SettingValue::Dword { value: 1 };
        let previous = original_previous(Some(&doc.entries()[0].previous), &live, &locked).clone();
        assert_eq!(previous, SettingValue::Absent);
        assert_eq!(
            restore_action(&previous, &live).expect("restore"),
            Some(SettingValue::Absent)
        );
        assert_ne!(previous, SettingValue::Dword { value: 0 });
        assert_ne!(previous, live);
    }

    #[test]
    fn a_disconnect_does_not_count_as_a_new_logon() {
        let doc = task_manager_document(ALICE, SettingValue::Absent, 0).expect("journal");
        // The session id dropped and the console is not active. That is a
        // disconnect, not proof the lock ended.
        let fact = fact_for_session(Some(false), false, None);
        assert_eq!(fact, SessionFact::Unknown);
        assert_eq!(
            recovery_action(&doc, fact, false, HostPresence::Dead),
            StartAction::FailClosed
        );
        // The console is active and the old session is gone. That logon ended.
        let gone = fact_for_session(Some(false), true, None);
        assert_eq!(gone, SessionFact::SessionGone);
        assert_eq!(
            recovery_action(&doc, gone, true, HostPresence::Dead),
            StartAction::Revert
        );
        assert_eq!(
            fact_for_session(None, true, Some(true)),
            SessionFact::Unknown
        );
        assert_eq!(
            fact_for_session(Some(true), true, None),
            SessionFact::Unknown
        );
    }

    #[test]
    fn a_stale_absent_original_does_not_delete_a_newer_value() {
        let doc = task_manager_document(ALICE, SettingValue::Absent, 0).expect("journal");
        let locked = SettingValue::Dword { value: 1 };
        let live = SettingValue::Dword { value: 0 };
        // Live 0 is not the lockdown dword. It is the user's value. A stale
        // journal that still says absent must not delete it.
        assert_eq!(
            original_previous(Some(&doc.entries()[0].previous), &live, &locked),
            &live
        );
        let applied = SettingValue::Dword { value: 1 };
        assert_eq!(
            original_previous(Some(&doc.entries()[0].previous), &applied, &locked),
            &SettingValue::Absent
        );
    }

    #[test]
    fn an_os_lock_that_already_restored_stays_locked_until_the_console_returns() {
        let mut doc = SettingsJournal::empty();
        doc.sid = ALICE.to_string();
        doc.os_confirmed = true;
        assert_eq!(held_start(&doc, false), Some(HeldStart::StayOsLocked));
        assert_eq!(held_start(&doc, true), Some(HeldStart::OwnerReturned));
        doc.says_locked = true;
        assert_eq!(held_start(&doc, false), None);
        assert!(!release_os_hold(SessionFact::SameLogon));
        assert!(!release_os_hold(SessionFact::Unknown));
        assert!(release_os_hold(SessionFact::DifferentLogon));
        assert!(release_os_hold(SessionFact::SessionGone));
    }

    #[test]
    fn a_wall_clock_change_on_the_same_boot_fails_closed() {
        let doc = task_manager_document(ALICE, SettingValue::Absent, 0).expect("journal");
        let saved = 86_400_000_000_000u64;
        // `wall` is the jump a clock change would apply to BootTime. It is
        // not an input. Unbiased uptime stays put or moves forward.
        let jumps = [
            ("forward", 36_000_000_000_i64, saved + 1),
            ("backward", -36_000_000_000_i64, saved),
            ("large forward", i64::MAX, saved.saturating_add(1 << 54)),
            ("large backward", i64::MIN, saved),
        ];
        for (label, wall, now) in jumps {
            let _ = wall;
            let fact = fact_after_boot(saved, Some(now), SessionFact::SameLogon);
            assert_eq!(fact, SessionFact::SameLogon, "{label}");
            assert_eq!(
                recovery_action(&doc, fact, true, HostPresence::Dead),
                StartAction::FailClosed,
                "{label}"
            );
        }
        // The uptime query failed. That is not a reboot. The same logon, and
        // an unknown session, both stay fail-closed.
        assert_eq!(
            fact_after_boot(saved, None, SessionFact::SameLogon),
            SessionFact::SameLogon
        );
        assert_eq!(
            recovery_action(
                &doc,
                fact_after_boot(saved, None, SessionFact::Unknown),
                false,
                HostPresence::Dead
            ),
            StartAction::FailClosed
        );
    }

    #[test]
    fn a_real_sign_in_clears_os_locked_by_the_repair_route() {
        let saved = LockIdentity {
            boot: 10,
            session: 2,
            logon: 99,
        };
        // Same logon, same session, console active: the owner reconnected.
        assert!(owner_reconnected(saved, 2, true, Some(99)));
        // Still disconnected, a different console, or a different logon.
        assert!(!owner_reconnected(saved, 2, false, Some(99)));
        assert!(!owner_reconnected(saved, 4, true, Some(99)));
        assert!(!owner_reconnected(saved, 2, true, Some(7)));
        assert!(!owner_reconnected(saved, 2, true, None));
        // The release is repair_action(OsLocked), which is the ONLOGON task.
        assert_eq!(
            sign_in_release(ProductState::OsLocked, true),
            repair_action(ProductState::OsLocked)
        );
        assert_eq!(repair_action(ProductState::OsLocked), RepairAction::Revert);
        assert_eq!(
            sign_in_release(ProductState::OsLocked, false),
            RepairAction::Refuse
        );
        // A soft lock is not ended by the console coming back.
        assert_eq!(
            sign_in_release(ProductState::Locked, true),
            RepairAction::Refuse
        );
    }

    #[test]
    fn a_real_reboot_reverts_to_unlocked() {
        let doc = task_manager_document(ALICE, SettingValue::Absent, 0).expect("journal");
        let fact = fact_after_boot(50_000_000_000, Some(10), SessionFact::SameLogon);
        assert_eq!(fact, SessionFact::DifferentBoot);
        assert_eq!(
            recovery_action(&doc, fact, true, HostPresence::Alive),
            StartAction::Revert
        );
        // Uptime that did not go backwards is not a reboot. A missing session
        // still reverts, through the session fact.
        let gone = fact_after_boot(
            50_000_000_000,
            Some(80_000_000_000),
            SessionFact::SessionGone,
        );
        assert_eq!(gone, SessionFact::SessionGone);
        assert_eq!(
            recovery_action(&doc, gone, false, HostPresence::Dead),
            StartAction::Revert
        );
    }

    #[test]
    fn a_different_boot_reverts_to_unlocked() {
        let doc = task_manager_document(ALICE, SettingValue::Absent, 0).expect("journal");
        assert_eq!(
            recovery_action(&doc, SessionFact::DifferentBoot, false, HostPresence::Dead),
            StartAction::Revert
        );
        assert_eq!(
            recovery_action(&doc, SessionFact::SessionGone, true, HostPresence::Alive),
            StartAction::Revert
        );
        assert_eq!(
            recovery_action(&doc, SessionFact::DifferentLogon, true, HostPresence::Dead),
            StartAction::Revert
        );
    }

    #[test]
    fn the_same_boot_with_a_dead_host_fails_closed() {
        let doc =
            task_manager_document(ALICE, SettingValue::Dword { value: 0 }, 0).expect("journal");
        // FailClosed is the OS-lock request. Startup does not revert, and it
        // does not report Unlocked, while this logon is still the one we locked.
        assert_eq!(
            recovery_action(&doc, SessionFact::SameLogon, false, HostPresence::Dead),
            StartAction::FailClosed
        );
    }

    #[test]
    fn a_wts_query_error_fails_closed() {
        let doc = task_manager_document(ALICE, SettingValue::Absent, 0).expect("journal");
        assert_eq!(
            recovery_action(&doc, SessionFact::Unknown, false, HostPresence::Dead),
            StartAction::FailClosed
        );
        let mut bare = doc.clone();
        bare.lock = None;
        assert_eq!(
            recovery_action(&bare, SessionFact::Unknown, true, HostPresence::Dead),
            StartAction::FailClosed
        );
    }

    #[test]
    fn an_active_console_with_a_dead_host_fails_closed() {
        let doc = task_manager_document(ALICE, SettingValue::Absent, 0).expect("journal");
        assert_eq!(
            recovery_action(&doc, SessionFact::SameLogon, true, HostPresence::Dead),
            StartAction::FailClosed
        );
        assert_eq!(
            service_start_action(&doc, true, HostPresence::Dead),
            StartAction::FailClosed
        );
    }

    #[test]
    fn startup_covers_each_session_and_host_branch() {
        let empty = SettingsJournal::empty();
        assert_eq!(
            service_start_action(&empty, true, HostPresence::Alive),
            StartAction::Idle
        );
        let mut locked = task_manager_document(ALICE, SettingValue::Absent, 0).expect("doc");
        assert_eq!(
            service_start_action(&locked, true, HostPresence::Alive),
            StartAction::FailClosed
        );
        assert_eq!(
            service_start_action(&locked, true, HostPresence::Dead),
            StartAction::FailClosed
        );
        assert_eq!(
            service_start_action(&locked, false, HostPresence::Alive),
            StartAction::FailClosed
        );
        assert_eq!(
            service_start_action(&locked, false, HostPresence::Unknown),
            StartAction::FailClosed
        );
        assert_eq!(
            service_start_action(&locked, true, HostPresence::Unknown),
            StartAction::FailClosed
        );
        locked.says_locked = false;
        assert_eq!(
            service_start_action(&locked, true, HostPresence::Alive),
            StartAction::Revert
        );
        let mut failures = SettingsJournal::empty();
        failures.failures = 4;
        assert_eq!(
            service_start_action(&failures, false, HostPresence::Dead),
            StartAction::Revert
        );
    }

    #[test]
    fn repair_refuses_while_locked_and_reverts_otherwise() {
        assert_eq!(repair_action(ProductState::Locked), RepairAction::Refuse);
        assert_eq!(repair_action(ProductState::Arming), RepairAction::Refuse);
        assert_eq!(repair_action(ProductState::Unlocking), RepairAction::Refuse);
        assert_eq!(
            repair_action(ProductState::FailingClosed),
            RepairAction::Refuse
        );
        assert_eq!(
            repair_action(ProductState::Recovering),
            RepairAction::Refuse
        );
        assert_eq!(repair_action(ProductState::Unlocked), RepairAction::Revert);
        assert_eq!(repair_action(ProductState::OsLocked), RepairAction::Revert);
    }

    #[test]
    fn reassert_starts_at_five_seconds() {
        assert!(!reassert_due(Duration::from_secs(0)));
        assert!(!reassert_due(Duration::from_millis(4_999)));
        assert!(reassert_due(Duration::from_secs(5)));
        assert!(reassert_due(Duration::from_millis(5_001)));
    }

    #[test]
    fn directory_admission_covers_each_fault() {
        let ok = DirFacts {
            owner: DirOwner::System,
            acl_exact: true,
            reparse: false,
            planted_file: false,
            hard_link: false,
        };
        assert!(admit_directory(ok).is_ok());
        assert!(
            admit_directory(DirFacts {
                owner: DirOwner::Administrators,
                ..ok
            })
            .is_ok()
        );
        assert_eq!(
            admit_directory(DirFacts {
                owner: DirOwner::User,
                ..ok
            }),
            Err(DirFault::UserOwned)
        );
        assert_eq!(
            admit_directory(DirFacts {
                acl_exact: false,
                ..ok
            }),
            Err(DirFault::Acl)
        );
        assert_eq!(
            admit_directory(DirFacts {
                reparse: true,
                ..ok
            }),
            Err(DirFault::Reparse)
        );
        assert_eq!(
            admit_directory(DirFacts {
                planted_file: true,
                ..ok
            }),
            Err(DirFault::Planted)
        );
        assert_eq!(
            admit_directory(DirFacts {
                hard_link: true,
                ..ok
            }),
            Err(DirFault::HardLink)
        );
        assert_eq!(
            admit_directory(DirFacts {
                owner: DirOwner::Unreadable,
                ..ok
            }),
            Err(DirFault::Unreadable)
        );
    }

    #[test]
    fn system_does_not_run_an_executable_from_a_user_writable_path() {
        let image_text = format!(r"C:\Program Files\{PRODUCT_DISPLAY_NAME}\softlock-service.exe");
        let image = Path::new(&image_text);
        let system = Path::new(r"C:\Windows\System32\softlock-service.exe");
        assert!(executable_may_run_as_system(image, image));
        assert!(executable_may_run_as_system(system, system));
        assert!(!executable_may_run_as_system(system, image));
        assert!(!executable_may_run_as_system(
            Path::new(r"C:\Program Files\Other\softlock-service.exe"),
            image
        ));
        assert!(!executable_may_run_as_system(
            Path::new(r"C:\Program Files\..\Users\Public\softlock-service.exe"),
            Path::new(r"C:\Program Files\..\Users\Public\softlock-service.exe")
        ));
        assert!(!executable_may_run_as_system(
            Path::new(r"C:\Users\Public\softlock-service.exe"),
            Path::new(r"C:\Users\Public\softlock-service.exe")
        ));
        assert!(!executable_may_run_as_system(
            Path::new(r"C:\Windows\Temp\softlock-service.exe"),
            Path::new(r"C:\Windows\Temp\softlock-service.exe")
        ));
        assert!(!executable_may_run_as_system(
            Path::new("softlock-service.exe"),
            image
        ));
        assert!(!executable_may_run_as_system(Path::new(""), image));
        assert_eq!(
            repair_entry_name(),
            format!("Repair {PRODUCT_DISPLAY_NAME} settings")
        );
        assert!(sid_is_safe(ALICE));
        assert!(!sid_is_safe("S-1-5-18"));
        assert!(!sid_is_safe("S-1-5-19"));
        assert!(!sid_is_safe("S-1-5-20"));
        assert!(!sid_is_safe("S-1-5-80-1"));
        assert!(!sid_is_safe("S-1-5-21-1-2-3-4\\ntuser"));
        assert!(!sid_is_safe(""));
        assert!(!sid_is_safe("S-1-"));
    }

    #[test]
    fn restore_uses_the_locked_user_not_whoever_is_on_the_console() {
        let doc = task_manager_document(ALICE, SettingValue::Dword { value: 0 }, 0).expect("doc");
        assert_eq!(restore_sid(&doc).expect("sid"), ALICE);
        assert_ne!(restore_sid(&doc).expect("sid"), BOB);
        let mut bob = doc.clone();
        bob.sid = BOB.to_string();
        assert_eq!(restore_sid(&bob).expect("bob"), BOB);
        assert_ne!(restore_sid(&bob).expect("bob"), ALICE);
    }

    #[test]
    fn a_service_account_is_not_the_console_user() {
        assert_eq!(console_user_from(Some(ALICE), Some("S-1-5-18")), Ok(ALICE));
        assert_eq!(console_user_from(None, Some(ALICE)), Ok(ALICE));
        assert_eq!(
            console_user_from(None, Some("S-1-5-18")),
            Err(JournalFault::Sid)
        );
        assert_eq!(
            console_user_from(Some("S-1-5-18"), Some(ALICE)),
            Err(JournalFault::Sid)
        );
        assert_eq!(console_user_from(None, None), Err(JournalFault::Sid));
        assert_eq!(
            host_identity(true, true, Some("S-1-5-18"), Some(ALICE)),
            HostPresence::Alive
        );
        assert_eq!(
            host_identity(true, true, Some(ALICE), Some(ALICE)),
            HostPresence::Alive
        );
        assert_eq!(
            host_identity(true, true, Some(BOB), Some(ALICE)),
            HostPresence::Dead
        );
        assert_eq!(
            host_identity(false, true, Some("S-1-5-18"), Some(ALICE)),
            HostPresence::Dead
        );
        assert_eq!(
            host_identity(true, true, None, Some(ALICE)),
            HostPresence::Unknown
        );
    }

    #[test]
    fn a_planted_hive_link_is_refused_and_only_two_variables_expand() {
        let expanded = expand_profile_image(r"%SystemDrive%\Users\Example", r"C:\Windows", "C:")
            .expect("expand");
        assert_eq!(expanded, r"C:\Users\Example");
        assert!(expand_profile_image(r"%SystemRoot%\..\Users", r"C:\Windows", "C:").is_err());
        assert!(expand_profile_image(r"%USERPROFILE%\ntuser", r"C:\Windows", "C:").is_err());
        assert_eq!(
            hive_target_allowed(r"C:\Users\Example", true, false, false),
            Err(JournalFault::Link)
        );
        assert_eq!(
            hive_target_allowed(r"C:\Users\Example", false, true, false),
            Err(JournalFault::Link)
        );
        assert_eq!(
            hive_target_allowed(r"C:\Users\Example", false, false, true),
            Err(JournalFault::Link)
        );
        assert!(hive_target_allowed(r"C:\Users\Example", false, false, false).is_ok());
        assert!(parent_dacl_trusted(
            "D:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)"
        ));
        assert!(parent_dacl_trusted(
            "D:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)(A;OICIIO;FA;;;CO)"
        ));
        assert!(!parent_dacl_trusted(
            "D:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BU)"
        ));
        assert!(!parent_dacl_trusted(
            "D:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;S-1-5-21-1-2-3-1001)"
        ));
    }

    #[test]
    fn the_journal_dacl_accepts_either_ace_order_and_refuses_anything_else() {
        assert!(journal_dacl_exact("D:P(A;;FA;;;SY)(A;;FA;;;BA)"));
        assert!(journal_dacl_exact("D:PAI(A;;FA;;;SY)(A;;FA;;;BA)"));
        assert!(journal_dacl_exact("D:P(A;;FA;;;BA)(A;;FA;;;SY)"));
        assert!(journal_dacl_exact(
            "D:P(A;;FA;;;S-1-5-18)(A;;FA;;;S-1-5-32-544)"
        ));
        assert!(!journal_dacl_exact("D:(A;;FA;;;SY)(A;;FA;;;BA)"));
        assert!(!journal_dacl_exact("D:P(A;;FA;;;SY)"));
        assert!(!journal_dacl_exact("D:P(A;;FA;;;SY)(A;;FA;;;WD)"));
        assert!(!journal_dacl_exact("D:P(A;OICI;FA;;;SY)(A;;FA;;;BA)"));
        assert!(!journal_dacl_exact("D:P(D;;FA;;;SY)(A;;FA;;;BA)"));
        assert!(!journal_dacl_exact("D:PR(A;;FA;;;SY)(A;;FA;;;BA)"));
        assert!(!journal_dacl_exact("D:PA(A;;FA;;;SY)(A;;FA;;;BA)"));
        assert!(journal_dacl_exact("D:PAR(A;;FA;;;SY)(A;;FA;;;BA)"));
        assert!(journal_dacl_exact("D:PAIAR(A;;FA;;;BA)(A;;FA;;;SY)"));
        assert!(journal_dacl_exact("D:PARAI(A;;FA;;;SY)(A;;FA;;;BA)"));
    }

    #[test]
    fn the_service_config_dir_can_be_locked() {
        let dir = r"C:\Users\owner\AppData\Local\Temp\sl-cfg";
        let file = format!(r"{dir}\config.toml");
        assert!(configured_file_matches(&file, dir));
        assert!(configured_file_matches(&format!(r"{dir}\Config.toml"), dir));
        assert!(configured_file_matches(r"C:\Temp\config.toml", r"C:\Temp"));
        assert!(!configured_file_matches(
            r"C:\Windows\System32\config.toml",
            r"C:\Windows\System32"
        ));
        assert!(!configured_file_matches(
            r"C:\Program Files\Vendor\config.toml",
            r"C:\Program Files\Vendor"
        ));
        assert!(!configured_file_matches(&file, r"C:\Temp"));
        assert!(!configured_file_matches(&file, r"C:\Users\owner\..\Temp"));
    }

    #[test]
    fn the_config_path_is_only_the_owner_profile_file() {
        let service = crate::name::SERVICE_NAME;
        let ok = format!(r"C:\Users\owner\AppData\Roaming\{service}\config.toml");
        assert!(config_path_allowed(&ok));
        assert!(!config_path_allowed(r"C:\Temp\config.toml"));
        assert!(!config_path_allowed(&format!(
            r"C:\Users\owner\AppData\Local\{service}\config.toml"
        )));
        assert!(!config_path_allowed(&format!(
            r"C:\Users\owner\AppData\Roaming\{service}\other.toml"
        )));
        assert!(!config_path_allowed(&format!(
            r"C:\Users\..\AppData\Roaming\{service}\config.toml"
        )));
    }

    #[test]
    fn inherited_group_write_aces_lose_write_with_the_owner() {
        let owner = "S-1-5-21-1-2-3-1001";
        let dacl = format!(
            "D:(A;OICIID;FA;;;S-1-1-0)(A;ID;0x1301bf;;;S-1-5-32-545)(A;ID;FA;;;S-1-5-11)(A;;FA;;;{owner})"
        );
        let locked = dacl_without_owner_write(&dacl, owner).expect("rewrite");
        assert!(locked.starts_with("D:P"), "{locked}");
        assert!(locked.contains("(A;OICI;FR;;;S-1-1-0)"), "{locked}");
        assert!(locked.contains("(A;;FR;;;S-1-5-32-545)"), "{locked}");
        assert!(locked.contains("(A;;FR;;;S-1-5-11)"), "{locked}");
        assert!(locked.contains(&format!("(A;;FR;;;{owner})")), "{locked}");
        assert!(locked.contains("(A;;FA;;;SY)"), "{locked}");
        assert!(!locked.contains("ID"), "{locked}");
    }

    #[test]
    fn an_inherited_console_user_ace_loses_write_when_system_owns_the_file() {
        let user = "S-1-5-21-1-2-3-1001";
        let dacl = format!("D:AI(A;;FA;;;SY)(A;ID;FA;;;{user})");
        let locked = dacl_without_owner_write(&dacl, user).expect("rewrite");
        assert!(locked.starts_with("D:P"), "{locked}");
        assert!(locked.contains("(A;;FA;;;SY)"), "{locked}");
        assert!(locked.contains(&format!("(A;;FR;;;{user})")), "{locked}");
        assert!(!locked.contains(&format!("FA;;;{user}")), "{locked}");
    }

    #[test]
    fn a_local_administrator_alias_loses_write() {
        let user = "S-1-5-21-1-2-3-500";
        let dacl = "D:P(A;;FA;;;SY)(A;;FA;;;LA)(A;;FA;;;WD)";
        let locked = dacl_without_owner_write(dacl, user).expect("rewrite");
        assert!(locked.starts_with("D:P"), "{locked}");
        assert!(locked.contains("(A;;FA;;;SY)"), "{locked}");
        assert!(locked.contains("(A;;FR;;;LA)"), "{locked}");
        assert!(locked.contains("(A;;FR;;;WD)"), "{locked}");
        assert!(locked.contains("(A;;FR;;;OW)"), "{locked}");
        assert!(!locked.contains("FA;;;LA"), "{locked}");
        assert!(!locked.contains(";DC;"), "{locked}");
        assert!(!locked.contains(";WD;"), "{locked}");
        let admins = "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;S-1-5-21-1-2-3-1001)";
        let directory = dacl_without_owner_write(admins, "S-1-5-21-1-2-3-1001").expect("directory");
        assert!(directory.contains("(A;;FA;;;SY)"), "{directory}");
        assert!(directory.contains("(A;;FR;;;BA)"), "{directory}");
        assert!(
            directory.contains("(A;;FR;;;S-1-5-21-1-2-3-1001)"),
            "{directory}"
        );
        assert!(directory.contains("(A;;FR;;;OW)"), "{directory}");
        assert!(!directory.contains("FA;;;BA"), "{directory}");
    }

    #[test]
    fn a_lock_is_held_only_when_the_descriptor_says_so() {
        let owner = "S-1-5-21-1-2-3-1001";
        let dacl =
            format!("D:(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;{owner})(A;;FA;;;WD)(A;;FA;;;S-1-3-0)");
        let locked = dacl_without_owner_write(&dacl, owner).expect("rewrite");
        assert!(
            !config_lock_held(&locked, owner),
            "the console user owning the file is not held: {locked}"
        );
        assert!(config_lock_held(&locked, "S-1-5-18"), "{locked}");
        assert!(config_lock_held(&locked, "S-1-5-32-544"), "{locked}");
        let canonical = locked
            .replace("SY", "S-1-5-18")
            .replace("BA", "S-1-5-32-544")
            .replace("WD", "S-1-1-0")
            .replace("OW", "S-1-3-4")
            .replace("FR", "0x120089");
        assert!(config_lock_held(&canonical, "S-1-5-18"), "{canonical}");
        let weakened = format!("{canonical}(A;;FA;;;{owner})");
        assert!(!config_lock_held(&weakened, "S-1-5-18"), "{weakened}");
        let hostile = format!("D:P(D;;WD;;;SY)(A;;FA;;;{owner})(A;;FR;;;OW)");
        assert!(
            !config_lock_held(&hostile, "S-1-5-18"),
            "a deny of WRITE_DAC for SYSTEM is not held: {hostile}"
        );
        assert!(!config_lock_held("D:P(A;;FA;;;SY)(A;;FA;;;OW)", "S-1-5-18"));
        assert!(!config_lock_held("D:P(A;;FA;;;SY)", "S-1-5-18"));
        let unprotected = locked.replacen("D:P", "D:", 1);
        assert!(!config_lock_held(&unprotected, "S-1-5-18"), "{unprotected}");
        let no_system = locked.replace("(A;;FA;;;SY)", "");
        assert!(!config_lock_held(&no_system, "S-1-5-18"), "{no_system}");
    }

    #[test]
    fn unlocked_needs_a_confirmation_line_written_during_the_wait() {
        let before = "boot\n";
        let during = "boot\nos-lock result=confirmed method=disconnect\n";
        assert!(confirmed_during_wait(
            before, during, "absent", "absent", false
        ));
        assert!(!confirmed_during_wait(
            before, before, "absent", "absent", false
        ));
        assert!(!confirmed_during_wait(
            before,
            "boot\nos-lock result=failed\n",
            "absent",
            "absent",
            false
        ));
        assert!(!confirmed_during_wait(
            during, during, "absent", "absent", false
        ));
        assert!(!confirmed_during_wait(
            "os-lock result=confirmed\n",
            "os-lock result=confirmed\n",
            "absent",
            "absent",
            false
        ));
        assert!(!confirmed_during_wait(
            "earlier os-lock result=confirmed\n",
            "os-lock result=confirmed\n",
            "absent",
            "absent",
            false
        ));
        assert!(!confirmed_during_wait(before, during, "1", "absent", false));
        assert!(!confirmed_during_wait(
            before, during, "absent", "absent", true
        ));
        assert!(!confirmed_during_wait("", "", "absent", "absent", false));
        assert_eq!(
            fresh_log(before, during),
            "os-lock result=confirmed method=disconnect\n"
        );
    }

    #[test]
    fn every_fault_has_a_log_code() {
        for fault in [
            JournalFault::Malformed,
            JournalFault::Version,
            JournalFault::Target,
            JournalFault::Value,
            JournalFault::Duplicate,
            JournalFault::NotDurable,
            JournalFault::Sid,
            JournalFault::Link,
        ] {
            assert!(!fault_log(fault).is_empty());
        }
        for fault in [
            DirFault::Reparse,
            DirFault::HardLink,
            DirFault::Planted,
            DirFault::UserOwned,
            DirFault::Acl,
            DirFault::Unreadable,
        ] {
            assert!(!dir_fault_log(fault).is_empty());
        }
    }

    #[test]
    fn the_journal_job_covers_the_hosted_kill_and_every_crate() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let text = std::fs::read_to_string(root.join(".github/workflows/settings-journal.yml"))
            .expect("settings journal workflow");
        for needle in [
            "GITHUB_ACTIONS",
            "RUNNER_ENVIRONMENT",
            "github-hosted",
            "crates/softlock-core/**",
            "crates/softlock-service/**",
            "crates/softlock-lockhost/**",
            "crates/softlock-windows/**",
            "Cargo.toml",
            "Cargo.lock",
            "live-fail-closed",
            "SOFTLOCK_JOURNAL_LIVE",
            "restore",
            "recover-active",
            "LocalSystem",
            "service.log",
            "journal-dir.txt",
            "windows-2022",
            "windows-2025",
        ] {
            assert!(
                text.contains(needle),
                "{needle} missing from the journal job"
            );
        }
    }
}
