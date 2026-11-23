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
    dacl_sddl_handle, delete_by_handle, drop_handle, handle_is_reparse, not_found, open_child,
    open_root, os_error,
};

const FILE_LIST_DIRECTORY: u32 = 0x0001;
const FILE_ADD_FILE: u32 = 0x0002;
const FILE_ADD_SUBDIRECTORY: u32 = 0x0004;
const FILE_TRAVERSE: u32 = 0x0020;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const DELETE: u32 = 0x0001_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;

const DIR_ACCESS: u32 = FILE_LIST_DIRECTORY
    | FILE_ADD_FILE
    | FILE_ADD_SUBDIRECTORY
    | FILE_TRAVERSE
    | READ_CONTROL
    | SYNCHRONIZE;
const FILE_ACCESS: u32 =
    GENERIC_READ | GENERIC_WRITE | READ_CONTROL | WRITE_DAC | DELETE | SYNCHRONIZE;

pub fn configured_path() -> Result<PathBuf, &'static str> {
    let path = if let Some(dir) = test_config_dir() {
        dir.join(CONFIG_FILE_NAME)
    } else {
        crate::taskmgr::owner_config_file()?
    };
    ensure_file(&path).map_err(|_| "config")?;
    Ok(path)
}

fn test_config_dir() -> Option<PathBuf> {
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
    let handle = open_config(Path::new(&entry.path), false).map_err(|_| "acl")?;
    let result = set_dacl_handle(handle, value);
    drop_handle(handle);
    result
}

pub fn restore_entry(entry: &SettingEntry) -> Result<(), &'static str> {
    let handle = open_config(Path::new(&entry.path), false).map_err(|_| "acl")?;
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
    let handle = open_config(Path::new(&entry.path), false).map_err(|_| "acl")?;
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
    let written = write_handle(handle, body.as_bytes());
    drop_handle(handle);
    written
}

fn claim_for_console_user(handle: HANDLE, sid: &str) -> io::Result<()> {
    use windows::Win32::Security::{
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        SE_RESTORE_NAME,
    };
    let _restore = crate::secrets::enable_privilege_scoped(SE_RESTORE_NAME)?;
    let sddl = format!("O:{sid}D:P(A;;FA;;;SY)(A;;FA;;;{sid})");
    set_security_handle(
        handle,
        &sddl,
        OWNER_SECURITY_INFORMATION
            | DACL_SECURITY_INFORMATION
            | PROTECTED_DACL_SECURITY_INFORMATION,
    )
    .map_err(io::Error::other)
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
    let mut directory = open_root(&drive)?;
    if refuse_reparse(directory)? {
        drop_handle(directory);
        return Err(io::Error::other("reparse"));
    }
    let last = parts.len() - 1;
    for (index, name) in parts.iter().enumerate() {
        if index == last {
            break;
        }
        let next = match open_child(directory, name, DIR_ACCESS, true, false) {
            Ok(handle) => handle,
            Err(error) if create && not_found(&error) => {
                open_child(directory, name, DIR_ACCESS, true, true)?
            }
            Err(error) => {
                drop_handle(directory);
                return Err(error);
            }
        };
        drop_handle(directory);
        if refuse_reparse(next)? {
            drop_handle(next);
            return Err(io::Error::other("reparse"));
        }
        directory = next;
    }
    let name = &parts[last];
    let file = match open_child(directory, name, FILE_ACCESS, false, false) {
        Ok(handle) => handle,
        Err(error) if create && not_found(&error) => {
            open_child(directory, name, FILE_ACCESS, false, true)?
        }
        Err(error) => {
            drop_handle(directory);
            return Err(error);
        }
    };
    drop_handle(directory);
    if refuse_reparse(file)? {
        drop_handle(file);
        return Err(io::Error::other("reparse"));
    }
    Ok(file)
}

fn open_for_owner_write(path: &Path) -> io::Result<HANDLE> {
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
        let next = open_child(directory, name, DIR_ACCESS, true, false)?;
        drop_handle(directory);
        if refuse_reparse(next)? {
            drop_handle(next);
            return Err(io::Error::other("reparse"));
        }
        directory = next;
    }
    let file = open_child(directory, &parts[last], GENERIC_WRITE, false, false);
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
    if rendered.is_empty() || !softlock_core::sid_is_safe(&rendered) {
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
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SetKernelObjectSecurity};
    use windows::core::PCWSTR;
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
    let applied = unsafe { SetKernelObjectSecurity(handle, information, descriptor) };
    unsafe {
        if !descriptor.0.is_null() {
            let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(descriptor.0)));
        }
    }
    applied.map_err(|_| "acl")
}
