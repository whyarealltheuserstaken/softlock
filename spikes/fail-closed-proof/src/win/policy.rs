//! Registry reads and writes for the lockdown values. The journal is flushed first.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree,
};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE,
    RRF_RT_REG_DWORD, RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegFlushKey, RegGetValueW,
    RegSetValueExW,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PCWSTR;

use crate::journal::{
    Document, FileJournal, Hive, JournalSink, PolicyStore, apply, overwrite, revert,
};

use super::util::{explain, status_error, wide, wide_ptr};

static ARMED: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn arm(path: PathBuf) {
    *lock_mutex(&ARMED) = Some(path);
}

pub fn restore_armed() {
    let path = lock_mutex(&ARMED).clone();
    if let Some(path) = path {
        let _ = restore_at(&path);
    }
}

pub struct RegistryStore {
    sid: String,
}

impl RegistryStore {
    pub fn new(sid: impl Into<String>) -> Self {
        Self { sid: sid.into() }
    }

    fn root_and_path(&self, hive: Hive, subkey: &str) -> (HKEY, String) {
        match hive {
            Hive::User => (HKEY_USERS, format!("{}\\{subkey}", self.sid)),
            Hive::Machine => (HKEY_LOCAL_MACHINE, subkey.to_string()),
        }
    }
}

impl PolicyStore for RegistryStore {
    fn read(&self, hive: Hive, subkey: &str, name: &str) -> Result<Option<u32>, String> {
        let (root, path) = self.root_and_path(hive, subkey);
        let path = wide(&path);
        let value = wide(name);
        let mut data = 0u32;
        let mut bytes = std::mem::size_of::<u32>() as u32;
        let status = unsafe {
            RegGetValueW(
                root,
                PCWSTR(path.as_ptr()),
                PCWSTR(value.as_ptr()),
                RRF_RT_REG_DWORD,
                None,
                Some((&mut data as *mut u32).cast()),
                Some(&mut bytes),
            )
        };
        if status == ERROR_SUCCESS {
            return Ok(Some(data));
        }
        if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
            return Ok(None);
        }
        Err(format!("read {name}: win32 {}", status.0))
    }

    fn write(&mut self, hive: Hive, subkey: &str, name: &str, value: u32) -> Result<(), String> {
        let key = self.open_key(hive, subkey, true)?;
        let value_name = wide(name);
        let bytes = value.to_le_bytes();
        let status = unsafe {
            RegSetValueExW(
                key,
                PCWSTR(value_name.as_ptr()),
                None,
                REG_DWORD,
                Some(&bytes),
            )
        };
        let flushed = unsafe { RegFlushKey(key) };
        close_key(key);
        status_error(status, &format!("write {name}"))?;
        status_error(flushed, &format!("flush {name}"))?;
        Ok(())
    }

    fn delete(&mut self, hive: Hive, subkey: &str, name: &str) -> Result<(), String> {
        let key = match self.open_key(hive, subkey, false) {
            Ok(key) => key,
            Err(error) if error.contains("win32 2") || error.contains("win32 3") => return Ok(()),
            Err(error) => return Err(error),
        };
        let value_name = wide(name);
        let status = unsafe { RegDeleteValueW(key, PCWSTR(value_name.as_ptr())) };
        let flushed = unsafe { RegFlushKey(key) };
        close_key(key);
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
            return Err(format!("delete {name}: win32 {}", status.0));
        }
        status_error(flushed, &format!("flush {name}"))?;
        Ok(())
    }
}

impl RegistryStore {
    fn open_key(&self, hive: Hive, subkey: &str, create: bool) -> Result<HKEY, String> {
        let (root, path) = self.root_and_path(hive, subkey);
        let path = wide(&path);
        let mut key = HKEY::default();
        if create {
            let status = unsafe {
                RegCreateKeyExW(
                    root,
                    PCWSTR(path.as_ptr()),
                    None,
                    PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_READ | KEY_WRITE,
                    None,
                    &mut key,
                    None,
                )
            };
            status_error(status, "RegCreateKeyExW")?;
        } else {
            let status = unsafe {
                windows::Win32::System::Registry::RegOpenKeyExW(
                    root,
                    PCWSTR(path.as_ptr()),
                    None,
                    KEY_READ | KEY_WRITE,
                    &mut key,
                )
            };
            status_error(status, "RegOpenKeyExW")?;
        }
        Ok(key)
    }
}

fn close_key(key: HKEY) {
    unsafe {
        let _ = RegCloseKey(key);
    }
}

pub fn current_sid() -> Result<String, String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|error| explain(error, "OpenProcessToken"))?;
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = windows::Win32::Foundation::CloseHandle(token);
            return Err("TokenUser size was zero".to_string());
        }
        let words = needed.div_ceil(8);
        let mut buffer = vec![0u64; words as usize];
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            (buffer.len() * 8) as u32,
            &mut needed,
        )
        .map_err(|error| explain(error, "GetTokenInformation"))?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = windows::core::PWSTR::null();
        let converted = ConvertSidToStringSidW(user.User.Sid, &mut text);
        let _ = windows::Win32::Foundation::CloseHandle(token);
        converted.map_err(|error| explain(error, "ConvertSidToStringSidW"))?;
        let sid = wide_ptr(text);
        if !text.0.is_null() {
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        }
        if sid.is_empty() {
            return Err("the account SID was empty".to_string());
        }
        Ok(sid)
    }
}

pub fn is_elevated() -> Result<bool, String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|error| explain(error, "OpenProcessToken"))?;
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0u32;
        let status = GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        let _ = windows::Win32::Foundation::CloseHandle(token);
        status.map_err(|error| explain(error, "TokenElevation"))?;
        Ok(elevation.TokenIsElevated != 0)
    }
}

pub fn apply_lockdown(output_dir: &Path) -> Result<(Document, RegistryStore), String> {
    let sid = current_sid()?;
    let path = journal_path(output_dir);
    with_lock(output_dir, || {
        let mut document = Document::new(sid.clone());
        let mut store = RegistryStore::new(sid);
        let mut sink = FileJournal::new(&path);
        for policy in crate::journal::LOCKDOWN {
            apply(
                &mut document,
                &mut store,
                &mut sink,
                policy.hive,
                policy.subkey,
                policy.name,
                policy.applied,
            )?;
            let current = store.read(policy.hive, policy.subkey, policy.name)?;
            if current != Some(policy.applied) {
                return Err(format!(
                    "{} read back as {current:?} after it was set to {}",
                    policy.name, policy.applied
                ));
            }
        }
        Ok((document, store))
    })
}

pub fn clear_disable_lock(
    output_dir: &Path,
    document: &Document,
    store: &mut RegistryStore,
) -> Result<(), String> {
    with_lock(output_dir, || {
        overwrite(
            document,
            store,
            crate::journal::Hive::User,
            crate::journal::SYSTEM_POLICIES,
            crate::journal::DISABLE_LOCK,
            0,
        )?;
        let current = store.read(
            crate::journal::Hive::User,
            crate::journal::SYSTEM_POLICIES,
            crate::journal::DISABLE_LOCK,
        )?;
        if current != Some(0) {
            return Err(format!(
                "DisableLockWorkstation read back as {current:?} after it was cleared"
            ));
        }
        Ok(())
    })
}

pub fn restore_at(path: &Path) -> Result<bool, String> {
    let Some(parent) = path.parent() else {
        return Err("journal path has no directory".to_string());
    };
    with_lock(parent, || restore_unlocked(path))
}

fn restore_unlocked(path: &Path) -> Result<bool, String> {
    let mut sink = FileJournal::new(path);
    let Some(mut document) = sink.load()? else {
        return Ok(false);
    };
    if document.is_empty() {
        sink.remove()?;
        return Ok(false);
    }
    let saved = document.entries.clone();
    let mut store = RegistryStore::new(document.sid.clone());
    revert(&mut document, &mut store, &mut sink)?;
    for entry in saved {
        let current = store.read(entry.hive, &entry.subkey, &entry.name)?;
        if current != entry.previous {
            return Err(format!(
                "{} restored to {current:?}, previous was {:?}",
                entry.name, entry.previous
            ));
        }
    }
    Ok(true)
}

pub fn journal_path(output_dir: &Path) -> PathBuf {
    output_dir.join(crate::protocol::JOURNAL_FILE)
}

fn with_lock<T>(dir: &Path, body: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let lock_path = dir.join("journal.lock");
    let file = File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| format!("open {}: {error}", lock_path.display()))?;
    let start = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => {
                let result = body();
                let _ = file.unlock();
                return result;
            }
            Err(TryLockError::WouldBlock) => {
                if start.elapsed() > Duration::from_secs(30) {
                    return Err("timed out waiting for the policy journal lock".to_string());
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(format!("lock {}: {error}", lock_path.display())),
        }
    }
}

fn lock_mutex<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}
