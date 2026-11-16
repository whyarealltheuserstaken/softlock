use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use windows::Win32::Foundation::{ERROR_SUCCESS, GetLastError, WIN32_ERROR};
use windows::core::PWSTR;

pub const SERVICE_NAME: &str = "P7DesktopProbe";

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

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|unit| *unit == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn pwstr(buf: &mut [u16]) -> PWSTR {
    PWSTR::from_raw(buf.as_mut_ptr())
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

pub fn absolute_dir(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, text)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| format!("rename {}: {error}", path.display()))?;
    Ok(())
}

pub fn log_line(path: &Path, line: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path);
    if let Ok(file) = file.as_mut() {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }
}

pub fn last_error(what: &str) -> String {
    let code: WIN32_ERROR = unsafe { GetLastError() };
    if code == ERROR_SUCCESS {
        format!("{what}: failed")
    } else {
        format!("{what}: win32 {}", code.0)
    }
}

pub fn read_text(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}
