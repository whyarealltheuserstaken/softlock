use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_SUCCESS, GetLastError, WIN32_ERROR};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::core::PWSTR;

pub const SERVICE_NAME: &str = "FailClosedProbe";

pub fn explain(error: windows::core::Error, what: &str) -> String {
    format!("{what}: {error} win32 {}", win32_code(&error))
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

pub fn wide_ptr(value: PWSTR) -> String {
    if value.0.is_null() {
        return String::new();
    }
    unsafe {
        let mut len = 0usize;
        while *value.0.add(len) != 0 && len < 512 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(value.0, len))
    }
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

pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, text)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|error| format!("remove {}: {error}", path.display()))?;
    }
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("rename {}: {error}", path.display()))?;
    Ok(())
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))
}

pub fn log_line(path: &Path, line: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }
}

pub fn status_error(status: WIN32_ERROR, what: &str) -> Result<(), String> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("{what}: win32 {}", status.0))
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

pub fn qpc() -> u64 {
    let mut counter = 0i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut counter);
    }
    counter as u64
}

pub fn qpc_freq() -> u64 {
    let mut frequency = 0i64;
    unsafe {
        let _ = QueryPerformanceFrequency(&mut frequency);
    }
    if frequency <= 0 { 1 } else { frequency as u64 }
}

pub fn current_session() -> u32 {
    let mut session = 0u32;
    let ok = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
    if ok.is_ok() { session } else { u32::MAX }
}

pub fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
