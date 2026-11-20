//! Apply DisableTaskMgr only after the journal is durable, and put it back exactly.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use softlock_core::{
    CommitFault, HardenError, Journal, JournalEntry, JournalFault, ProductState, RepairAction,
    SERVICE_NAME, SettingValue, SettingsJournal, StartAction, TASK_MANAGER, commit_then_change,
    encode_document, executable_may_run_as_system, fault_log, parse_document, repair_action,
    repair_entry_name, restore_action, service_start_action, task_manager_document,
};

use crate::journal_store::{load_journal, save_journal};
use crate::taskmgr::{
    console_is_active, console_sid, lock_host_running, read_task_manager, watch_policy,
    write_task_manager,
};

pub enum Boot {
    Idle,
    Reverted,
    FailClosed(Journal),
}

pub struct SettingsEngine {
    sid: String,
    pending: Option<SettingsJournal>,
    holding: Arc<AtomicBool>,
    notified: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    last: Instant,
    watcher: Mutex<Option<JoinHandle<()>>>,
}

impl SettingsEngine {
    pub fn open() -> Result<Self, &'static str> {
        let sid = console_sid()?;
        // Opening the directory refuses a user-owned or pre-created folder.
        load_journal().map_err(|_| "journal")?;
        Ok(Self {
            sid,
            pending: None,
            holding: Arc::new(AtomicBool::new(false)),
            notified: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            last: Instant::now(),
            watcher: Mutex::new(None),
        })
    }

    pub fn boot(&mut self) -> Result<Boot, &'static str> {
        let loaded = load_journal().map_err(|_| "journal")?;
        let Some(text) = loaded else {
            return Ok(Boot::Idle);
        };
        let doc = parse_document(text.as_bytes()).map_err(fault_log)?;
        if doc.is_empty() {
            return Ok(Boot::Idle);
        }
        let console = match console_is_active() {
            Ok(active) => active,
            Err(_) if doc.says_locked() => {
                self.hold();
                return Ok(Boot::FailClosed(memory_journal(&doc)));
            }
            Err(_) => false,
        };
        match service_start_action(&doc, console, lock_host_running()) {
            StartAction::Idle => Ok(Boot::Idle),
            StartAction::Revert => {
                self.revert_document(&doc)?;
                Ok(Boot::Reverted)
            }
            StartAction::FailClosed => {
                self.hold();
                Ok(Boot::FailClosed(memory_journal(&doc)))
            }
        }
    }

    pub fn stage(&mut self, journal: &mut Journal) -> Result<(), HardenError> {
        let previous = read_task_manager(&self.sid).map_err(|reason| {
            crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
            HardenError::Failed
        })?;
        let doc = task_manager_document(previous, journal.failures()).map_err(|_| {
            crate::audit("settings", &[("result", "failed"), ("reason", "value")]);
            HardenError::Failed
        })?;
        self.pending = Some(doc);
        journal.record(JournalEntry {
            target: TASK_MANAGER.to_string(),
            previous: previous_text(previous),
            new_value: "1".to_string(),
        });
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
        let saved = commit_then_change(
            || save_journal(&bytes).map_err(|_| JournalFault::NotDurable),
            || {
                pause_after_flush();
                write_task_manager(&sid, SettingValue::Dword { value: 1 })
            },
        );
        match saved {
            Ok(()) => {
                self.hold();
                crate::audit(
                    "settings",
                    &[("result", "applied"), ("target", TASK_MANAGER)],
                );
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
        if let Some(text) = load_journal().map_err(|_| "journal")? {
            let doc = parse_document(text.as_bytes()).map_err(fault_log)?;
            return self.revert_document(&doc);
        }
        for entry in journal.entries() {
            if entry.target != TASK_MANAGER {
                continue;
            }
            let previous = match entry.previous.as_deref() {
                None => SettingValue::Absent,
                Some("0") => SettingValue::Dword { value: 0 },
                Some("1") => SettingValue::Dword { value: 1 },
                Some(_) => return Err("value"),
            };
            self.write_previous(previous)?;
        }
        self.release();
        Ok(())
    }

    pub fn repair_leftover(&mut self) -> Result<(), &'static str> {
        let Some(text) = load_journal().map_err(|_| "journal")? else {
            return Ok(());
        };
        let doc = parse_document(text.as_bytes()).map_err(fault_log)?;
        if doc.is_empty() {
            return Ok(());
        }
        self.revert_document(&doc)
    }

    pub fn reassert_now(&mut self) -> Result<(), &'static str> {
        let current = read_task_manager(&self.sid)?;
        if current != (SettingValue::Dword { value: 1 }) {
            write_task_manager(&self.sid, SettingValue::Dword { value: 1 })?;
            crate::audit(
                "settings",
                &[("result", "reasserted"), ("target", TASK_MANAGER)],
            );
        }
        self.last = Instant::now();
        Ok(())
    }

    /// Wall-clock re-assert. The controller clock does not advance in the service.
    pub fn reassert_if_due(&mut self) -> Result<(), &'static str> {
        if !self.holding.load(Ordering::Acquire) {
            return Ok(());
        }
        let notified = self.notified.swap(false, Ordering::AcqRel);
        if !notified && self.last.elapsed() < Duration::from_secs(5) {
            return Ok(());
        }
        self.reassert_now()
    }

    fn write_previous(&mut self, previous: SettingValue) -> Result<(), &'static str> {
        let current = read_task_manager(&self.sid)?;
        match restore_action(previous, current).map_err(|_| "value")? {
            Some(next) => write_task_manager(&self.sid, next),
            None => Ok(()),
        }
    }

    fn revert_document(&mut self, doc: &SettingsJournal) -> Result<(), &'static str> {
        for entry in doc.entries() {
            if entry.target != TASK_MANAGER {
                return Err("target");
            }
            self.write_previous(entry.previous)?;
        }
        let empty = encode_document(&SettingsJournal::empty()).map_err(|_| "malformed")?;
        save_journal(&empty).map_err(|_| "not-durable")?;
        self.release();
        crate::audit(
            "settings",
            &[("result", "restored"), ("target", TASK_MANAGER)],
        );
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

fn previous_text(value: SettingValue) -> Option<String> {
    match value {
        SettingValue::Absent => None,
        SettingValue::Dword { value } => Some(value.to_string()),
    }
}

fn memory_journal(doc: &SettingsJournal) -> Journal {
    let mut journal = Journal::empty();
    for entry in doc.entries() {
        journal.record(JournalEntry {
            target: entry.target.clone(),
            previous: previous_text(entry.previous),
            new_value: "1".to_string(),
        });
    }
    journal.set_failures(doc.failures());
    if doc.says_locked() {
        journal.mark_locked();
    }
    journal.flush();
    journal
}

pub fn repair_main() -> i32 {
    let state = match crate::windows::running_product_state() {
        None => ProductState::Unlocked,
        Some(state) => state,
    };
    if repair_action(state) == RepairAction::Refuse {
        crate::audit("settings", &[("result", "refused"), ("reason", "locked")]);
        eprintln!("repair refused while SoftLock is locked");
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
    let created = Command::new(r"C:\Windows\System32\schtasks.exe")
        .args([
            "/Create", "/F", "/TN", &task, "/SC", "ONLOGON", "/RU", "SYSTEM", "/RL", "HIGHEST",
            "/TR", &command,
        ])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if created {
        crate::audit("settings", &[("result", "task")]);
    } else {
        crate::audit("settings", &[("result", "failed"), ("reason", "task")]);
    }
    if let Err(reason) = write_start_menu(&exe) {
        crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
    }
}

fn write_start_menu(exe: &Path) -> Result<(), &'static str> {
    let programs = PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs");
    if !programs.is_dir() {
        return Err("menu");
    }
    let path = programs.join(format!("{}.cmd", repair_entry_name()));
    let body = format!("@echo off\r\n\"{}\" --repair-settings\r\n", exe.display());
    std::fs::write(&path, body).map_err(|_| "menu")
}

fn command_path_allowed(path: &Path) -> bool {
    let text = path.to_string_lossy();
    executable_may_run_as_system(path)
        && !text
            .chars()
            .any(|c| matches!(c, '&' | '|' | '>' | '<' | '%' | '!' | '\n' | '\r' | '"'))
}

#[cfg(feature = "live-fail-closed")]
fn pause_after_flush() {
    if !crate::windows::live_hooks_allowed() {
        return;
    }
    if std::env::var("SOFTLOCK_JOURNAL_PAUSE").ok().as_deref() != Some("1") {
        return;
    }
    let path = std::env::var("SOFTLOCK_JOURNAL_PAUSE_FILE").unwrap_or_default();
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if !path.is_empty() && Path::new(&path).is_file() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(not(feature = "live-fail-closed"))]
fn pause_after_flush() {}
