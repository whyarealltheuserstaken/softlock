use std::path::Path;

use crate::account::Account;
use crate::error::Error;
use crate::policy::{Hive, PolicyId, SYSTEM_KEY, split_key_id};
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

    fn missing_key_id(&self, policy: PolicyId) -> Result<Option<String>, Error> {
        let info = policy.info();
        match open_existing(info.hive, info.key_path, KEY_READ, policy.name())? {
            Some(_) => Ok(None),
            None => Ok(Some(format!("{}\\{}", info.hive.label(), info.key_path))),
        }
    }

    fn delete_key_if_empty(&mut self, key_id: &str) -> Result<(), Error> {
        let Some((hive, path)) = split_key_id(key_id) else {
            return Err(Error::new(format!("Unknown policy key {key_id}.")));
        };
        let Some(opened) = open_existing(hive, path, KEY_READ, key_id)? else {
            return Ok(());
        };
        if !key_is_empty(&opened, key_id)? {
            return Ok(());
        }
        drop(opened);
        delete_key(hive, path, key_id)
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

fn key_is_empty(key: &OpenKey, label: &str) -> Result<bool, Error> {
    use windows_sys::Win32::System::Registry::RegQueryInfoKeyW;

    let mut subkeys = 0u32;
    let mut values = 0u32;
    let code = unsafe {
        RegQueryInfoKeyW(
            key.0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut subkeys,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut values,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if code != ERROR_SUCCESS {
        return Err(win_error(code, "read", label));
    }
    Ok(subkeys == 0 && values == 0)
}

fn delete_key(hive: Hive, path: &str, label: &str) -> Result<(), Error> {
    use windows_sys::Win32::System::Registry::RegDeleteKeyExW;

    let wide_path = wide(path);
    let code =
        unsafe { RegDeleteKeyExW(hive_handle(hive), wide_path.as_ptr(), KEY_WOW64_64KEY, 0) };
    if code == ERROR_SUCCESS || code == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(win_error(code, "remove", label))
    }
}

/// The account of this process. Its HKCU hive is the one a change would write.
pub fn current_account() -> Result<Account, Error> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Security::TOKEN_QUERY;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(Error::new("Could not read this window's account."));
        }
        let account = account_from_token(token);
        CloseHandle(token);
        account
    }
}

/// The person at the keyboard, when Windows will say who that is.
pub fn console_account() -> Option<Account> {
    use windows_sys::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId;

    let session = unsafe { WTSGetActiveConsoleSessionId() };
    if session == 0 || session == u32::MAX {
        return None;
    }
    account_from_session_token(session).or_else(|| account_from_explorer(session))
}

/// Refuse to change settings when this window is a different account.
pub fn ensure_interactive_account() -> Result<(), Error> {
    crate::account::same_account(&current_account()?, console_account().as_ref())
}

/// Limit the saved-settings folder to Administrators and SYSTEM.
pub fn lock_down_directory(path: &Path) -> Result<(), Error> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_FILE_OBJECT,
        SetNamedSecurityInfoW,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
    };

    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut path_wide = wide(&absolute.display().to_string());
    let descriptor = wide("D:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)");
    unsafe {
        let mut security = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor.as_ptr(),
            SDDL_REVISION_1,
            &mut security,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(folder_lock_error());
        }
        let mut present = 0i32;
        let mut dacl = std::ptr::null_mut();
        let mut defaulted = 0i32;
        let got = GetSecurityDescriptorDacl(security, &mut present, &mut dacl, &mut defaulted);
        if got == 0 || present == 0 || dacl.is_null() {
            LocalFree(security.cast());
            return Err(folder_lock_error());
        }
        let code = SetNamedSecurityInfoW(
            path_wide.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            dacl,
            std::ptr::null(),
        );
        LocalFree(security.cast());
        if code != ERROR_SUCCESS {
            return Err(win_error(code, "lock", "the saved-settings folder"));
        }
    }
    Ok(())
}

fn folder_lock_error() -> Error {
    Error::new(
        "Could not lock the saved-settings folder. Copy the program to C:\\Temp\\softlock-spike and run it from there. Nothing was changed.",
    )
}

fn account_from_session_token(session: u32) -> Option<Account> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::RemoteDesktop::WTSQueryUserToken;

    unsafe {
        let mut token = std::ptr::null_mut();
        if WTSQueryUserToken(session, &mut token) == 0 {
            return None;
        }
        let account = account_from_token(token).ok();
        CloseHandle(token);
        account
    }
}

fn account_from_explorer(session: u32) -> Option<Account> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..PROCESSENTRY32W::default()
        };
        let mut found = None;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if exe_name(&entry.szExeFile).eq_ignore_ascii_case("explorer.exe") {
                    let mut process_session = 0u32;
                    let same_session =
                        ProcessIdToSessionId(entry.th32ProcessID, &mut process_session) != 0
                            && process_session == session;
                    if same_session {
                        let process =
                            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                        if !process.is_null() {
                            found = account_of_process(process);
                            CloseHandle(process);
                            if found.is_some() {
                                break;
                            }
                        }
                    }
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        found
    }
}

fn account_of_process(process: windows_sys::Win32::Foundation::HANDLE) -> Option<Account> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Security::TOKEN_QUERY;
    use windows_sys::Win32::System::Threading::OpenProcessToken;

    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let account = account_from_token(token).ok();
        CloseHandle(token);
        account
    }
}

fn account_from_token(token: windows_sys::Win32::Foundation::HANDLE) -> Result<Account, Error> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_USER, TokenUser};

    unsafe {
        let mut needed = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        if needed == 0 {
            return Err(Error::new("Could not read the account id."));
        }
        let mut buffer = vec![0u8; needed as usize];
        if GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        ) == 0
        {
            return Err(Error::new("Could not read the account id."));
        }
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut text = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 || text.is_null() {
            return Err(Error::new("Could not read the account id."));
        }
        let sid = utf16_ptr(text);
        LocalFree(text.cast());
        let name = lookup_account_name(user.User.Sid).unwrap_or_else(|| sid.clone());
        Ok(Account { sid, name })
    }
}

fn lookup_account_name(sid: windows_sys::Win32::Security::PSID) -> Option<String> {
    use windows_sys::Win32::Security::LookupAccountSidW;

    unsafe {
        let mut name_len = 0u32;
        let mut domain_len = 0u32;
        let mut use_name = 0;
        LookupAccountSidW(
            std::ptr::null(),
            sid,
            std::ptr::null_mut(),
            &mut name_len,
            std::ptr::null_mut(),
            &mut domain_len,
            &mut use_name,
        );
        if name_len == 0 {
            return None;
        }
        let mut name = vec![0u16; name_len as usize];
        let mut domain = vec![0u16; domain_len.max(1) as usize];
        if LookupAccountSidW(
            std::ptr::null(),
            sid,
            name.as_mut_ptr(),
            &mut name_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut use_name,
        ) == 0
        {
            return None;
        }
        let name = trim_wide(&name);
        let domain = trim_wide(&domain);
        if domain.is_empty() {
            Some(name)
        } else {
            Some(format!("{domain}\\{name}"))
        }
    }
}

fn exe_name(raw: &[u16]) -> String {
    trim_wide(raw)
}

fn trim_wide(raw: &[u16]) -> String {
    let end = raw.iter().position(|unit| *unit == 0).unwrap_or(raw.len());
    String::from_utf16_lossy(&raw[..end])
}

unsafe fn utf16_ptr(ptr: *const u16) -> String {
    let mut len = 0usize;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr, len) })
}
