//! Apply the lockdown rows only after the journal is durable, and put each one back exactly.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use softlock_core::PRODUCT_DISPLAY_NAME;
use softlock_core::{
    BackoffRecord, BackoffRestore, CONFIG_DIR, CONFIG_FILE, ClockSample, CommitFault, HardenError,
    HeldStart, Journal, JournalEntry, JournalFault, LOCKDOWN, ProductState, RepairAction,
    SERVICE_NAME, SettingValue, SettingsJournal, StartAction, allowed_previous_for,
    commit_then_change, encode_document, executable_may_run_as_system, fault_log, held_start,
    lockdown_document, lockdown_policy, mark_workstation_clear, original_previous, parse_document,
    reassert_write, recovery_action, release_os_hold, repair_action, repair_entry_name,
    restore_action_for, restore_backoff, restore_sid, service_start_action, with_config_acl,
    workstation_value_after_attempt,
};

use crate::journal_store::{load_journal, save_journal};
use crate::taskmgr::{
    console_is_active, console_sid, current_lock_identity, lock_host_running, observe_lock,
    read_lockdown, watch_policy, write_lockdown,
};

static SKIP_WORKSTATION: AtomicBool = AtomicBool::new(false);
static RESTORED: AtomicBool = AtomicBool::new(false);

pub enum Boot {
    Idle,
    Reverted,
    FailClosed(Journal),
    /// Policies are already restored and the console is not active.
    StayOsLocked,
}

#[cfg(test)]
static OPEN_ERROR: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(crate) fn fail_open_for_test(fail: bool) {
    OPEN_ERROR.store(fail, Ordering::Release);
}

pub struct SettingsEngine {
    sid: String,
    pending: Option<SettingsJournal>,
    holding: Arc<AtomicBool>,
    notified: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    last: Instant,
    watcher: Mutex<Option<JoinHandle<()>>>,
    keep_os_lock: bool,
    /// Password failures that still count toward Windows lockout. Revert keeps
    /// them. A successful password check clears them.
    password_attempts: Vec<softlock_core::PasswordStamp>,
}

impl SettingsEngine {
    pub fn open() -> Result<Self, &'static str> {
        #[cfg(test)]
        if OPEN_ERROR.load(Ordering::Acquire) {
            return Err("journal");
        }
        // Opening the directory refuses a user-owned or pre-created folder.
        load_journal().map_err(|_| "journal")?;
        Ok(Self {
            sid: String::new(),
            pending: None,
            holding: Arc::new(AtomicBool::new(false)),
            notified: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            last: Instant::now(),
            watcher: Mutex::new(None),
            keep_os_lock: false,
            password_attempts: Vec::new(),
        })
    }

    pub fn keep_os_lock_after_revert(&mut self, keep: bool) {
        self.keep_os_lock = keep;
    }

    pub fn password_attempts(&self) -> Vec<softlock_core::PasswordStamp> {
        self.password_attempts.clone()
    }

    /// Rewrite the password-attempt list. Lockdown rows stay. A missing file
    /// keeps the list in memory for the document the next lock writes.
    pub fn store_password_attempts(
        &mut self,
        attempts: &[softlock_core::PasswordStamp],
    ) -> Result<(), &'static str> {
        self.password_attempts = attempts.to_vec();
        if let Some(pending) = self.pending.as_mut() {
            pending.password_attempts = self.password_attempts.clone();
        }
        let Some(text) = load_journal().map_err(|_| "unreadable")? else {
            return Ok(());
        };
        let mut doc = parse_document(text.as_bytes()).map_err(|_| "malformed")?;
        doc.password_attempts = self.password_attempts.clone();
        let bytes = encode_document(&doc).map_err(|_| "malformed")?;
        save_journal(&bytes).map_err(|_| "not-durable")?;
        Ok(())
    }

    fn remember_password(&mut self, doc: &SettingsJournal) {
        self.password_attempts = doc.password_attempts.clone();
    }

    pub fn boot(&mut self) -> Result<Boot, &'static str> {
        let loaded = load_journal().map_err(|_| "unreadable")?;
        let Some(text) = loaded else {
            return Ok(Boot::Idle);
        };
        let doc = parse_document(text.as_bytes()).map_err(|_| "malformed")?;
        self.remember_password(&doc);
        if doc.is_empty() {
            return Ok(Boot::Idle);
        }
        let console = console_is_active();
        let console_active = console.unwrap_or(false);
        if let Some(held) = held_start(&doc, console_active) {
            return match held {
                HeldStart::OwnerReturned => {
                    // An active console with the same logon is the desktop
                    // we disconnected, not a new sign-in. Stay OsLocked.
                    let fact = observe_lock(doc.lock, true);
                    if !release_os_hold(fact) {
                        return Ok(Boot::StayOsLocked);
                    }
                    let mut cleared = SettingsJournal::empty();
                    cleared.password_attempts = self.password_attempts.clone();
                    let empty = encode_document(&cleared).map_err(fault_log)?;
                    save_journal(&empty).map_err(|_| "not-durable")?;
                    Ok(Boot::Idle)
                }
                HeldStart::StayOsLocked => Ok(Boot::StayOsLocked),
            };
        }
        self.sid = restore_sid(&doc).map_err(fault_log)?.to_string();
        // A failed WTS query comes back as SessionFact::Unknown and fails
        // closed. A proven different boot, a missing session, or a different
        // logon is the only revert. SPEC Decision 5 and section 7.
        let mut fact = observe_lock(doc.lock, console_active);
        if console.is_err() && fact == softlock_core::SessionFact::SameLogon {
            fact = softlock_core::SessionFact::Unknown;
        }
        match recovery_action(&doc, fact, console.unwrap_or(false), lock_host_running()) {
            StartAction::Idle => Ok(Boot::Idle),
            StartAction::Revert => {
                self.revert_document(&doc)?;
                Ok(Boot::Reverted)
            }
            StartAction::FailClosed => {
                // Hold the rows until the OS lock is confirmed. The controller
                // reverts then, from this journal. SPEC section 7.
                self.hold();
                if doc.workstation_clear {
                    reapply_disable_lock_workstation()?;
                }
                Ok(Boot::FailClosed(memory_journal(&doc)))
            }
            StartAction::StayOsLocked => Ok(Boot::StayOsLocked),
        }
    }

    pub fn stage(&mut self, journal: &mut Journal) -> Result<(), HardenError> {
        let sid = console_sid().map_err(|reason| {
            crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
            HardenError::Failed
        })?;
        self.sid = sid;
        // A live lockdown dword keeps the journaled previous. Any other live
        // value is the user's, so a stale absent original cannot delete it.
        let durable = load_journal().ok().flatten().and_then(|text| {
            parse_document(text.as_bytes())
                .ok()
                .filter(|doc| doc.says_locked())
        });
        let mut previous = Vec::with_capacity(LOCKDOWN.len());
        for policy in LOCKDOWN {
            let live = read_lockdown(&self.sid, policy).map_err(|reason| {
                crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
                HardenError::Failed
            })?;
            let journaled = durable.as_ref().and_then(|doc| {
                doc.entries()
                    .iter()
                    .find(|entry| entry.target == policy.target)
                    .map(|entry| &entry.previous)
            });
            let locked = SettingValue::Dword {
                value: policy.locked,
            };
            previous.push((
                policy.target,
                original_previous(journaled, &live, &locked).clone(),
            ));
        }
        let mut doc =
            lockdown_document(&self.sid, &previous, journal.failures()).map_err(|fault| {
                crate::audit(
                    "settings",
                    &[("result", "failed"), ("reason", fault_log(fault))],
                );
                HardenError::Failed
            })?;
        doc.lock = current_lock_identity();
        // The seam service runs as the console user. A protected DACL on its
        // private file needs a privilege that user does not have, and a failed
        // row fail-closes the lock. SOFTLOCK_LOCK_CONFIG_ACL=1 is the decline
        // proof, which does take that lock. The hosted LocalSystem job covers
        // the real profile file.
        if lock_config_acl() {
            let path = crate::config_file::configured_path().map_err(|reason| {
                crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
                HardenError::Failed
            })?;
            let entry = crate::config_file::prepare_entry(&path).map_err(|reason| {
                crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
                HardenError::Failed
            })?;
            let directory = crate::config_file::prepare_dir_entry(&path).map_err(|reason| {
                crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
                HardenError::Failed
            })?;
            doc = with_config_acl(doc, entry).map_err(|fault| {
                crate::audit(
                    "settings",
                    &[("result", "failed"), ("reason", fault_log(fault))],
                );
                HardenError::Failed
            })?;
            doc = with_config_acl(doc, directory).map_err(|fault| {
                crate::audit(
                    "settings",
                    &[("result", "failed"), ("reason", fault_log(fault))],
                );
                HardenError::Failed
            })?;
        }
        doc.password_attempts = self.password_attempts.clone();
        self.pending = Some(doc);
        for (target, value) in &previous {
            let locked = lockdown_policy(target)
                .map(|policy| policy.locked)
                .unwrap_or(1);
            journal.record(JournalEntry {
                target: (*target).to_string(),
                previous: previous_text(value),
                new_value: locked.to_string(),
            });
        }
        Ok(())
    }

    pub fn persist_and_apply(&mut self) -> Result<(), HardenError> {
        let Some(doc) = self.pending.clone() else {
            crate::audit("settings", &[("result", "failed"), ("reason", "journal")]);
            return Err(HardenError::Failed);
        };
        let bytes = encode_document(&doc).map_err(|fault| {
            crate::audit(
                "settings",
                &[("result", "failed"), ("reason", fault_log(fault))],
            );
            HardenError::Failed
        })?;
        let sid = self.sid.clone();
        RESTORED.store(false, Ordering::Release);
        let saved = commit_then_change(
            || save_journal(&bytes).map_err(|_| JournalFault::NotDurable),
            || {
                // Killing the service in either pause leaves the journal durable.
                // An OS-lock confirmation puts DisableLockWorkstation back to
                // its locked value. The owner's previous values stay in the
                // journal until unlock.
                pause_for("before");
                for entry in doc.entries() {
                    if entry.target == CONFIG_FILE || entry.target == CONFIG_DIR {
                        continue;
                    }
                    let Some(policy) = lockdown_policy(&entry.target) else {
                        return Err("target");
                    };
                    write_lockdown(&sid, policy, entry.new_value.clone())?;
                }
                for entry in doc.entries() {
                    if entry.target == CONFIG_FILE || entry.target == CONFIG_DIR {
                        crate::config_file::apply_entry(entry)?;
                    }
                }
                pause_for("after");
                Ok(())
            },
        );
        match saved {
            Ok(()) => {
                self.hold();
                crate::audit("settings", &[("result", "applied"), ("rows", "lockdown")]);
                Ok(())
            }
            Err(CommitFault::Journal(fault)) => {
                crate::audit(
                    "settings",
                    &[("result", "failed"), ("reason", fault_log(fault))],
                );
                Err(HardenError::Failed)
            }
            Err(CommitFault::Change(reason)) => {
                crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
                Err(HardenError::Failed)
            }
        }
    }

    pub fn revert_memory(&mut self, journal: &Journal) -> Result<(), &'static str> {
        // Called when the OS lock is confirmed, or when the owner unlocks.
        // SPEC section 7 reverts the journal at confirmation, not before.
        // Confirmation keeps an OsLocked marker. An owner unlock clears it.
        if let Some(text) = load_journal().map_err(|_| "journal")? {
            let doc = parse_document(text.as_bytes()).map_err(fault_log)?;
            return self.revert_document(&doc);
        }
        for entry in journal.entries() {
            if lockdown_policy(&entry.target).is_none() {
                continue;
            }
            let previous = match entry.previous.as_deref() {
                None => SettingValue::Absent,
                Some(text) => {
                    let Ok(value) = text.parse::<u32>() else {
                        return Err("value");
                    };
                    SettingValue::Dword { value }
                }
            };
            if !allowed_previous_for(&entry.target, &previous) {
                return Err("value");
            }
            self.write_previous(&entry.target, previous)?;
        }
        self.release();
        Ok(())
    }

    /// `Some(true)` while the confirmed-disconnect marker is still on disk.
    /// `Some(false)` once [`Self::repair_leftover`] has removed it. `None`
    /// means the journal could not be read, which is not a sign-in.
    pub fn os_confirmed_marker(&mut self) -> Option<bool> {
        let text = load_journal().ok()??;
        let doc = parse_document(text.as_bytes()).ok()?;
        Some(doc.os_confirmed)
    }

    /// The confirmed disconnect's logon is the console again, and that
    /// console is active. This is the read [`Self::repair_leftover`] uses
    /// before it writes.
    pub fn owner_back(&mut self) -> bool {
        let Some(text) = load_journal().ok().flatten() else {
            return false;
        };
        let Ok(doc) = parse_document(text.as_bytes()) else {
            return false;
        };
        doc.os_confirmed && doc.lock.is_some_and(crate::taskmgr::owner_session_returned)
    }

    pub fn repair_leftover(&mut self) -> Result<(), &'static str> {
        let Some(text) = load_journal().map_err(|_| "journal")? else {
            return Ok(());
        };
        let doc = parse_document(text.as_bytes()).map_err(fault_log)?;
        if doc.is_empty() {
            return Ok(());
        }
        // A confirmed disconnect is cleared only when this same logon is the
        // console again. The running service calls this on the logon or
        // unlock that brings the owner back, and the ONLOGON task calls it
        // when the service is not the process that sees the event. A session
        // that merely looks active before it has left is not a sign-in.
        if doc.os_confirmed {
            let returned = doc.lock.is_some_and(crate::taskmgr::owner_session_returned);
            if softlock_core::sign_in_release(softlock_core::ProductState::OsLocked, returned)
                != softlock_core::RepairAction::Revert
            {
                return Err("session");
            }
        }
        self.sid = restore_sid(&doc).map_err(fault_log)?.to_string();
        self.revert_document(&doc)
    }

    pub fn reassert_now(&mut self) -> Result<(), &'static str> {
        if RESTORED.load(Ordering::Acquire) {
            return Ok(());
        }
        // A failed row must not spin. The next attempt waits the normal
        // interval even when this one returns an error before the end.
        self.last = Instant::now();
        let Some(text) = load_journal().map_err(|_| "journal")? else {
            return Ok(());
        };
        let doc = parse_document(text.as_bytes()).map_err(fault_log)?;
        let skip_workstation = SKIP_WORKSTATION.load(Ordering::Acquire) || doc.workstation_clear;
        for entry in doc.entries() {
            if entry.target == CONFIG_FILE || entry.target == CONFIG_DIR {
                crate::config_file::reassert_entry(entry)?;
                continue;
            }
            let Some(policy) = lockdown_policy(&entry.target) else {
                return Err("target");
            };
            let current = read_lockdown(&self.sid, policy)?;
            let skip = skip_workstation && entry.target == "DisableLockWorkstation";
            if let Some(next) = reassert_write(&entry.target, &entry.new_value, &current, skip) {
                write_lockdown(&self.sid, policy, next)?;
                crate::audit(
                    "settings",
                    &[("result", "reasserted"), ("target", policy.target)],
                );
            }
        }
        Ok(())
    }

    /// Wall-clock re-assert. The controller clock does not advance in the service.
    pub fn reassert_if_due(&mut self) -> Result<(), &'static str> {
        if RESTORED.load(Ordering::Acquire) || !self.holding.load(Ordering::Acquire) {
            return Ok(());
        }
        let notified = self.notified.swap(false, Ordering::AcqRel);
        if !notified && self.last.elapsed() < Duration::from_secs(5) {
            return Ok(());
        }
        self.reassert_now()
    }

    fn write_previous(&mut self, target: &str, previous: SettingValue) -> Result<(), &'static str> {
        let Some(policy) = lockdown_policy(target) else {
            return Err("target");
        };
        let current = read_lockdown(&self.sid, policy)?;
        match restore_action_for(target, &previous, &current).map_err(|_| "value")? {
            Some(next) => write_lockdown(&self.sid, policy, next),
            None => Ok(()),
        }
    }

    fn revert_document(&mut self, doc: &SettingsJournal) -> Result<(), &'static str> {
        RESTORED.store(true, Ordering::Release);
        for entry in doc.entries() {
            if entry.target == CONFIG_FILE || entry.target == CONFIG_DIR {
                crate::config_file::restore_entry(entry)?;
                continue;
            }
            if lockdown_policy(&entry.target).is_none() {
                return Err("target");
            }
            self.write_previous(&entry.target, entry.previous.clone())?;
        }
        let mut next = if self.keep_os_lock {
            let mut held = SettingsJournal::empty();
            held.sid = doc.sid.clone();
            held.os_confirmed = true;
            held.lock = doc.lock;
            held
        } else {
            SettingsJournal::empty()
        };
        next.password_attempts = doc.password_attempts.clone();
        self.password_attempts = next.password_attempts.clone();
        let empty = encode_document(&next).map_err(|_| "malformed")?;
        save_journal(&empty).map_err(|_| "not-durable")?;
        SKIP_WORKSTATION.store(false, Ordering::Release);
        self.release();
        crate::audit("settings", &[("result", "restored"), ("rows", "lockdown")]);
        Ok(())
    }

    /// Rewrite the failure count and the backoff deadline. The lockdown rows
    /// stay. A failed write leaves the in-memory count in place.
    pub fn store_backoff(
        &mut self,
        failures: u32,
        until_tick: u64,
        written_tick: u64,
        written_wall_ms: u64,
    ) -> Result<(), &'static str> {
        let Some(text) = load_journal().map_err(|_| "unreadable")? else {
            return Err("unreadable");
        };
        let mut doc = parse_document(text.as_bytes()).map_err(|_| "malformed")?;
        doc.failures = failures;
        doc.backoff_until_tick = until_tick;
        doc.written_tick = written_tick;
        doc.written_wall_ms = written_wall_ms;
        let bytes = encode_document(&doc).map_err(|_| "malformed")?;
        save_journal(&bytes).map_err(|_| "not-durable")?;
        Ok(())
    }

    fn hold(&mut self) {
        self.holding.store(true, Ordering::Release);
        self.last = Instant::now();
        self.stop.store(false, Ordering::Release);
        self.start_watcher();
    }

    fn release(&mut self) {
        self.holding.store(false, Ordering::Release);
        self.stop.store(true, Ordering::Release);
    }

    fn start_watcher(&mut self) {
        let mut slot = self
            .watcher
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if slot.is_some() {
            return;
        }
        let stop = Arc::clone(&self.stop);
        let notified = Arc::clone(&self.notified);
        let sid = self.sid.clone();
        *slot = Some(thread::spawn(move || watch_policy(&sid, &stop, &notified)));
    }
}

fn lock_config_acl() -> bool {
    if crate::config_file::test_config_dir().is_none() {
        return true;
    }
    std::env::var("SOFTLOCK_LOCK_CONFIG_ACL").ok().as_deref() == Some("1")
}

fn previous_text(value: &SettingValue) -> Option<String> {
    match value {
        SettingValue::Absent => None,
        SettingValue::Dword { value } => Some(value.to_string()),
        SettingValue::Sddl { .. } => None,
    }
}

fn memory_journal(doc: &SettingsJournal) -> Journal {
    let mut journal = Journal::empty();
    for entry in doc.entries() {
        journal.record(JournalEntry {
            target: entry.target.clone(),
            previous: previous_text(&entry.previous),
            new_value: match &entry.new_value {
                SettingValue::Dword { value } => value.to_string(),
                SettingValue::Absent => "0".to_string(),
                SettingValue::Sddl { value } => value.clone(),
            },
        });
    }
    journal.set_failures(doc.failures());
    let sample = clock_sample();
    let wait = match restore_backoff(
        BackoffRecord {
            failures: doc.failures(),
            written_tick: doc.written_tick,
            until_tick: doc.backoff_until_tick,
            written_wall_ms: doc.written_wall_ms,
        },
        sample,
    ) {
        BackoffRestore::Clear => 0,
        BackoffRestore::Wait(delay) => u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
    };
    journal.set_wait_ms(wait);
    if doc.says_locked() {
        journal.mark_locked();
    }
    journal.flush();
    journal
}

fn clock_sample() -> ClockSample {
    use std::time::{SystemTime, UNIX_EPOCH};
    use windows::Win32::System::SystemInformation::GetTickCount64;
    let wall_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    ClockSample {
        tick: unsafe { GetTickCount64() },
        wall_ms,
    }
}

/// Journal the clear, then delete `DisableLockWorkstation`. A failed save
/// does not change the value. Disconnect-only callers do not use this.
pub fn clear_disable_lock_workstation_for_os_lock() -> Result<bool, &'static str> {
    let Some(text) = load_journal().map_err(|_| "journal")? else {
        return Ok(false);
    };
    let doc = parse_document(text.as_bytes()).map_err(fault_log)?;
    if doc.is_empty()
        || !doc
            .entries()
            .iter()
            .any(|entry| entry.target == "DisableLockWorkstation")
    {
        return Ok(false);
    }
    let sid = restore_sid(&doc).map_err(fault_log)?.to_string();
    let policy = lockdown_policy("DisableLockWorkstation").ok_or("target")?;
    let current = read_lockdown(&sid, policy)?;
    if doc.workstation_clear
        && current
            != (SettingValue::Dword {
                value: policy.locked,
            })
    {
        SKIP_WORKSTATION.store(true, Ordering::Release);
        return Ok(true);
    }
    let marked = mark_workstation_clear(&doc).map_err(fault_log)?;
    let bytes = encode_document(&marked).map_err(fault_log)?;
    save_journal(&bytes).map_err(|_| "not-durable")?;
    SKIP_WORKSTATION.store(true, Ordering::Release);
    write_lockdown(&sid, policy, SettingValue::Absent)?;
    Ok(true)
}

/// Put `DisableLockWorkstation` back to its locked value and clear the
/// temporary-clear flag before the registry write, so a crash cannot leave
/// re-assert skipping an absent value. The owner's previous value stays.
pub fn reapply_disable_lock_workstation() -> Result<(), &'static str> {
    SKIP_WORKSTATION.store(false, Ordering::Release);
    let Some(text) = load_journal().map_err(|_| "journal")? else {
        return Ok(());
    };
    let doc = parse_document(text.as_bytes()).map_err(fault_log)?;
    let Some((settled, value)) = workstation_value_after_attempt(&doc).map_err(fault_log)? else {
        return Ok(());
    };
    let bytes = encode_document(&settled).map_err(fault_log)?;
    save_journal(&bytes).map_err(|_| "not-durable")?;
    let sid = restore_sid(&settled).map_err(fault_log)?.to_string();
    let policy = lockdown_policy("DisableLockWorkstation").ok_or("target")?;
    write_lockdown(&sid, policy, value)
}

pub fn repair_main() -> i32 {
    let state = match crate::windows::running_product_state() {
        None => ProductState::Unlocked,
        Some(state) => state,
    };
    if repair_action(state) == RepairAction::Refuse {
        crate::audit("settings", &[("result", "refused"), ("reason", "locked")]);
        eprintln!("repair refused while {PRODUCT_DISPLAY_NAME} is locked");
        return 2;
    }
    let mut engine = match SettingsEngine::open() {
        Ok(engine) => engine,
        Err(reason) => {
            crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
            eprintln!("repair failed: {reason}");
            return 1;
        }
    };
    match engine.repair_leftover() {
        Ok(()) => 0,
        Err(reason) => {
            crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
            eprintln!("repair failed: {reason}");
            1
        }
    }
}

pub fn register_sign_in_recovery() {
    let Ok(exe) = std::env::current_exe() else {
        crate::audit("settings", &[("result", "failed"), ("reason", "exe")]);
        return;
    };
    if !command_path_allowed(&exe) {
        crate::audit(
            "settings",
            &[("result", "skipped"), ("reason", "untrusted-exe")],
        );
        return;
    }
    let task = format!("{SERVICE_NAME}-restore");
    let command = format!("\"{}\" --repair-settings", exe.display());
    let mut created_task = Command::new(r"C:\Windows\System32\schtasks.exe");
    created_task.args([
        "/Create", "/F", "/TN", &task, "/SC", "ONLOGON", "/RU", "SYSTEM", "/RL", "HIGHEST", "/TR",
        &command,
    ]);
    let created = command_status(&mut created_task);
    if !created {
        crate::audit(
            "settings",
            &[("result", "failed"), ("reason", "task-create")],
        );
        return;
    }
    let mut query_task = Command::new(r"C:\Windows\System32\schtasks.exe");
    query_task.args(["/Query", "/TN", &task]);
    let present = command_status(&mut query_task);
    if !present {
        crate::audit(
            "settings",
            &[("result", "failed"), ("reason", "task-missing")],
        );
        return;
    }
    crate::audit("settings", &[("result", "task")]);
    if let Err(reason) = write_start_menu(&exe) {
        crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
    }
}

fn command_status(command: &mut Command) -> bool {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return false,
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if started.elapsed() < Duration::from_secs(15) => {
                thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.try_wait();
                return false;
            }
        }
    }
}

fn write_start_menu(exe: &Path) -> Result<(), &'static str> {
    let programs = PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs");
    // `is_dir` follows a junction. A reparse point here is not the Start Menu.
    if !menu_directory_is_plain(&programs) {
        return Err("menu");
    }
    let path = programs.join(format!("{}.cmd", repair_entry_name()));
    let body = format!("@echo off\r\n\"{}\" --repair-settings\r\n", exe.display());
    write_menu_new(&path, body.as_bytes())
}

fn command_path_allowed(path: &Path) -> bool {
    let text = path.to_string_lossy();
    if !executable_may_run_as_system(path, path)
        || text
            .chars()
            .any(|c| matches!(c, '&' | '|' | '>' | '<' | '%' | '!' | '\n' | '\r' | '"'))
    {
        return false;
    }
    image_has_no_reparse(path)
}

/// Every component, including the image, is opened with
/// `FILE_FLAG_OPEN_REPARSE_POINT`. A junction or symlink under the trusted
/// prefix is refused, and the image open does not follow one.
fn image_has_no_reparse(path: &Path) -> bool {
    let mut built = PathBuf::new();
    let mut saw_root = false;
    for component in path.components() {
        match component {
            std::path::Component::Prefix(_) => {
                built.push(component.as_os_str());
            }
            std::path::Component::RootDir => {
                built.push(component.as_os_str());
                saw_root = true;
                if !opened_without_reparse(&built) {
                    return false;
                }
            }
            std::path::Component::Normal(part) => {
                built.push(part);
                if !opened_without_reparse(&built) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    saw_root
}

fn opened_without_reparse(path: &Path) -> bool {
    matches!(crate::secrets::is_reparse_point(path), Ok(false))
}

fn menu_directory_is_plain(path: &Path) -> bool {
    matches!(crate::secrets::is_reparse_point(path), Ok(false))
}

#[cfg(windows)]
fn write_menu_new(path: &Path, body: &[u8]) -> Result<(), &'static str> {
    use std::os::windows::io::FromRawHandle;
    use windows::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_GENERIC_READ, FILE_GENERIC_WRITE, OPEN_EXISTING,
    };
    use windows::core::PCWSTR;
    let wide = crate::secrets::wide_str(&path.display().to_string());
    let existing = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            windows::Win32::Storage::FileSystem::FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    };
    match existing {
        Ok(handle) => {
            let trusted = crate::secrets::owner_is_trusted(handle).unwrap_or(false);
            let reparse = crate::secrets::handle_is_reparse(handle).unwrap_or(true);
            if reparse || !trusted {
                crate::secrets::drop_handle(handle);
                return Err("menu-owner");
            }
            let mut file = unsafe { std::fs::File::from_raw_handle(handle.0) };
            file.set_len(0).map_err(|_| "menu")?;
            std::io::Write::write_all(&mut file, body).map_err(|_| "menu")?;
            return Ok(());
        }
        Err(error) => {
            if !crate::secrets::not_found(&crate::secrets::os_error(error)) {
                return Err("menu");
            }
        }
    }
    let descriptor = crate::secrets::security_descriptor_from("D:P(A;;FA;;;SY)(A;;FA;;;BA)")
        .map_err(|_| "menu")?;
    let sa = windows::Win32::Security::SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<windows::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let created = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(0),
            Some(&sa),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    };
    crate::secrets::free_descriptor(&descriptor);
    let handle = created.map_err(|_| "menu")?;
    if crate::secrets::handle_is_reparse(handle).unwrap_or(true)
        || !crate::secrets::owner_is_trusted(handle).unwrap_or(false)
    {
        crate::secrets::drop_handle(handle);
        return Err("menu-owner");
    }
    let mut file = unsafe { std::fs::File::from_raw_handle(handle.0) };
    std::io::Write::write_all(&mut file, body).map_err(|_| "menu")?;
    Ok(())
}

#[cfg(not(windows))]
fn write_menu_new(_path: &Path, _body: &[u8]) -> Result<(), &'static str> {
    Err("menu")
}

#[cfg(feature = "live-fail-closed")]
fn pause_for(phase: &str) {
    if !crate::windows::live_hooks_allowed() {
        return;
    }
    if std::env::var("SOFTLOCK_JOURNAL_PAUSE").ok().as_deref() != Some(phase) {
        return;
    }
    let path = std::env::var("SOFTLOCK_JOURNAL_PAUSE_FILE").unwrap_or_default();
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if !path.is_empty()
            && std::fs::read_to_string(&path)
                .ok()
                .is_some_and(|text| text.contains("go"))
        {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(not(feature = "live-fail-closed"))]
fn pause_for(_phase: &str) {}

#[cfg(test)]
mod path_and_menu {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{command_path_allowed, menu_directory_is_plain, write_menu_new};

    /// Both tests used to share `softlock-menu-<pid>` and delete it while the
    /// other was still creating children. Each call gets its own directory.
    fn scratch(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("softlock-{label}-{}-{n}", std::process::id()))
    }

    fn system32_name(label: &str, n: u64) -> PathBuf {
        PathBuf::from(format!(
            r"C:\Windows\System32\softlock-{label}-{}-{n}",
            std::process::id()
        ))
    }

    struct Remove<'a> {
        files: &'a [PathBuf],
        junctions: &'a [PathBuf],
        trees: &'a [PathBuf],
    }

    impl Drop for Remove<'_> {
        fn drop(&mut self) {
            for path in self.files {
                let _ = fs::remove_file(path);
            }
            // `remove_dir` on a junction does not follow it into the target.
            for path in self.junctions {
                let _ = fs::remove_dir(path);
            }
            for path in self.trees {
                let _ = fs::remove_dir_all(path);
            }
        }
    }

    fn junction(link: &Path, target: &Path) {
        let status = Command::new("cmd")
            .args([
                "/c",
                "mklink",
                "/J",
                &link.display().to_string(),
                &target.display().to_string(),
            ])
            .status()
            .expect("mklink");
        assert!(status.success(), "junction {:?} -> {:?}", link, target);
    }

    fn symlink_file(link: &Path, target: &Path) {
        let output = Command::new("cmd")
            .args([
                "/c",
                "mklink",
                &link.display().to_string(),
                &target.display().to_string(),
            ])
            .output()
            .expect("mklink");
        assert!(
            output.status.success(),
            "symlink {:?} -> {:?}: {}",
            link,
            target,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write_owned(path: &Path, body: &[u8]) {
        use std::os::windows::io::FromRawHandle;
        use windows::Win32::Storage::FileSystem::{
            CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        };
        use windows::core::PCWSTR;
        let descriptor =
            crate::secrets::security_descriptor_from("O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)")
                .expect("sddl");
        let sa = windows::Win32::Security::SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<windows::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let wide = crate::secrets::wide_str(&path.display().to_string());
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(0),
                Some(&sa),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .expect("create owned cmd");
        crate::secrets::free_descriptor(&descriptor);
        let mut file = unsafe { std::fs::File::from_raw_handle(handle.0) };
        std::io::Write::write_all(&mut file, body).expect("body");
    }

    #[test]
    fn a_junction_or_symlink_under_the_trusted_prefix_is_refused() {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = scratch("junction");
        let real = root.join("real");
        fs::create_dir_all(&real).expect("temp");
        let payload = real.join("softlock-service.exe");
        fs::write(&payload, b"exe").expect("exe");
        let plain = system32_name("plain", n);
        let via_junction = system32_name("junction", n);
        let _ = fs::remove_dir_all(&plain);
        let _ = fs::remove_dir(&via_junction);
        let _cleanup = Remove {
            files: &[],
            junctions: std::slice::from_ref(&via_junction),
            trees: &[plain.clone(), root.clone()],
        };
        fs::create_dir(&plain).expect("plain dir");
        let plain_exe = plain.join("softlock-service.exe");
        fs::write(&plain_exe, b"exe").expect("plain exe");
        junction(&via_junction, &real);
        let linked = via_junction.join("softlock-service.exe");
        let symlink = plain.join("linked-service.exe");
        symlink_file(&symlink, &payload);
        let metachar = Path::new(r"C:\Windows\System32\softlock&service.exe");
        let allowed_plain = command_path_allowed(&plain_exe);
        let allowed_junction = command_path_allowed(&linked);
        let allowed_symlink = command_path_allowed(&symlink);
        let allowed_meta = command_path_allowed(metachar);
        assert!(allowed_plain, "a plain image under System32 was refused");
        assert!(!allowed_junction, "a junction under System32 was accepted");
        assert!(!allowed_symlink, "a symlink image was accepted");
        assert!(
            !allowed_meta,
            "a metacharacter in the image path was accepted"
        );
    }

    #[test]
    fn an_existing_menu_command_is_rewritten_and_a_reparse_folder_is_refused() {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = scratch("menu");
        let programs = root.join("programs");
        fs::create_dir_all(&programs).expect("programs");
        let link = root.join("programs-link");
        junction(&link, &programs);
        assert!(
            menu_directory_is_plain(&programs),
            "a real programs directory was refused"
        );
        assert!(link.is_dir(), "the junction did not look like a directory");
        assert!(
            !menu_directory_is_plain(&link),
            "a reparse programs folder was followed"
        );
        let cmd = programs.join("Repair.cmd");
        write_owned(&cmd, b"old\r\n");
        write_menu_new(&cmd, b"@echo off\r\nnew\r\n").expect("refresh");
        let body = fs::read(&cmd).expect("read");
        let other = root.join("other.cmd");
        fs::write(&other, b"other").expect("other");
        let linked_cmd = system32_name(&format!("menu-link-{n}"), n).with_extension("cmd");
        let _ = fs::remove_file(&linked_cmd);
        let _cleanup = Remove {
            files: std::slice::from_ref(&linked_cmd),
            junctions: std::slice::from_ref(&link),
            trees: std::slice::from_ref(&root),
        };
        symlink_file(&linked_cmd, &other);
        let reparse_file = write_menu_new(&linked_cmd, b"nope\r\n");
        assert_eq!(body, b"@echo off\r\nnew\r\n");
        assert!(reparse_file.is_err(), "a symlink command was rewritten");
    }
}
