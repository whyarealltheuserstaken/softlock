//! DisableTaskMgr for the console user. Reads keep the registry type.
//! An allow-listed previous value is the only one that is written back.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use softlock_core::{SettingValue, sid_is_safe};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, HANDLE};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_NOTIFY, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD,
    REG_EXPAND_SZ, REG_NOTIFY_CHANGE_LAST_SET, REG_SZ, REG_VALUE_TYPE, RegCloseKey,
    RegCreateKeyExW, RegDeleteValueW, RegLoadKeyW, RegNotifyChangeKeyValue, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW, RegUnLoadKeyW,
};
use windows::core::PCWSTR;

use crate::secrets::{enable_privilege_scoped, wide_str};

const POLICY_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\System";
const VALUE_NAME: &str = "DisableTaskMgr";

pub fn read_task_manager(sid: &str) -> Result<SettingValue, &'static str> {
    if !sid_is_safe(sid) {
        return Err("sid");
    }
    let _hive = HiveGuard::ensure(sid)?;
    let path = format!(r"{sid}\{POLICY_KEY}");
    let Some(key) = open_users(&path, KEY_QUERY_VALUE)? else {
        return Ok(SettingValue::Absent);
    };
    let value = query_dword(key);
    close(key);
    value
}

pub fn write_task_manager(sid: &str, value: SettingValue) -> Result<(), &'static str> {
    if !sid_is_safe(sid) {
        return Err("sid");
    }
    match value {
        SettingValue::Absent => delete_value(sid),
        SettingValue::Dword {
            value: dword @ (0 | 1),
        } => set_dword(sid, dword),
        SettingValue::Dword { .. } => Err("value"),
    }
}

fn set_dword(sid: &str, value: u32) -> Result<(), &'static str> {
    let _hive = HiveGuard::ensure(sid)?;
    let path = format!(r"{sid}\{POLICY_KEY}");
    let key = create_users(&path)?;
    let name = wide_str(VALUE_NAME);
    let bytes = value.to_le_bytes();
    let status =
        unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_DWORD, Some(&bytes)) };
    close(key);
    win32(status)
}

fn delete_value(sid: &str) -> Result<(), &'static str> {
    let _hive = HiveGuard::ensure(sid)?;
    let path = format!(r"{sid}\{POLICY_KEY}");
    let Some(key) = open_users(&path, KEY_SET_VALUE)? else {
        return Ok(());
    };
    let name = wide_str(VALUE_NAME);
    let status = unsafe { RegDeleteValueW(key, PCWSTR(name.as_ptr())) };
    close(key);
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err("registry")
    }
}

fn query_dword(key: HKEY) -> Result<SettingValue, &'static str> {
    let name = wide_str(VALUE_NAME);
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(SettingValue::Absent);
    }
    if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
        return Err("registry");
    }
    if kind != REG_DWORD || size < 4 {
        return Err("value");
    }
    let mut buffer = [0u8; 4];
    let mut size = 4u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(SettingValue::Absent);
    }
    win32(status)?;
    if kind != REG_DWORD {
        return Err("value");
    }
    Ok(SettingValue::Dword {
        value: u32::from_le_bytes(buffer),
    })
}

struct HiveGuard {
    sid: Option<String>,
}

impl HiveGuard {
    fn ensure(sid: &str) -> Result<Self, &'static str> {
        if let Some(key) = open_users(sid, KEY_QUERY_VALUE)? {
            close(key);
            return Ok(Self { sid: None });
        }
        let profile = profile_path(sid)?;
        let file = Path::new(&profile).join("NTUSER.DAT");
        let _backup = enable_privilege_scoped(windows::Win32::Security::SE_BACKUP_NAME)
            .map_err(|_| "hive")?;
        let _restore = enable_privilege_scoped(windows::Win32::Security::SE_RESTORE_NAME)
            .map_err(|_| "hive")?;
        let sub = wide_str(sid);
        let path = wide_str(&file.display().to_string());
        let status =
            unsafe { RegLoadKeyW(HKEY_USERS, PCWSTR(sub.as_ptr()), PCWSTR(path.as_ptr())) };
        if status != ERROR_SUCCESS {
            return Err("hive");
        }
        Ok(Self {
            sid: Some(sid.to_string()),
        })
    }
}

impl Drop for HiveGuard {
    fn drop(&mut self) {
        let Some(sid) = self.sid.take() else {
            return;
        };
        let sub = wide_str(&sid);
        let _restore = enable_privilege_scoped(windows::Win32::Security::SE_RESTORE_NAME);
        unsafe {
            let _ = RegUnLoadKeyW(HKEY_USERS, PCWSTR(sub.as_ptr()));
        }
    }
}

fn profile_path(sid: &str) -> Result<String, &'static str> {
    let key_path = format!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{sid}");
    let name = wide_str(&key_path);
    let mut opened = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(name.as_ptr()),
            None,
            KEY_QUERY_VALUE,
            &mut opened,
        )
    };
    if status != ERROR_SUCCESS {
        return Err("hive");
    }
    let value_name = wide_str("ProfileImagePath");
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            opened,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
        close(opened);
        return Err("hive");
    }
    if kind != REG_SZ && kind != REG_EXPAND_SZ {
        close(opened);
        return Err("hive");
    }
    let mut buffer = vec![0u8; size as usize + 2];
    let status = unsafe {
        RegQueryValueExW(
            opened,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr()),
            Some(&mut size),
        )
    };
    close(opened);
    win32(status)?;
    let units: Vec<u16> = buffer[..size as usize]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    let text = String::from_utf16(&units).map_err(|_| "hive")?;
    if text.is_empty() || text.contains('\0') {
        return Err("hive");
    }
    Ok(text)
}

fn open_users(
    path: &str,
    access: windows::Win32::System::Registry::REG_SAM_FLAGS,
) -> Result<Option<HKEY>, &'static str> {
    let name = wide_str(path);
    let mut opened = HKEY::default();
    let status =
        unsafe { RegOpenKeyExW(HKEY_USERS, PCWSTR(name.as_ptr()), None, access, &mut opened) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    win32(status)?;
    Ok(Some(opened))
}

fn create_users(path: &str) -> Result<HKEY, &'static str> {
    let name = wide_str(path);
    let mut opened = HKEY::default();
    let mut disposition = windows::Win32::System::Registry::REG_CREATE_KEY_DISPOSITION::default();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_USERS,
            PCWSTR(name.as_ptr()),
            None,
            None,
            windows::Win32::System::Registry::REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE | KEY_QUERY_VALUE,
            None,
            &mut opened,
            Some(&mut disposition),
        )
    };
    win32(status)?;
    Ok(opened)
}

fn close(key: HKEY) {
    unsafe {
        let _ = RegCloseKey(key);
    }
}

fn win32(status: windows::Win32::Foundation::WIN32_ERROR) -> Result<(), &'static str> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err("registry")
    }
}

pub fn console_sid() -> Result<String, &'static str> {
    let session = crate::windows::console_session();
    if session == 0xFFFF_FFFF {
        return Err("sid");
    }
    unsafe {
        let mut token = HANDLE::default();
        if windows::Win32::System::RemoteDesktop::WTSQueryUserToken(session, &mut token).is_ok() {
            let sid = token_sid(token);
            let _ = windows::Win32::Foundation::CloseHandle(token);
            return sid;
        }
    }
    process_sid()
}

fn process_sid() -> Result<String, &'static str> {
    use windows::Win32::Security::TOKEN_QUERY;
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(|_| "sid")?;
        let sid = token_sid(token);
        let _ = windows::Win32::Foundation::CloseHandle(token);
        sid
    }
}

fn token_sid(token: HANDLE) -> Result<String, &'static str> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_USER, TokenUser};
    use windows::core::PWSTR;
    unsafe {
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            return Err("sid");
        }
        let mut buffer = vec![0u8; needed as usize];
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .map_err(|_| "sid")?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).map_err(|_| "sid")?;
        let sid = text.to_string().unwrap_or_default();
        if !text.is_null() {
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        }
        if sid_is_safe(&sid) {
            Ok(sid)
        } else {
            Err("sid")
        }
    }
}

pub fn console_is_active() -> Result<bool, &'static str> {
    use windows::Win32::System::RemoteDesktop::{
        WTSActive, WTSConnectState, WTSFreeMemory, WTSQuerySessionInformationW,
    };
    use windows::core::PWSTR;
    let session = crate::windows::console_session();
    if session == 0xFFFF_FFFF {
        return Err("session");
    }
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        if WTSQuerySessionInformationW(None, session, WTSConnectState, &mut buffer, &mut bytes)
            .is_err()
            || buffer.is_null()
        {
            return Err("session");
        }
        let state = *buffer.0.cast::<u32>();
        WTSFreeMemory(buffer.0.cast());
        Ok(state == WTSActive.0 as u32)
    }
}

pub fn watch_policy(sid: &str, stop: &AtomicBool, notified: &AtomicBool) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    while !stop.load(Ordering::Acquire) {
        let path = format!(r"{sid}\{POLICY_KEY}");
        let Ok(Some(key)) = open_users(&path, KEY_NOTIFY) else {
            thread::sleep(Duration::from_secs(1));
            continue;
        };
        let Ok(event) = (unsafe { CreateEventW(None, true, false, None) }) else {
            close(key);
            thread::sleep(Duration::from_secs(1));
            continue;
        };
        let status = unsafe {
            RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, Some(event), true)
        };
        if status != ERROR_SUCCESS {
            close(key);
            unsafe {
                let _ = CloseHandle(event);
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        }
        let wait = unsafe { WaitForSingleObject(event, 1000) };
        unsafe {
            let _ = CloseHandle(event);
        }
        close(key);
        if wait == WAIT_OBJECT_0 {
            notified.store(true, Ordering::Release);
        }
    }
}

pub fn lock_host_running() -> bool {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return false;
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..PROCESSENTRY32W::default()
    };
    let mut found = false;
    unsafe {
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name = String::from_utf16_lossy(
                    &entry.szExeFile[..entry
                        .szExeFile
                        .iter()
                        .position(|unit| *unit == 0)
                        .unwrap_or(entry.szExeFile.len())],
                );
                if name.eq_ignore_ascii_case("softlock-lockhost.exe") {
                    found = true;
                    break;
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(snapshot);
    }
    found
}
