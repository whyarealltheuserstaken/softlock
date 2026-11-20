//! Crash-safe settings journal. No operating-system calls.
//!
//! The document is written and flushed before a setting changes. Restore
//! writes the previous value back, including when that value was absent.
//! Only [`TASK_MANAGER`] is applied here, and only an allow-listed DWORD.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::name::PRODUCT_DISPLAY_NAME;

/// Re-assert the locked value on this interval, and on a registry notification.
pub const REASSERT_AFTER: Duration = Duration::from_secs(5);

/// The one policy this journal applies. Later hardening items use the same document.
pub const TASK_MANAGER: &str = "DisableTaskMgr";

pub const JOURNAL_VERSION: u32 = 1;

const LOCKED_DWORD: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SettingValue {
    #[serde(rename = "absent")]
    Absent,
    #[serde(rename = "dword")]
    Dword { value: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingEntry {
    pub target: String,
    pub previous: SettingValue,
    pub new_value: SettingValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsJournal {
    pub v: u32,
    /// The user who was locked. Restore writes this hive and no other.
    #[serde(default)]
    pub sid: String,
    pub says_locked: bool,
    pub failures: u32,
    pub entries: Vec<SettingEntry>,
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
            says_locked: false,
            failures: 0,
            entries: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && !self.says_locked && self.failures == 0
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

pub fn allowed_previous(value: SettingValue) -> bool {
    match value {
        SettingValue::Absent => true,
        SettingValue::Dword { value: 0 | 1 } => true,
        SettingValue::Dword { .. } => false,
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
    if !allowed_previous(previous) {
        return Err(JournalFault::Value);
    }
    Ok(SettingsJournal {
        v: JOURNAL_VERSION,
        sid: sid.to_string(),
        says_locked: true,
        failures,
        entries: vec![SettingEntry {
            target: TASK_MANAGER.to_string(),
            previous,
            new_value: SettingValue::Dword {
                value: LOCKED_DWORD,
            },
        }],
    })
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
        if entry.target != TASK_MANAGER {
            return Err(JournalFault::Target);
        }
        if !seen.insert(entry.target.clone()) {
            return Err(JournalFault::Duplicate);
        }
        if !allowed_previous(entry.previous) {
            return Err(JournalFault::Value);
        }
        if entry.new_value
            != (SettingValue::Dword {
                value: LOCKED_DWORD,
            })
        {
            return Err(JournalFault::Value);
        }
    }
    Ok(())
}

/// `Ok(None)` means the current value already matches and the write is skipped.
pub fn restore_action(
    previous: SettingValue,
    current: SettingValue,
) -> Result<Option<SettingValue>, JournalFault> {
    if !allowed_previous(previous) {
        return Err(JournalFault::Value);
    }
    if current == previous {
        Ok(None)
    } else {
        Ok(Some(previous))
    }
}

pub fn reassert_due(elapsed: Duration) -> bool {
    elapsed >= REASSERT_AFTER
}

/// Save first. A failed save does not run `change`.
pub fn commit_then_change<E>(
    save: impl FnOnce() -> Result<(), JournalFault>,
    change: impl FnOnce() -> Result<(), E>,
) -> Result<(), CommitFault<E>> {
    save().map_err(CommitFault::Journal)?;
    change().map_err(CommitFault::Change)
}

/// A live Lock Host in an active console session is still locked.
/// A dead host is not an active lock, so startup reverts the journal.
pub fn service_start_action(
    doc: &SettingsJournal,
    console_active: bool,
    host: HostPresence,
) -> StartAction {
    if doc.is_empty() {
        return StartAction::Idle;
    }
    let live = doc.says_locked
        && (host == HostPresence::Unknown || (console_active && host == HostPresence::Alive));
    if live {
        StartAction::FailClosed
    } else {
        StartAction::Revert
    }
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
    let Some(body) = text.strip_prefix("D:P") else {
        return false;
    };
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
        let absent =
            restore_action(SettingValue::Absent, SettingValue::Dword { value: 1 }).expect("absent");
        let zero = restore_action(
            SettingValue::Dword { value: 0 },
            SettingValue::Dword { value: 1 },
        )
        .expect("zero");
        let admin = restore_action(
            SettingValue::Dword { value: 1 },
            SettingValue::Dword { value: 1 },
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
        let first = restore_action(previous, current).expect("first");
        current = first.expect("write");
        let second = restore_action(previous, current).expect("second");
        assert_eq!(second, None);
        assert_eq!(current, previous);
        let third = restore_action(SettingValue::Absent, SettingValue::Absent).expect("third");
        assert_eq!(third, None);
    }

    #[test]
    fn a_disallowed_dword_is_not_restored_or_applied() {
        let err = restore_action(
            SettingValue::Dword { value: 2 },
            SettingValue::Dword { value: 1 },
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
        assert!(parse_document(br#"{"v":1,"says_locked":true,"failures":0,"entries":[{"target":"NoClose","previous":{"kind":"absent"},"new_value":{"kind":"dword","value":1}}]}"#).is_err());
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
            StartAction::Revert
        );
        assert_eq!(
            service_start_action(&locked, false, HostPresence::Alive),
            StartAction::Revert
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
        assert!(journal_dacl_exact("D:P(A;;FA;;;BA)(A;;FA;;;SY)"));
        assert!(journal_dacl_exact(
            "D:P(A;;FA;;;S-1-5-18)(A;;FA;;;S-1-5-32-544)"
        ));
        assert!(!journal_dacl_exact("D:(A;;FA;;;SY)(A;;FA;;;BA)"));
        assert!(!journal_dacl_exact("D:P(A;;FA;;;SY)"));
        assert!(!journal_dacl_exact("D:P(A;;FA;;;SY)(A;;FA;;;WD)"));
        assert!(!journal_dacl_exact("D:P(A;OICI;FA;;;SY)(A;;FA;;;BA)"));
        assert!(!journal_dacl_exact("D:P(D;;FA;;;SY)(A;;FA;;;BA)"));
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
