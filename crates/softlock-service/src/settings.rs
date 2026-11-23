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
    /// An active lock must keep DisableTaskMgr. Revert does not clear it.
    preserve: bool,
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

    use super::{command_path_allowed, menu_directory_is_plain, write_menu_new};

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("softlock-menu-{}", std::process::id()))
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
        let status = Command::new("cmd")
            .args([
                "/c",
                "mklink",
                &link.display().to_string(),
                &target.display().to_string(),
            ])
            .status()
            .expect("mklink");
        assert!(status.success(), "symlink {:?} -> {:?}", link, target);
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
        let id = std::process::id();
        let root = temp_root();
        let _ = fs::remove_dir_all(&root);
        let real = root.join("real");
        fs::create_dir_all(&real).expect("temp");
        let payload = real.join("softlock-service.exe");
        fs::write(&payload, b"exe").expect("exe");
        let plain = PathBuf::from(format!(r"C:\Windows\System32\softlock-plain-{id}"));
        let via_junction = PathBuf::from(format!(r"C:\Windows\System32\softlock-junction-{id}"));
        let _ = fs::remove_dir_all(&plain);
        let _ = fs::remove_dir(&via_junction);
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
        let _ = fs::remove_file(&symlink);
        let _ = fs::remove_dir(&via_junction);
        let _ = fs::remove_dir_all(&plain);
        let _ = fs::remove_dir_all(&root);
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
        let root = temp_root().join("menu");
        let _ = fs::remove_dir_all(&root);
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
        let linked_cmd = programs.join("linked.cmd");
        symlink_file(&linked_cmd, &other);
        let reparse_file = write_menu_new(&linked_cmd, b"nope\r\n");
        let _ = fs::remove_dir(&link);
        let _ = fs::remove_dir_all(&root);
        assert_eq!(body, b"@echo off\r\nnew\r\n");
        assert!(reparse_file.is_err(), "a symlink command was rewritten");
    }
}
