//! Apply DisableTaskMgr only after the journal is durable, and put it back exactly.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use softlock_core::PRODUCT_DISPLAY_NAME;
use softlock_core::{
    CommitFault, HardenError, Journal, JournalEntry, JournalFault, ProductState, RepairAction,
    SERVICE_NAME, SettingValue, SettingsJournal, StartAction, TASK_MANAGER, commit_then_change,
    encode_document, executable_may_run_as_system, fault_log, parse_document, repair_action,
    repair_entry_name, restore_action, restore_sid, service_start_action, task_manager_document,
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
    /// An active lock must keep DisableTaskMgr. Revert does not clear it.
    preserve: bool,
}

impl SettingsEngine {
    pub fn open() -> Result<Self, &'static str> {
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
            preserve: false,
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
        self.sid = restore_sid(&doc).map_err(fault_log)?.to_string();
        let console = match console_is_active() {
            Ok(active) => active,
            Err(_) if doc.says_locked() => {
                self.preserve = true;
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
                self.preserve = true;
                self.hold();
                Ok(Boot::FailClosed(memory_journal(&doc)))
            }
        }
    }

    pub fn stage(&mut self, journal: &mut Journal) -> Result<(), HardenError> {
        let sid = console_sid().map_err(|reason| {
            crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
            HardenError::Failed
        })?;
        self.sid = sid;
        let previous = read_task_manager(&self.sid).map_err(|reason| {
            crate::audit("settings", &[("result", "failed"), ("reason", reason)]);
            HardenError::Failed
        })?;
        let doc =
            task_manager_document(&self.sid, previous, journal.failures()).map_err(|fault| {
                crate::audit(
                    "settings",
                    &[("result", "failed"), ("reason", fault_log(fault))],
                );
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
                // The lock call is still inside the authority mutex here, so
                // an OS-lock confirmation cannot revert this value until the
                // pause ends. Killing the service in either pause leaves the
                // journal durable.
                pause_for("before");
                let written = write_task_manager(&sid, SettingValue::Dword { value: 1 });
                pause_for("after");
                written
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
        if self.preserve {
            return Ok(());
        }
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
        self.sid = restore_sid(&doc).map_err(fault_log)?.to_string();
        self.preserve = false;
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
    let created = Command::new(r"C:\Windows\System32\schtasks.exe")
        .args([
            "/Create", "/F", "/TN", &task, "/SC", "ONLOGON", "/RU", "SYSTEM", "/RL", "HIGHEST",
            "/TR", &command,
        ])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !created {
        crate::audit(
            "settings",
            &[("result", "failed"), ("reason", "task-create")],
        );
        return;
    }
    let present = Command::new(r"C:\Windows\System32\schtasks.exe")
        .args(["/Query", "/TN", &task])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
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

fn write_start_menu(exe: &Path) -> Result<(), &'static str> {
    let programs = PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs");
    if !programs.is_dir() {
        return Err("menu");
    }
    let path = programs.join(format!("{}.cmd", repair_entry_name()));
    let body = format!("@echo off\r\n\"{}\" --repair-settings\r\n", exe.display());
    write_menu_new(&path, body.as_bytes())
}

fn command_path_allowed(path: &Path) -> bool {
    let text = path.to_string_lossy();
    executable_may_run_as_system(path, path)
        && !text
            .chars()
            .any(|c| matches!(c, '&' | '|' | '>' | '<' | '%' | '!' | '\n' | '\r' | '"'))
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
            FILE_GENERIC_READ.0,
            windows::Win32::Storage::FileSystem::FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    };
    if let Ok(handle) = existing {
        let trusted = crate::secrets::owner_is_trusted(handle).unwrap_or(false);
        let reparse = crate::secrets::handle_is_reparse(handle).unwrap_or(true);
        crate::secrets::drop_handle(handle);
        if reparse || !trusted {
            return Err("menu-owner");
        }
        return Ok(());
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
