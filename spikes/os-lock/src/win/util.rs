use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use windows::Win32::Foundation::{ERROR_SUCCESS, GetLastError, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
};
use windows::core::PCWSTR;

pub const SERVICE_NAME: &str = "OsLockProbe";
pub const PROBE_DIR: &str = r"C:\ProgramData\os-lock-probe";

pub fn explain(error: windows::core::Error, what: &str) -> String {
    format!("{what}: {error} ({:#010x})", error.code().0 as u32)
}

pub fn win32_code(error: &windows::core::Error) -> u32 {
    let code = error.code().0 as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 {
        code & 0xFFFF
    } else {
        code
    }
}

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn exe_path() -> Result<PathBuf, String> {
    let path = std::env::current_exe().map_err(|error| format!("current exe: {error}"))?;
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        Ok(PathBuf::from(rest))
    } else {
        Ok(path)
    }
}

pub fn flag_value(args: &[String], name: &str) -> Result<String, String> {
    let mut values = args.iter();
    while let Some(arg) = values.next() {
        if arg == name {
            let Some(value) = values.next() else {
                return Err(format!("missing value for {name}"));
            };
            if value.is_empty() {
                return Err(format!("empty value for {name}"));
            }
            return Ok(value.clone());
        }
    }
    Err(format!("missing {name}"))
}

pub fn flag_path(args: &[String], name: &str) -> Result<PathBuf, String> {
    let value = flag_value(args, name)?;
    let path = PathBuf::from(&value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path))
    }
}

pub fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

pub fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let temporary = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&temporary)
            .map_err(|error| format!("write {}: {error}", temporary.display()))?;
        file.write_all(text.as_bytes())
            .map_err(|error| format!("write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("sync {}: {error}", temporary.display()))?;
    }
    fs::rename(&temporary, path).map_err(|error| format!("rename {}: {error}", path.display()))?;
    Ok(())
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let text = serde_json::to_string(value).map_err(|error| error.to_string())?;
    write_text(path, &text)
}

pub fn last_error(what: &str) -> String {
    let code = unsafe { GetLastError() };
    format!("{what}: {code:?}")
}

pub fn success(status: WIN32_ERROR) -> bool {
    status == ERROR_SUCCESS
}

/// True when this path itself is a junction, symlink, or other reparse point.
/// A missing path is not a reparse point.
pub fn path_is_reparse(path: &Path) -> bool {
    let wide = wide(&path.to_string_lossy());
    let attributes = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        return false;
    }
    attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
}
