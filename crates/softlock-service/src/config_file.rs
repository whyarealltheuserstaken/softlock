//! The owner's config file. It is untrusted: size-capped, and every path
//! component is opened with `FILE_OPEN_REPARSE_POINT` from the parent handle.
//! Reads, writes, and DACL changes use that same handle.

use std::io;
use std::path::{Path, PathBuf};

use softlock_core::{
    CONFIG_FILE_NAME, JournalFault, MAX_CONFIG_BYTES, SettingEntry, SettingValue, config_acl_entry,
    config_path_allowed, dacl_without_owner_write, restore_action,
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
    drop_handle(handle);
    let text = text.map_err(|_| "config")?;
    if text.len() > MAX_CONFIG_BYTES {
        return Err("config");
    }
    let previous = previous.map_err(|_| "acl")?;
    let console = crate::taskmgr::console_sid().map_err(|_| "sid")?;
    let locked = dacl_without_owner_write(&previous, &console).map_err(|_| "acl")?;
    let path_text = path_text(path).map_err(|_| "acl")?;
    config_acl_entry(&path_text, &previous, &locked).map_err(fault)
}

pub fn apply_entry(entry: &SettingEntry) -> Result<(), &'static str> {
    let SettingValue::Sddl { value } = &entry.new_value else {
        return Err("value");
    };
    let handle = open_for_acl(Path::new(&entry.path)).map_err(|_| "acl")?;
    let result = set_dacl_handle(handle, value);
    drop_handle(handle);
    result
}

pub fn restore_entry(entry: &SettingEntry) -> Result<(), &'static str> {
    let handle = open_for_acl(Path::new(&entry.path)).map_err(|_| "acl")?;
    let current = dacl_sddl_handle(handle).map_err(|_| "acl");
    let result = match current {
        Ok(current) => {
            let current_value = SettingValue::Sddl {
                value: current.clone(),
            };
            match restore_action(&entry.previous, &current_value).map_err(|_| "value")? {
                Some(SettingValue::Sddl { value }) => set_dacl_handle(handle, &value),
                Some(_) => Err("value"),
                None => Ok(()),
            }
        }
        Err(reason) => Err(reason),
    };
    drop_handle(handle);
    result
}

pub fn reassert_entry(entry: &SettingEntry) -> Result<(), &'static str> {
    let SettingValue::Sddl { value: locked } = &entry.new_value else {
        return Err("value");
    };
    let handle = open_for_acl(Path::new(&entry.path)).map_err(|_| "acl")?;
    let current = dacl_sddl_handle(handle).map_err(|_| "acl");
    let result = match current {
        Ok(current) if current == *locked => Ok(()),
        Ok(_) => set_dacl_handle(handle, locked),
        Err(reason) => Err(reason),
    };
    drop_handle(handle);
    result
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
    match open_with_access(path, access | SYNCHRONIZE) {
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

fn open_for_acl(path: &Path) -> io::Result<HANDLE> {
    match open_config(path, false) {
        Ok(handle) => Ok(handle),
        // SYSTEM still has full control. The console user's locked ACE is read.
        Err(_) => open_with_access(path, READ_CONTROL | WRITE_DAC | SYNCHRONIZE),
    }
}

fn open_for_owner_write(path: &Path) -> io::Result<HANDLE> {
    open_with_access(path, FILE_ACCESS)
}

fn open_with_access(path: &Path, access: u32) -> io::Result<HANDLE> {
    let (drive, parts) = components(path)?;
    if parts.is_empty() {
        return Err(io::Error::other("config"));
    }
    let mut directory = open_root(&drive)?;
    if refuse_reparse(directory)? {
        drop_handle(directory);
        return Err(io::Error::other("reparse"));
    }
    let last = parts.len() - 1;
    for name in &parts[..last] {
        let next = open_child(directory, name, DIR_OPEN, true, false)?;
        drop_handle(directory);
        if refuse_reparse(next)? {
            drop_handle(next);
            return Err(io::Error::other("reparse"));
        }
        directory = next;
    }
    let file = open_child(directory, &parts[last], access, false, false);
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

#[cfg(feature = "live-fail-closed")]
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
