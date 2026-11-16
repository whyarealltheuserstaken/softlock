use crate::error::Error;
use crate::policy::{Hive, PolicyId, SYSTEM_KEY};
use crate::session::{PolicyStore, SafetyNet};
use crate::value::RecordedValue;

const RUN_ONCE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
const RUN_ONCE_NAME: &str = "spike-ctrl-alt-del";

/// The real Windows registry.
#[derive(Debug, Default)]
pub struct WindowsStore;

impl PolicyStore for WindowsStore {
    fn read(&self, policy: PolicyId) -> Result<RecordedValue, Error> {
        let info = policy.info();
        read_value(info.hive, info.key_path, info.value_name, policy.name())
    }

    fn apply(&mut self, policy: PolicyId, value: &RecordedValue) -> Result<(), Error> {
        let info = policy.info();
        write_value(
            info.hive,
            info.key_path,
            info.value_name,
            value,
            policy.name(),
        )
    }
}

/// Writes the restore command to RunOnce so the next sign-in can put values back.
pub struct RunOnceSafety {
    command: String,
}

impl RunOnceSafety {
    pub fn for_current_exe() -> Result<Self, Error> {
        let exe = std::env::current_exe()
            .map_err(|err| Error::new(format!("Could not find this program: {err}")))?;
        Ok(Self {
            command: crate::run_once_command(&exe),
        })
    }
}

impl SafetyNet for RunOnceSafety {
    fn arm(&mut self) -> Result<(), Error> {
        write_value(
            Hive::CurrentUser,
            RUN_ONCE_KEY,
            RUN_ONCE_NAME,
            &RecordedValue::reg_sz(&self.command),
            "the next-sign-in restore",
        )
    }

    fn disarm(&mut self) -> Result<(), Error> {
        delete_value(
            Hive::CurrentUser,
            RUN_ONCE_KEY,
            RUN_ONCE_NAME,
            "the next-sign-in restore",
        )
    }
}

pub fn can_write_machine_policy() -> bool {
    create_key(Hive::LocalMachine, SYSTEM_KEY, "machine policy").is_ok()
}

pub fn process_is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

pub fn relaunch_elevated() -> Result<(), Error> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let exe = std::env::current_exe()
        .map_err(|err| Error::new(format!("Could not find this program: {err}")))?;
    let params = std::env::args()
        .skip(1)
        .map(|arg| crate::quote_windows_arg(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    let operation = wide("runas");
    let file = wide(&exe.display().to_string());
    let parameters = wide(&params);
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            if params.is_empty() {
                std::ptr::null()
            } else {
                parameters.as_ptr()
            },
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (result as isize) <= 32 {
        return Err(Error::new(
            "Windows did not open an Administrator window. Nothing was changed.",
        ));
    }
    Ok(())
}

struct OpenKey(windows_sys::Win32::System::Registry::HKEY);

impl Drop for OpenKey {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::System::Registry::RegCloseKey(self.0);
            }
        }
    }
}

fn read_value(hive: Hive, key: &str, name: &str, label: &str) -> Result<RecordedValue, Error> {
    let Some(opened) = open_existing(hive, key, KEY_READ, label)? else {
        return Ok(RecordedValue::Absent);
    };
    query(&opened, name, label)
}

fn write_value(
    hive: Hive,
    key: &str,
    name: &str,
    value: &RecordedValue,
    label: &str,
) -> Result<(), Error> {
    match value {
        RecordedValue::Absent => delete_value(hive, key, name, label),
        RecordedValue::Present { reg_type, data } => {
            let opened = create_key(hive, key, label)?;
            set_raw(&opened, name, *reg_type, data, label)
        }
    }
}

fn delete_value(hive: Hive, key: &str, name: &str, label: &str) -> Result<(), Error> {
    let Some(opened) = open_existing(hive, key, KEY_WRITE, label)? else {
        return Ok(());
    };
    let wide_name = wide(name);
    let code = unsafe {
        windows_sys::Win32::System::Registry::RegDeleteValueW(opened.0, wide_name.as_ptr())
    };
    if code == ERROR_SUCCESS || code == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(win_error(code, "remove", label))
    }
}

fn query(key: &OpenKey, name: &str, label: &str) -> Result<RecordedValue, Error> {
    use windows_sys::Win32::System::Registry::RegQueryValueExW;

    let wide_name = wide(name);
    let mut reg_type = 0u32;
    let mut len = 0u32;
    let code = unsafe {
        RegQueryValueExW(
            key.0,
            wide_name.as_ptr(),
            std::ptr::null(),
            &mut reg_type,
            std::ptr::null_mut(),
            &mut len,
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(RecordedValue::Absent);
    }
    if code != ERROR_SUCCESS && code != ERROR_MORE_DATA {
        return Err(win_error(code, "read", label));
    }
    let mut data = vec![0u8; len as usize];
    let code = unsafe {
        RegQueryValueExW(
            key.0,
            wide_name.as_ptr(),
            std::ptr::null(),
            &mut reg_type,
            data.as_mut_ptr(),
            &mut len,
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(RecordedValue::Absent);
    }
    if code != ERROR_SUCCESS {
        return Err(win_error(code, "read", label));
    }
    data.truncate(len as usize);
    Ok(RecordedValue::Present { reg_type, data })
}

fn set_raw(
    key: &OpenKey,
    name: &str,
    reg_type: u32,
    data: &[u8],
    label: &str,
) -> Result<(), Error> {
    let wide_name = wide(name);
    let ptr = if data.is_empty() {
        std::ptr::null()
    } else {
        data.as_ptr()
    };
    let code = unsafe {
        windows_sys::Win32::System::Registry::RegSetValueExW(
            key.0,
            wide_name.as_ptr(),
            0,
            reg_type,
            ptr,
            data.len() as u32,
        )
    };
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(win_error(code, "write", label))
    }
}

fn open_existing(
    hive: Hive,
    subkey: &str,
    access: u32,
    label: &str,
) -> Result<Option<OpenKey>, Error> {
    use windows_sys::Win32::System::Registry::{HKEY, RegOpenKeyExW};

    let wide_key = wide(subkey);
    let mut opened: HKEY = std::ptr::null_mut();
    let code = unsafe {
        RegOpenKeyExW(
            hive_handle(hive),
            wide_key.as_ptr(),
            0,
            access | KEY_WOW64_64KEY,
            &mut opened,
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if code != ERROR_SUCCESS {
        return Err(win_error(code, "open", label));
    }
    Ok(Some(OpenKey(opened)))
}

fn create_key(hive: Hive, subkey: &str, label: &str) -> Result<OpenKey, Error> {
    use windows_sys::Win32::System::Registry::{HKEY, REG_OPTION_NON_VOLATILE, RegCreateKeyExW};

    let wide_key = wide(subkey);
    let mut opened: HKEY = std::ptr::null_mut();
    let mut disposition = 0u32;
    let code = unsafe {
        RegCreateKeyExW(
            hive_handle(hive),
            wide_key.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
            std::ptr::null(),
            &mut opened,
            &mut disposition,
        )
    };
    if code != ERROR_SUCCESS {
        return Err(win_error(code, "open", label));
    }
    Ok(OpenKey(opened))
}

fn hive_handle(hive: Hive) -> windows_sys::Win32::System::Registry::HKEY {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    match hive {
        Hive::CurrentUser => HKEY_CURRENT_USER,
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn win_error(code: u32, action: &str, label: &str) -> Error {
    if code == ERROR_ACCESS_DENIED {
        return Error::new(format!(
            "Could not {action} {label}. Open an Administrator terminal and try again."
        ));
    }
    Error::new(format!(
        "Could not {action} {label} (Windows error {code})."
    ))
}

const KEY_READ: u32 = windows_sys::Win32::System::Registry::KEY_QUERY_VALUE
    | windows_sys::Win32::System::Registry::KEY_ENUMERATE_SUB_KEYS;
const KEY_WRITE: u32 = windows_sys::Win32::System::Registry::KEY_SET_VALUE;
const KEY_WOW64_64KEY: u32 = windows_sys::Win32::System::Registry::KEY_WOW64_64KEY;
const ERROR_SUCCESS: u32 = windows_sys::Win32::Foundation::ERROR_SUCCESS;
const ERROR_FILE_NOT_FOUND: u32 = windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
const ERROR_MORE_DATA: u32 = windows_sys::Win32::Foundation::ERROR_MORE_DATA;
const ERROR_ACCESS_DENIED: u32 = windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
