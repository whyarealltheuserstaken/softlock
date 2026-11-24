//! The owner's config file. It is untrusted: size-capped, and every path
//! component is opened with `FILE_OPEN_REPARSE_POINT` from the parent handle.
//! Reads, writes, and DACL changes use that same handle.

use std::io;
use std::path::{Path, PathBuf};

use softlock_core::{
    CONFIG_DIR, CONFIG_FILE_NAME, JournalFault, MAX_CONFIG_BYTES, SettingEntry, SettingValue,
    config_acl_entry, config_dir_entry, config_path_allowed, dacl_without_owner_write,
    restore_action,
};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, HANDLE};
use windows::Win32::Storage::FileSystem::{
    FILE_BEGIN, GetFileSizeEx, ReadFile, SetEndOfFile, SetFilePointerEx, WriteFile,
};

use crate::secrets::{
    already_exists_io, dacl_sddl_handle, delete_by_handle, drop_handle, handle_is_reparse,
    not_found, open_child, open_child_at, open_root, os_error,
};

const FILE_LIST_DIRECTORY: u32 = 0x0001;
const FILE_ADD_FILE: u32 = 0x0002;
const FILE_ADD_SUBDIRECTORY: u32 = 0x0004;
const FILE_TRAVERSE: u32 = 0x0020;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const DELETE: u32 = 0x0001_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_READ_DATA: u32 = 0x0001;
const FILE_WRITE_DATA: u32 = 0x0002;
const FILE_APPEND_DATA: u32 = 0x0004;
const FILE_READ_EA: u32 = 0x0008;
const FILE_WRITE_EA: u32 = 0x0010;
const FILE_READ_ATTRIBUTES: u32 = 0x0080;
const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;

const DIR_OPEN: u32 =
    FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE;
const DIR_CREATE: u32 = DIR_OPEN | FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY;
const FILE_ACCESS: u32 = FILE_READ_DATA
    | FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_READ_EA
    | FILE_WRITE_EA
    | FILE_READ_ATTRIBUTES
    | FILE_WRITE_ATTRIBUTES
    | READ_CONTROL
    | WRITE_DAC
    | DELETE
    | SYNCHRONIZE;
pub fn configured_path() -> Result<PathBuf, &'static str> {
    let path = if let Some(dir) = test_config_dir() {
        dir.join(CONFIG_FILE_NAME)
    } else {
        crate::taskmgr::owner_config_file()?
    };
    ensure_file(&path).map_err(config_reason)?;
    Ok(path)
}

fn config_reason(error: io::Error) -> &'static str {
    let code = error.raw_os_error().unwrap_or(-1);
    let text = error.to_string().replace(' ', "_");
    Box::leak(format!("cfg_{code}_{text}").into_boxed_str())
}

pub(crate) fn test_config_dir() -> Option<PathBuf> {
    crate::authority::test_path("SOFTLOCK_CONFIG_DIR")
}

pub fn prepare_entry(path: &Path) -> Result<SettingEntry, &'static str> {
    let handle = open_config(path, false).map_err(|_| "config")?;
    let text = read_handle(handle);
    let previous = dacl_sddl_handle(handle);
    let owner = owner_sid_handle(handle);
    drop_handle(handle);
    let text = text.map_err(|_| "config")?;
    if text.len() > MAX_CONFIG_BYTES {
        return Err("config");
    }
    let previous = previous.map_err(|_| "acl")?;
    let owner = owner.map_err(|_| "acl")?;
    let console = crate::taskmgr::console_sid().map_err(|_| "sid")?;
    let locked = dacl_without_owner_write(&previous, &console).map_err(|_| "acl")?;
    let path_text = path_text(path).map_err(|_| "acl")?;
    config_acl_entry(&path_text, &previous, &locked, &owner).map_err(fault)
}

pub fn prepare_dir_entry(path: &Path) -> Result<SettingEntry, &'static str> {
    let Some(dir) = path.parent() else {
        return Err("acl");
    };
    let handle = open_acl(dir, true).map_err(|_| "acl")?;
    let previous = dacl_sddl_handle(handle);
    let owner = owner_sid_handle(handle);
    drop_handle(handle);
    let previous = previous.map_err(|_| "acl")?;
    let owner = owner.map_err(|_| "acl")?;
    let console = crate::taskmgr::console_sid().map_err(|_| "sid")?;
    let locked = dacl_without_owner_write(&previous, &console).map_err(|_| "acl")?;
    let text = dir.to_string_lossy().replace('/', "\\");
    config_dir_entry(&text, &previous, &locked, &owner).map_err(fault)
}

pub fn apply_entry(entry: &SettingEntry) -> Result<(), &'static str> {
    let SettingValue::Sddl { value } = &entry.new_value else {
        return Err("value");
    };
    let handle = open_entry(entry).map_err(|_| "acl")?;
    let result = set_dacl_handle(handle, value);
    drop_handle(handle);
    result
}

pub fn restore_entry(entry: &SettingEntry) -> Result<(), &'static str> {
    let ordinary = restore_entry_ordinary(entry);
    if ordinary.is_ok() {
        return Ok(());
    }
    // The locked DACL, or one that denies SYSTEM WRITE_DAC, refuses the
    // ordinary open. Take ownership and write the journaled previous with
    // backup semantics. A failed restore still fail-closes. The owner is
    // restored with the DACL so the descriptor matches the original.
    let SettingValue::Sddl { value } = &entry.previous else {
        return ordinary;
    };
    write_dacl_privileged(
        Path::new(&entry.path),
        entry.target == CONFIG_DIR,
        value,
        &entry.previous_owner,
    )
}

fn restore_entry_ordinary(entry: &SettingEntry) -> Result<(), &'static str> {
    let handle = open_entry(entry).map_err(|_| "acl")?;
    let current = dacl_sddl_handle(handle).map_err(|_| "acl");
    let result = match current {
        Ok(current) => {
            let current_value = SettingValue::Sddl { value: current };
            match restore_action(&entry.previous, &current_value).map_err(|_| "value")? {
                Some(SettingValue::Sddl { value }) => {
                    set_dacl_handle(handle, &value)?;
                    set_owner_sid(handle, &entry.previous_owner)
                }
                Some(_) => Err("value"),
                None => set_owner_sid(handle, &entry.previous_owner),
            }
        }
        Err(reason) => Err(reason),
    };
    drop_handle(handle);
    result
}

fn set_owner_sid(handle: HANDLE, owner: &str) -> Result<(), &'static str> {
    if owner.is_empty() {
        return Ok(());
    }
    use windows::Win32::Security::OWNER_SECURITY_INFORMATION;
    set_security_handle(handle, &format!("O:{owner}"), OWNER_SECURITY_INFORMATION)
}

fn write_dacl_privileged(
    path: &Path,
    directory: bool,
    sddl: &str,
    owner: &str,
) -> Result<(), &'static str> {
    use windows::Win32::Security::{SE_BACKUP_NAME, SE_RESTORE_NAME, SE_TAKE_OWNERSHIP_NAME};
    let _take =
        crate::secrets::enable_privilege_scoped(SE_TAKE_OWNERSHIP_NAME).map_err(|_| "privilege")?;
    let _restore =
        crate::secrets::enable_privilege_scoped(SE_RESTORE_NAME).map_err(|_| "privilege")?;
    let _backup =
        crate::secrets::enable_privilege_scoped(SE_BACKUP_NAME).map_err(|_| "privilege")?;
    let handle = open_backup(path, directory).map_err(|_| "acl")?;
    if refuse_reparse(handle).map_err(|_| "reparse")? {
        drop_handle(handle);
        return Err("reparse");
    }
    let result = set_dacl_handle(handle, sddl).and_then(|()| set_owner_sid(handle, owner));
    drop_handle(handle);
    result
}

pub fn reassert_entry(entry: &SettingEntry) -> Result<(), &'static str> {
    let SettingValue::Sddl { value: locked } = &entry.new_value else {
        return Err("value");
    };
    if !locked.starts_with("D:") {
        return Err("value");
    }
    // The poll thread is the service. A leaked impersonation would make the
    // token the client, and WRITE_DAC would then be the client's right.
    let impersonating = thread_is_impersonating();
    if impersonating {
        unsafe {
            let _ = windows::Win32::Security::RevertToSelf();
        }
    }
    let directory = entry.target == CONFIG_DIR;
    let path = Path::new(&entry.path);
    let (owner, dacl) = read_security(path, directory);
    let held = security_is_held(owner.as_deref(), dacl.as_deref());
    note_acl("acl-check", &owner, &dacl, held, impersonating);
    if held {
        return Ok(());
    }
    // Not held. Rewrite as SYSTEM. Access denied is not a reason to stop:
    // SeTakeOwnershipPrivilege and SeRestorePrivilege, with backup semantics,
    // put the owner back and write the DACL. Only a failed rewrite fail-closes.
    let (owner, dacl) = reclaim_security(path, directory, locked)?;
    let held = security_is_held(Some(&owner), Some(&dacl));
    note_acl("acl-reclaimed", &Some(owner), &Some(dacl), held, false);
    if !held {
        return Err("acl");
    }
    // A later READ_CONTROL open is the same check without the backup
    // privilege. If that open fails, the descriptor just read stands.
    let (again_owner, again_dacl) = read_security(path, directory);
    if again_owner.is_some() && again_dacl.is_some() {
        let again = security_is_held(again_owner.as_deref(), again_dacl.as_deref());
        note_acl("acl-read", &again_owner, &again_dacl, again, false);
        if !again {
            return Err("acl");
        }
    }
    Ok(())
}

fn security_is_held(owner: Option<&str>, dacl: Option<&str>) -> bool {
    let (Some(owner), Some(dacl)) = (owner, dacl) else {
        return false;
    };
    softlock_core::config_lock_held(dacl, owner) && console_granted_write(dacl) != Some(true)
}

/// `Some(true)` when the console user's token is granted write. No token
/// (the foreground seam process is not LocalSystem) leaves the SDDL parse
/// as the authority.
fn console_granted_write(dacl: &str) -> Option<bool> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::{
        AccessCheck, DuplicateTokenEx, GENERIC_MAPPING, PSECURITY_DESCRIPTOR,
        SecurityImpersonation, TOKEN_QUERY, TokenImpersonation,
    };
    use windows::core::{BOOL, PCWSTR};
    let session = crate::windows::console_session();
    if session == 0xFFFF_FFFF {
        return None;
    }
    let mut primary = HANDLE::default();
    unsafe {
        if windows::Win32::System::RemoteDesktop::WTSQueryUserToken(session, &mut primary).is_err()
        {
            return None;
        }
    }
    let mut impersonation = HANDLE::default();
    let duplicated = unsafe {
        DuplicateTokenEx(
            primary,
            TOKEN_QUERY,
            None,
            SecurityImpersonation,
            TokenImpersonation,
            &mut impersonation,
        )
    };
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(primary);
    }
    if duplicated.is_err() {
        return None;
    }
    let wide = dacl
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide.as_ptr()),
            1,
            &mut descriptor,
            None,
        )
    };
    if converted.is_err() {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(impersonation);
        }
        return None;
    }
    let mapping = GENERIC_MAPPING {
        GenericRead: 0x120089,
        GenericWrite: 0x120116,
        GenericExecute: 0x1200A0,
        GenericAll: 0x1F01FF,
    };
    let desired = FILE_WRITE_DATA
        | FILE_APPEND_DATA
        | FILE_WRITE_EA
        | FILE_WRITE_ATTRIBUTES
        | 0x40
        | DELETE
        | WRITE_DAC
        | WRITE_OWNER;
    let mut privileges = vec![0u64; 32];
    let mut length = (privileges.len() * std::mem::size_of::<u64>()) as u32;
    let mut granted = 0u32;
    let mut status = BOOL::default();
    let checked = unsafe {
        AccessCheck(
            descriptor,
            impersonation,
            desired,
            &mapping,
            Some(privileges.as_mut_ptr().cast()),
            &mut length,
            &mut granted,
            &mut status,
        )
    };
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(impersonation);
        if !descriptor.0.is_null() {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
    }
    checked.ok()?;
    Some(status.as_bool())
}

fn read_security(path: &Path, directory: bool) -> (Option<String>, Option<String>) {
    let Ok(handle) = open_dacl_read(path, directory) else {
        return (None, None);
    };
    let owner = owner_sid_handle(handle).ok();
    let dacl = dacl_sddl_handle(handle).ok();
    drop_handle(handle);
    (owner, dacl)
}

fn reclaim_security(
    path: &Path,
    directory: bool,
    locked: &str,
) -> Result<(String, String), &'static str> {
    use windows::Win32::Security::{
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        SE_BACKUP_NAME, SE_RESTORE_NAME, SE_TAKE_OWNERSHIP_NAME,
    };
    let _take = crate::secrets::enable_privilege_scoped(SE_TAKE_OWNERSHIP_NAME).map_err(|_| {
        crate::audit(
            "settings",
            &[("result", "acl-privilege"), ("name", "take-ownership")],
        );
        "privilege"
    })?;
    let _restore = crate::secrets::enable_privilege_scoped(SE_RESTORE_NAME).map_err(|_| {
        crate::audit(
            "settings",
            &[("result", "acl-privilege"), ("name", "restore")],
        );
        "privilege"
    })?;
    let _backup = crate::secrets::enable_privilege_scoped(SE_BACKUP_NAME).map_err(|_| {
        crate::audit(
            "settings",
            &[("result", "acl-privilege"), ("name", "backup")],
        );
        "privilege"
    })?;
    let handle = open_backup(path, directory).map_err(|_| "acl")?;
    if refuse_reparse(handle).map_err(|_| "reparse")? {
        drop_handle(handle);
        return Err("reparse");
    }
    let descriptor = format!("O:SY{locked}");
    let wrote = set_security_handle(
        handle,
        &descriptor,
        OWNER_SECURITY_INFORMATION
            | DACL_SECURITY_INFORMATION
            | PROTECTED_DACL_SECURITY_INFORMATION,
    );
    if wrote.is_err() {
        // SeTakeOwnershipPrivilege can name Administrators as the owner
        // when naming SYSTEM is refused. Owner Rights still blocks a
        // later WRITE_DAC, so the DACL write above has to have succeeded
        // for a hostile descriptor. An owner change alone is enough when
        // the ACEs are already the lock.
        let _ = set_security_handle(handle, "O:BA", OWNER_SECURITY_INFORMATION);
    }
    let owner = owner_sid_handle(handle).map_err(|_| "acl");
    let dacl = dacl_sddl_handle(handle).map_err(|_| "acl");
    drop_handle(handle);
    match (owner, dacl) {
        (Ok(owner), Ok(dacl)) if security_is_held(Some(&owner), Some(&dacl)) => Ok((owner, dacl)),
        (Ok(owner), Ok(dacl)) => {
            note_acl("acl-reclaimed", &Some(owner), &Some(dacl), false, false);
            Err(wrote.err().unwrap_or("acl"))
        }
        _ => Err(wrote.err().unwrap_or("acl")),
    }
}

fn note_acl(
    phase: &'static str,
    owner: &Option<String>,
    dacl: &Option<String>,
    held: bool,
    impersonating: bool,
) {
    let owner = owner.clone().unwrap_or_else(|| "unread".to_string());
    let dacl = dacl.clone().unwrap_or_else(|| "unread".to_string());
    let user_write = match console_granted_write(&dacl) {
        Some(true) => "1",
        Some(false) => "0",
        None => "unknown",
    };
    let dacl: String = dacl
        .chars()
        .filter(|c| !c.is_whitespace())
        .take(360)
        .collect();
    let token = process_token_sid();
    crate::audit(
        "settings",
        &[
            ("result", phase),
            ("owner", &owner),
            ("dacl", &dacl),
            ("token", &token),
            ("impersonating", if impersonating { "1" } else { "0" }),
            ("user_write", user_write),
            ("held", if held { "1" } else { "0" }),
        ],
    );
}

fn process_token_sid() -> String {
    use windows::Win32::Security::TOKEN_QUERY;
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return "unknown".to_string();
        }
        let sid = token_user_sid(token).unwrap_or_else(|_| "unknown".to_string());
        let _ = windows::Win32::Foundation::CloseHandle(token);
        sid
    }
}

fn token_user_sid(token: windows::Win32::Foundation::HANDLE) -> io::Result<String> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_USER, TokenUser};
    use windows::core::PWSTR;
    unsafe {
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            return Err(io::Error::other("sid"));
        }
        let mut buffer = vec![0u8; needed as usize];
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .map_err(crate::secrets::os_error)?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).map_err(crate::secrets::os_error)?;
        let sid = text.to_string().unwrap_or_default();
        if !text.is_null() {
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        }
        if sid.starts_with("S-1-")
            && sid.len() < 96
            && sid
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            Ok(sid)
        } else {
            Err(io::Error::other("sid"))
        }
    }
}

fn thread_is_impersonating() -> bool {
    use windows::Win32::Security::TOKEN_QUERY;
    use windows::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};
    unsafe {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        let opened = OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token);
        if opened.is_ok() {
            let _ = windows::Win32::Foundation::CloseHandle(token);
            true
        } else {
            false
        }
    }
}

fn open_dacl_read(path: &Path, directory: bool) -> io::Result<HANDLE> {
    let access = if directory {
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE
    } else {
        FILE_READ_DATA | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE
    };
    // Parents are opened without FILE_TRAVERSE. FR does not grant that bit,
    // and the locked directory is the parent of the config file.
    open_with_access_kind(path, access, directory, false)
}

fn open_backup(path: &Path, _directory: bool) -> io::Result<HANDLE> {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::core::PCWSTR;
    let access = READ_CONTROL | WRITE_DAC | WRITE_OWNER | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
    let wide = path
        .to_string_lossy()
        .replace('/', "\\")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let flags = FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT;
    unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
        .map_err(crate::secrets::os_error)
    }
}

pub fn write_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(io::Error::other("config exceeds the size limit"));
    }
    let handle = open_config(path, true)?;
    let result = write_handle(handle, bytes);
    drop_handle(handle);
    result
}

pub fn remove_file(path: &Path) -> io::Result<()> {
    let handle = match open_config(path, false) {
        Ok(handle) => handle,
        Err(error) if not_found(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    let deleted = delete_by_handle(handle);
    drop_handle(handle);
    deleted
}

pub fn read_config(path: &Path) -> io::Result<String> {
    let handle = open_config(path, false)?;
    let text = read_handle(handle);
    drop_handle(handle);
    text
}

/// One of the rights a locked config file must refuse the console user.
pub struct OwnerRight {
    pub name: &'static str,
    pub access: u32,
}

pub fn owner_lock_rights() -> &'static [OwnerRight] {
    &[
        OwnerRight {
            name: "FILE_WRITE_DATA",
            access: FILE_WRITE_DATA,
        },
        OwnerRight {
            name: "FILE_APPEND_DATA",
            access: FILE_APPEND_DATA,
        },
        OwnerRight {
            name: "WRITE_DAC",
            access: WRITE_DAC,
        },
        OwnerRight {
            name: "WRITE_OWNER",
            access: WRITE_OWNER,
        },
        OwnerRight {
            name: "DELETE",
            access: DELETE,
        },
        OwnerRight {
            name: "FILE_WRITE_ATTRIBUTES",
            access: FILE_WRITE_ATTRIBUTES,
        },
    ]
}

pub fn owner_right_denied(path: &Path, access: u32) -> io::Result<bool> {
    // FILE_READ_ATTRIBUTES lets the reparse query run. FR already grants it,
    // so a locked file still denies the right under test.
    match open_with_access(path, access | FILE_READ_ATTRIBUTES | SYNCHRONIZE) {
        Ok(handle) => {
            let reparse = handle_is_reparse(handle)?;
            drop_handle(handle);
            if reparse {
                Err(io::Error::other("reparse"))
            } else {
                Ok(false)
            }
        }
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED.0 as i32) => Ok(true),
        Err(error) => Err(error),
    }
}

pub fn owner_write_denied(path: &Path) -> io::Result<bool> {
    match open_for_owner_write(path) {
        Ok(handle) => {
            let reparse = handle_is_reparse(handle)?;
            drop_handle(handle);
            if reparse {
                Err(io::Error::other("reparse"))
            } else {
                Ok(false)
            }
        }
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED.0 as i32) => Ok(true),
        Err(error) => Err(error),
    }
}

#[cfg(feature = "live-fail-closed")]
pub fn testing_dacl(path: &Path) -> io::Result<String> {
    let handle = open_config(path, false)?;
    let text = dacl_sddl_handle(handle);
    drop_handle(handle);
    text
}

#[cfg(feature = "live-fail-closed")]
pub fn testing_owner(path: &Path) -> io::Result<String> {
    let handle = open_config(path, false)?;
    let owner = owner_sid_handle(handle);
    drop_handle(handle);
    owner
}

#[cfg(feature = "live-fail-closed")]
pub fn testing_restore_dacl(path: &Path, sddl: &str) -> io::Result<()> {
    let handle = open_config(path, false)?;
    let result = set_dacl_handle(handle, sddl).map_err(|_| io::Error::other("acl"));
    drop_handle(handle);
    result
}

fn ensure_file(path: &Path) -> io::Result<()> {
    match open_config(path, false) {
        Ok(handle) => {
            drop_handle(handle);
            Ok(())
        }
        Err(error) if not_found(&error) => create_for_console_user(path),
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED.0 as i32) => {
            let _user = crate::taskmgr::impersonate_console_user().map_err(io::Error::other)?;
            match open_config(path, false) {
                Ok(handle) => {
                    drop_handle(handle);
                    Ok(())
                }
                Err(error) if not_found(&error) => create_for_console_user(path),
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn create_for_console_user(path: &Path) -> io::Result<()> {
    let sid = crate::taskmgr::console_sid().map_err(io::Error::other)?;
    if !softlock_core::sid_is_safe(&sid) {
        return Err(io::Error::other("sid"));
    }
    let handle = open_config(path, true)?;
    if let Err(error) = claim_for_console_user(handle, &sid) {
        drop_handle(handle);
        return Err(error);
    }
    let body = softlock_core::Config::initial().document().to_string();
    let written = write_handle(handle, body.as_bytes()).map_err(|error| annotate("write", error));
    drop_handle(handle);
    written
}

fn claim_for_console_user(handle: HANDLE, sid: &str) -> io::Result<()> {
    use windows::Win32::Security::{
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        SE_RESTORE_NAME,
    };
    let dacl = format!("D:P(A;;FA;;;SY)(A;;FA;;;{sid})");
    let _restore = crate::secrets::enable_privilege_scoped(SE_RESTORE_NAME).ok();
    let owned = format!("O:{sid}{dacl}");
    if set_security_handle(
        handle,
        &owned,
        OWNER_SECURITY_INFORMATION
            | DACL_SECURITY_INFORMATION
            | PROTECTED_DACL_SECURITY_INFORMATION,
    )
    .is_ok()
    {
        return Ok(());
    }
    set_security_handle(
        handle,
        &dacl,
        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
    )
    .map_err(|reason| annotate("claim", io::Error::other(reason)))
}

fn annotate(step: &'static str, error: io::Error) -> io::Error {
    let code = error.raw_os_error().unwrap_or(0);
    let label = match (step, code) {
        ("open", 5) => "open-5",
        ("open", 2) => "open-2",
        ("open", 3) => "open-3",
        ("create", 5) => "create-5",
        ("create", 3) => "create-3",
        ("claim", 5) => "claim-5",
        ("claim", 87) => "claim-87",
        ("claim", 1307) => "claim-1307",
        ("write", 5) => "write-5",
        ("attr", 5) => "attr-5",
        _ => step,
    };
    io::Error::other(label)
}

fn fault(fault: JournalFault) -> &'static str {
    softlock_core::fault_log(fault)
}

fn path_text(path: &Path) -> Result<String, ()> {
    let text = path.to_string_lossy().replace('/', "\\");
    if config_path_allowed(&text) {
        Ok(text)
    } else {
        Err(())
    }
}

fn open_config(path: &Path, create: bool) -> io::Result<HANDLE> {
    let (drive, parts) = components(path)?;
    if parts.is_empty() {
        return Err(io::Error::other("config"));
    }
    let mut directory = open_root(&drive).map_err(|error| annotate("open", error))?;
    if refuse_reparse(directory).map_err(|error| annotate("attr", error))? {
        drop_handle(directory);
        return Err(io::Error::other("reparse"));
    }
    let last = parts.len() - 1;
    for (index, name) in parts.iter().enumerate() {
        if index == last {
            break;
        }
        let next = match open_child(directory, name, DIR_OPEN, true, false) {
            Ok(handle) => handle,
            Err(error) if not_found(&error) && create => {
                match open_child_at(directory, name, DIR_CREATE, true, true, true) {
                    Ok(handle) => handle,
                    Err(error) if already_exists_io(&error) => {
                        open_child(directory, name, DIR_OPEN, true, false)
                            .map_err(|error| annotate("open", error))?
                    }
                    Err(error) => {
                        drop_handle(directory);
                        return Err(annotate("create", error));
                    }
                }
            }
            Err(error) if not_found(&error) => {
                drop_handle(directory);
                return Err(error);
            }
            Err(error) => {
                drop_handle(directory);
                return Err(annotate("open", error));
            }
        };
        drop_handle(directory);
        if refuse_reparse(next).map_err(|error| annotate("attr", error))? {
            drop_handle(next);
            return Err(io::Error::other("reparse"));
        }
        directory = next;
    }
    let name = &parts[last];
    let file = match open_child(directory, name, FILE_ACCESS, false, false) {
        Ok(handle) => handle,
        Err(error) if not_found(&error) && create => {
            match open_child_at(directory, name, FILE_ACCESS, false, true, true) {
                Ok(handle) => handle,
                Err(error) if already_exists_io(&error) => {
                    open_child(directory, name, FILE_ACCESS, false, false)
                        .map_err(|error| annotate("open", error))?
                }
                Err(error) => {
                    drop_handle(directory);
                    return Err(annotate("create", error));
                }
            }
        }
        Err(error) if not_found(&error) => {
            drop_handle(directory);
            return Err(error);
        }
        Err(error) => {
            drop_handle(directory);
            return Err(annotate("open", error));
        }
    };
    drop_handle(directory);
    if refuse_reparse(file)? {
        drop_handle(file);
        return Err(io::Error::other("reparse"));
    }
    Ok(file)
}

fn open_entry(entry: &SettingEntry) -> io::Result<HANDLE> {
    open_acl(Path::new(&entry.path), entry.target == CONFIG_DIR)
}

fn open_acl(path: &Path, directory: bool) -> io::Result<HANDLE> {
    let access = if directory {
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC | SYNCHRONIZE
    } else {
        READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES | SYNCHRONIZE
    };
    match open_with_access_kind(path, access, directory, true) {
        Ok(handle) => Ok(handle),
        Err(_) if !directory => open_config(path, false),
        Err(error) => Err(error),
    }
}

fn open_for_owner_write(path: &Path) -> io::Result<HANDLE> {
    open_with_access(path, FILE_ACCESS)
}

fn open_with_access(path: &Path, access: u32) -> io::Result<HANDLE> {
    open_with_access_kind(path, access, false, true)
}

fn open_with_access_kind(
    path: &Path,
    access: u32,
    as_directory: bool,
    traverse: bool,
) -> io::Result<HANDLE> {
    let (drive, parts) = components(path)?;
    if parts.is_empty() {
        return Err(io::Error::other("config"));
    }
    let mut directory = open_root(&drive)?;
    if refuse_reparse(directory)? {
        drop_handle(directory);
        return Err(io::Error::other("reparse"));
    }
    let parent_access = if traverse {
        DIR_OPEN
    } else {
        DIR_OPEN & !FILE_TRAVERSE
    };
    let last = parts.len() - 1;
    for name in &parts[..last] {
        let next = open_child(directory, name, parent_access, true, false)?;
        drop_handle(directory);
        if refuse_reparse(next)? {
            drop_handle(next);
            return Err(io::Error::other("reparse"));
        }
        directory = next;
    }
    let file = open_child(directory, &parts[last], access, as_directory, false);
    drop_handle(directory);
    let file = file?;
    if refuse_reparse(file)? {
        drop_handle(file);
        return Err(io::Error::other("reparse"));
    }
    Ok(file)
}

fn refuse_reparse(handle: HANDLE) -> io::Result<bool> {
    handle_is_reparse(handle)
}

fn components(path: &Path) -> io::Result<(String, Vec<String>)> {
    let text = path.to_string_lossy().replace('/', "\\");
    if text.contains("..") || text.contains('"') || text.contains('*') {
        return Err(io::Error::other("config"));
    }
    let mut parts = text
        .split('\\')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return Err(io::Error::other("config"));
    }
    let drive = parts.remove(0);
    if drive.len() != 2 || !drive.as_bytes()[0].is_ascii_alphabetic() || drive.as_bytes()[1] != b':'
    {
        return Err(io::Error::other("config"));
    }
    if parts.iter().any(|part| part == "." || part == "..") {
        return Err(io::Error::other("config"));
    }
    Ok((drive, parts))
}

fn read_handle(handle: HANDLE) -> io::Result<String> {
    let mut size = 0i64;
    unsafe { GetFileSizeEx(handle, &mut size).map_err(os_error)? };
    if size < 0 || size as u64 > MAX_CONFIG_BYTES as u64 {
        return Err(io::Error::other("config exceeds the size limit"));
    }
    let mut buffer = vec![0u8; size as usize];
    let mut read = 0u32;
    unsafe {
        ReadFile(handle, Some(buffer.as_mut_slice()), Some(&mut read), None).map_err(os_error)?
    };
    buffer.truncate(read as usize);
    String::from_utf8(buffer).map_err(|_| io::Error::other("config"))
}

fn write_handle(handle: HANDLE, bytes: &[u8]) -> io::Result<()> {
    unsafe {
        SetFilePointerEx(handle, 0, None, FILE_BEGIN).map_err(os_error)?;
    }
    let mut written = 0u32;
    unsafe { WriteFile(handle, Some(bytes), Some(&mut written), None).map_err(os_error)? };
    if written as usize != bytes.len() {
        return Err(io::Error::other("config"));
    }
    unsafe { SetEndOfFile(handle).map_err(os_error)? };
    Ok(())
}

fn owner_sid_handle(handle: HANDLE) -> io::Result<String> {
    use windows::Win32::Foundation::LocalFree;
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, GetSecurityInfo, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
    use windows::core::PWSTR;
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let mut owner = windows::Win32::Security::PSID::default();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            Some(&mut descriptor),
        )
    };
    if status.0 != 0 {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    let mut text = PWSTR::null();
    let converted = unsafe { ConvertSidToStringSidW(owner, &mut text) };
    let rendered = if converted.is_ok() && !text.is_null() {
        unsafe { text.to_string() }.unwrap_or_default()
    } else {
        String::new()
    };
    unsafe {
        if !text.is_null() {
            let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(text.0.cast())));
        }
        if !descriptor.0.is_null() {
            let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(descriptor.0)));
        }
    }
    if rendered.is_empty()
        || !rendered.starts_with("S-1-")
        || rendered.len() >= 96
        || !rendered
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        Err(io::Error::other("owner"))
    } else {
        Ok(rendered)
    }
}

fn set_dacl_handle(handle: HANDLE, sddl: &str) -> Result<(), &'static str> {
    use windows::Win32::Security::{
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
    };
    let mut info = DACL_SECURITY_INFORMATION;
    if sddl.starts_with("D:P") {
        info |= PROTECTED_DACL_SECURITY_INFORMATION;
    } else {
        info |= UNPROTECTED_DACL_SECURITY_INFORMATION;
    }
    set_security_handle(handle, sddl, info)
}

fn set_security_handle(
    handle: HANDLE,
    sddl: &str,
    information: windows::Win32::Security::OBJECT_SECURITY_INFORMATION,
) -> Result<(), &'static str> {
    use windows::Win32::Foundation::LocalFree;
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SE_FILE_OBJECT, SetSecurityInfo,
    };
    use windows::Win32::Security::{
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
        OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_RESTORE_NAME,
        SetKernelObjectSecurity,
    };
    use windows::core::{BOOL, PCWSTR};
    // The owner can set a DACL on their own file. SeRestorePrivilege and
    // SeSecurityPrivilege are enabled when the token has them, which is how
    // LocalSystem writes the protected bit. A missing privilege leaves the
    // call to fail on its own; the lock must not depend on a privilege the
    // console user was never granted if WRITE_DAC is enough.
    let _security =
        crate::secrets::enable_privilege_scoped(windows::Win32::Security::SE_SECURITY_NAME).ok();
    let _restore = crate::secrets::enable_privilege_scoped(SE_RESTORE_NAME).ok();
    let wide = sddl
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide.as_ptr()),
            1,
            &mut descriptor,
            None,
        )
        .map_err(|_| "acl")?;
    }
    let mut owner = PSID::default();
    let mut owner_defaulted = BOOL::default();
    let owner_requested = (information.0 & OWNER_SECURITY_INFORMATION.0) != 0;
    if owner_requested {
        unsafe {
            GetSecurityDescriptorOwner(descriptor, &mut owner, &mut owner_defaulted)
                .map_err(|_| "acl")?;
        }
    }
    let mut dacl: *mut windows::Win32::Security::ACL = std::ptr::null_mut();
    let mut present = BOOL::default();
    let mut dacl_defaulted = BOOL::default();
    let dacl_requested = (information.0 & DACL_SECURITY_INFORMATION.0) != 0;
    if dacl_requested {
        unsafe {
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut dacl_defaulted)
                .map_err(|_| "acl")?;
        }
    }
    // Windows reads a protected DACL back as `D:PAI`. Putting that string
    // through SetSecurityInfo with the protected bit set fails while the
    // auto-inherit control bit is also set. The ACEs stay. The protected
    // bit is the information flag below, not this control bit.
    unsafe {
        let _ = windows::Win32::Security::SetSecurityDescriptorControl(
            descriptor,
            windows::Win32::Security::SE_DACL_AUTO_INHERIT_REQ
                | windows::Win32::Security::SE_DACL_AUTO_INHERITED,
            windows::Win32::Security::SECURITY_DESCRIPTOR_CONTROL(0),
        );
    }
    let status = unsafe {
        SetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            information,
            owner_requested.then_some(owner),
            None,
            (dacl_requested && present.as_bool() && !dacl.is_null()).then_some(dacl.cast_const()),
            None,
        )
    };
    let kernel = if status.0 == 0 {
        Ok(())
    } else {
        unsafe { SetKernelObjectSecurity(handle, information, descriptor) }.map_err(|_| "acl")
    };
    unsafe {
        if !descriptor.0.is_null() {
            let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(descriptor.0)));
        }
    }
    kernel
}

#[cfg(test)]
mod create_tests {
    use std::fs;
    use std::process::Command;

    use super::open_config;
    use crate::secrets::drop_handle;

    #[test]
    fn a_missing_config_file_is_created_with_file_create() {
        let dir = std::env::temp_dir().join(format!("sl-create-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("config.toml");
        let handle = open_config(&path, true).expect("create");
        drop_handle(handle);
        assert!(path.is_file(), "the create did not leave a file");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlink_at_the_config_name_is_not_created_through() {
        let dir = std::env::temp_dir().join(format!("sl-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let target = dir.join("target.toml");
        fs::write(&target, "secret").expect("target");
        let link = dir.join("config.toml");
        let status = Command::new("cmd")
            .args([
                "/c",
                "mklink",
                &link.display().to_string(),
                &target.display().to_string(),
            ])
            .status()
            .expect("mklink");
        assert!(status.success(), "mklink did not plant the symlink");
        assert!(
            open_config(&link, true).is_err(),
            "create opened the symlink target"
        );
        assert_eq!(fs::read_to_string(&target).expect("target"), "secret");
        let _ = fs::remove_dir_all(&dir);
    }
}
