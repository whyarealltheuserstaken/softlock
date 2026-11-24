//! Settings journal on disk.
//!
//! The directory is a sibling of the PIN store, under the LocalSystem profile,
//! so a standard user cannot create it. It is created with a protected access
//! list in the same call. Every use opens it without following a reparse point
//! and checks the owner, the access list, and the names inside. Files are
//! created with `FILE_CREATE` (CreateNew) through that directory handle.

use std::io;
use std::path::{Path, PathBuf};

use softlock_core::SERVICE_NAME;

use softlock_core::{journal_dacl_exact, parent_dacl_trusted};

use crate::secrets::{
    already_exists_io, create_directory_relative, create_relative, dacl_sddl_handle,
    delete_by_handle, drop_handle, enable_privilege_scoped, handle_is_reparse, link_count,
    list_directory_names, not_found, open_directory, open_relative, owner_is_trusted,
    read_handle_text, rename_relative, write_all_handle,
};
use crate::service_data_directory;

const JOURNAL_SECURITY_SYSTEM: &str = "O:SYD:P(A;;FA;;;SY)(A;;FA;;;BA)";
const JOURNAL_SECURITY_ADMINS: &str = "O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)";
const JOURNAL_FILE_SDDL: &str = "D:P(A;;FA;;;SY)(A;;FA;;;BA)";
const JOURNAL_FILE: &str = "journal.json";

pub fn journal_directory() -> PathBuf {
    let data = service_data_directory();
    match data.parent() {
        Some(parent) => parent.join(format!("{SERVICE_NAME}Journal")),
        None => data.join("journal"),
    }
}

pub fn save_journal(bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = crate::authority::test_path("SOFTLOCK_JOURNAL_DIR") {
        return save_plain(&dir, bytes);
    }
    let dir = verified_directory()?;
    let name = temp_name();
    let file = match create_relative(&name, dir, JOURNAL_FILE_SDDL) {
        Ok(file) => file,
        Err(error) => {
            drop_handle(dir);
            return Err(error);
        }
    };
    if let Err(error) = fresh_file(file) {
        let _ = delete_by_handle(file);
        drop_handle(file);
        drop_handle(dir);
        return Err(error);
    }
    if let Err(error) = write_all_handle(file, bytes) {
        let _ = delete_by_handle(file);
        drop_handle(file);
        drop_handle(dir);
        return Err(error);
    }
    if let Err(error) = existing_journal_is_plain(dir) {
        let _ = delete_by_handle(file);
        drop_handle(file);
        drop_handle(dir);
        return Err(error);
    }
    let renamed = rename_relative(file, dir, JOURNAL_FILE, true);
    drop_handle(file);
    drop_handle(dir);
    renamed
}

pub fn load_journal() -> io::Result<Option<String>> {
    if let Some(dir) = crate::authority::test_path("SOFTLOCK_JOURNAL_DIR") {
        return load_plain(&dir);
    }
    let dir = verified_directory()?;
    let opened = open_relative(dir, JOURNAL_FILE);
    let file = match opened {
        Ok(file) => file,
        Err(error) if not_found(&error) => {
            drop_handle(dir);
            return Ok(None);
        }
        Err(error) => {
            drop_handle(dir);
            return Err(error);
        }
    };
    if let Err(error) = fresh_file(file) {
        drop_handle(file);
        drop_handle(dir);
        return Err(error);
    }
    let text = read_handle_text(file);
    drop_handle(dir);
    text.map(Some)
}

fn verified_directory() -> io::Result<windows::Win32::Foundation::HANDLE> {
    let path = journal_directory();
    match open_directory(&path, true) {
        Ok(handle) => {
            if let Err(error) = verify_directory(handle) {
                drop_handle(handle);
                return Err(error);
            }
            Ok(handle)
        }
        Err(error) if not_found(&error) => {
            create_journal_directory(&path)?;
            let handle = open_directory(&path, true)?;
            if let Err(error) = verify_directory(handle) {
                drop_handle(handle);
                return Err(error);
            }
            Ok(handle)
        }
        Err(error) => Err(error),
    }
}

fn create_journal_directory(path: &Path) -> io::Result<()> {
    let Some(parent_path) = path.parent() else {
        return Err(io::Error::other("unreadable"));
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(io::Error::other("unreadable"));
    };
    let parent = open_directory(parent_path, true)?;
    if let Err(error) = trusted_parent(parent) {
        drop_handle(parent);
        return Err(error);
    }
    let _restore = match enable_privilege_scoped(windows::Win32::Security::SE_RESTORE_NAME) {
        Ok(guard) => guard,
        Err(error) => {
            drop_handle(parent);
            return Err(error);
        }
    };
    let created =
        create_directory_relative(name, parent, JOURNAL_SECURITY_SYSTEM).or_else(|error| {
            if already_exists_io(&error) {
                Ok(())
            } else {
                create_directory_relative(name, parent, JOURNAL_SECURITY_ADMINS)
            }
        });
    drop_handle(parent);
    match created {
        Ok(()) => Ok(()),
        Err(error) if already_exists_io(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

fn trusted_parent(handle: windows::Win32::Foundation::HANDLE) -> io::Result<()> {
    if handle_is_reparse(handle)? {
        return Err(io::Error::other("reparse"));
    }
    if !owner_is_trusted(handle)? {
        return Err(io::Error::other("user-owned"));
    }
    let dacl = dacl_sddl_handle(handle)?;
    if !parent_dacl_trusted(&dacl) {
        return Err(io::Error::other("acl"));
    }
    Ok(())
}

fn verify_directory(handle: windows::Win32::Foundation::HANDLE) -> io::Result<()> {
    if handle_is_reparse(handle)? {
        return Err(io::Error::other("reparse"));
    }
    if !owner_is_trusted(handle)? {
        return Err(io::Error::other("user-owned"));
    }
    let dacl = dacl_sddl_handle(handle)?;
    if !journal_dacl_exact(&dacl) {
        return Err(io::Error::other("acl"));
    }
    let names = list_directory_names(handle)?;
    for name in &names {
        if !allowed_name(name) {
            return Err(io::Error::other("planted"));
        }
    }
    for name in names {
        if name == JOURNAL_FILE {
            continue;
        }
        let file = open_relative(handle, &name)?;
        if let Err(error) = fresh_file(file) {
            drop_handle(file);
            return Err(error);
        }
        delete_by_handle(file)?;
        drop_handle(file);
    }
    Ok(())
}

fn existing_journal_is_plain(dir: windows::Win32::Foundation::HANDLE) -> io::Result<()> {
    match open_relative(dir, JOURNAL_FILE) {
        Ok(existing) => {
            let plain = fresh_file(existing);
            drop_handle(existing);
            plain
        }
        Err(error) if not_found(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

fn fresh_file(handle: windows::Win32::Foundation::HANDLE) -> io::Result<()> {
    if handle_is_reparse(handle)? {
        return Err(io::Error::other("reparse"));
    }
    if link_count(handle)? != 1 {
        return Err(io::Error::other("hardlink"));
    }
    Ok(())
}

fn allowed_name(name: &str) -> bool {
    if name == JOURNAL_FILE {
        return true;
    }
    let Some(rest) = name.strip_prefix("journal.") else {
        return false;
    };
    let Some(hex) = rest.strip_suffix(".new") else {
        return false;
    };
    hex.len() == 16 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

fn save_plain(dir: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(dir)
        && meta.file_type().is_symlink()
    {
        return Err(io::Error::other("reparse"));
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(JOURNAL_FILE);
    if let Ok(meta) = std::fs::symlink_metadata(&path)
        && meta.file_type().is_symlink()
    {
        return Err(io::Error::other("reparse"));
    }
    let temp = dir.join(temp_name());
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, &path)
}

fn load_plain(dir: &Path) -> io::Result<Option<String>> {
    let path = dir.join(JOURNAL_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(io::Error::other("reparse")),
        Ok(_) => Ok(Some(std::fs::read_to_string(path)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn temp_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mixed = nanos ^ u128::from(std::process::id());
    format!("journal.{:016x}.new", mixed as u64)
}
