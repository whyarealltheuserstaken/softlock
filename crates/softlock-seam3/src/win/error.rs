use std::fmt;

#[derive(Debug)]
pub struct HarnessError {
    message: String,
}

impl HarnessError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HarnessError {}

pub fn explain(error: windows::core::Error, what: &str) -> HarnessError {
    HarnessError::new(format!("{what}: {error} ({:#010x})", error.code().0 as u32))
}

pub fn win32(
    code: windows::Win32::Foundation::WIN32_ERROR,
    what: &str,
) -> Result<(), HarnessError> {
    if code == windows::Win32::Foundation::ERROR_SUCCESS {
        Ok(())
    } else {
        Err(HarnessError::new(format!("{what}: win32 {}", code.0)))
    }
}

pub fn last_error(what: &str) -> HarnessError {
    let code = unsafe { windows::Win32::Foundation::GetLastError() };
    if code == windows::Win32::Foundation::ERROR_SUCCESS {
        HarnessError::new(format!("{what}: failed"))
    } else {
        HarnessError::new(format!("{what}: win32 {}", code.0))
    }
}
