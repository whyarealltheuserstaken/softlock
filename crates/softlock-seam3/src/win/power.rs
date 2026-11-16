use std::process::Command;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Power::{
    PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
};
use windows::Win32::System::SystemServices::POWER_REQUEST_CONTEXT_VERSION;
use windows::Win32::System::Threading::{
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
};
use windows::core::PWSTR;

use crate::model::PowerSnapshot;
use crate::powercfg::parse_powercfg_requests;

use super::error::{HarnessError, explain};

pub struct PowerHold {
    handle: HANDLE,
}

impl PowerHold {
    /// Holds `PowerRequestSystemRequired` until dropped.
    pub fn system(reason: &str) -> Result<Self, HarnessError> {
        let mut text: Vec<u16> = reason.encode_utf16().chain(std::iter::once(0)).collect();
        let context = REASON_CONTEXT {
            Version: POWER_REQUEST_CONTEXT_VERSION,
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 {
                SimpleReasonString: PWSTR(text.as_mut_ptr()),
            },
        };
        let handle = unsafe {
            PowerCreateRequest(&context).map_err(|error| explain(error, "PowerCreateRequest"))?
        };
        unsafe {
            PowerSetRequest(handle, PowerRequestSystemRequired)
                .map_err(|error| explain(error, "PowerSetRequest"))?;
        }
        Ok(Self { handle })
    }
}

impl Drop for PowerHold {
    fn drop(&mut self) {
        unsafe {
            let _ = PowerClearRequest(self.handle, PowerRequestSystemRequired);
            let _ = CloseHandle(self.handle);
        }
    }
}

pub fn snapshot_power_requests() -> Result<PowerSnapshot, HarnessError> {
    let output = Command::new("powercfg")
        .arg("/requests")
        .output()
        .map_err(|error| HarnessError::new(format!("powercfg /requests: {error}")))?;
    if !output.status.success() {
        return Err(HarnessError::new(format!(
            "powercfg /requests exited {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let text = command_text(&output.stdout);
    Ok(PowerSnapshot {
        requests: parse_powercfg_requests(&text),
    })
}

fn command_text(bytes: &[u8]) -> String {
    let utf16 = bytes.len() >= 4 && bytes[1] == 0 && bytes[3] == 0;
    if utf16 {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}
