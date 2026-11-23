//! DisableTaskMgr for the console user. Reads keep the registry type.
//! An allow-listed previous value is the only one that is written back.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use softlock_core::{
    HostPresence, LockIdentity, LockdownPolicy, PolicyHive, SessionFact, SettingValue,
    console_user_from, expand_profile_image, fact_after_boot, fact_for_session,
    hive_target_allowed, host_identity, same_exe_path, sid_is_safe,
};
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
const EXPLORER_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";
const REG_OPTION_OPEN_LINK: u32 = 0x0000_0008;

pub fn read_lockdown(sid: &str, policy: &LockdownPolicy) -> Result<SettingValue, &'static str> {
    if !sid_is_safe(sid) {
        return Err("sid");
    }
    let key = match policy.hive {
        PolicyHive::User => {
            let _hive = HiveGuard::ensure(sid)?;
            let path = format!(r"{sid}\{}", policy.subkey);
            let Some(key) = open_users(&path, KEY_QUERY_VALUE)? else {
                return Ok(SettingValue::Absent);
            };
            key
        }
        PolicyHive::Machine => {
            let Some(key) = open_checked(HKEY_LOCAL_MACHINE, policy.subkey, KEY_QUERY_VALUE)?
            else {
                return Ok(SettingValue::Absent);
            };
            key
        }
    };
    let value = query_dword(key, policy.target);
    close(key);
    value
}

pub fn write_lockdown(
    sid: &str,
    policy: &LockdownPolicy,
    value: SettingValue,
) -> Result<(), &'static str> {
    if !sid_is_safe(sid) || !softlock_core::allowed_previous_for(policy.target, &value) {
        return Err("value");
    }
    match value {
        SettingValue::Absent => delete_lockdown(sid, policy),
        SettingValue::Dword { value } => set_lockdown(sid, policy, value),
        SettingValue::Sddl { .. } => Err("value"),
    }
}

fn set_lockdown(sid: &str, policy: &LockdownPolicy, value: u32) -> Result<(), &'static str> {
    let (root, path, hive) = rooted(sid, policy)?;
    let _hive = hive;
    let key = create_key(root, &path)?;
    let name = wide_str(policy.target);
    let bytes = value.to_le_bytes();
    let status =
        unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_DWORD, Some(&bytes)) };
    close(key);
    win32(status)
}

fn delete_lockdown(sid: &str, policy: &LockdownPolicy) -> Result<(), &'static str> {
    let (root, path, hive) = rooted(sid, policy)?;
    let _hive = hive;
    let Some(key) = open_checked(root, &path, KEY_SET_VALUE)? else {
        return Ok(());
    };
    let name = wide_str(policy.target);
    let status = unsafe { RegDeleteValueW(key, PCWSTR(name.as_ptr())) };
    close(key);
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err("registry")
    }
}

fn rooted(
    sid: &str,
    policy: &LockdownPolicy,
) -> Result<(HKEY, String, Option<HiveGuard>), &'static str> {
    match policy.hive {
        PolicyHive::User => {
            let hive = HiveGuard::ensure(sid)?;
            Ok((HKEY_USERS, format!(r"{sid}\{}", policy.subkey), Some(hive)))
        }
        PolicyHive::Machine => Ok((HKEY_LOCAL_MACHINE, policy.subkey.to_string(), None)),
    }
}

fn query_dword(key: HKEY, name: &str) -> Result<SettingValue, &'static str> {
    let wide = wide_str(name);
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(wide.as_ptr()),
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
            PCWSTR(wide.as_ptr()),
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
        if let Some(key) = open_checked(HKEY_USERS, sid, KEY_QUERY_VALUE)? {
            close(key);
            return Ok(Self { sid: None });
        }
        let raw = profile_path(sid)?;
        let root = windows_directory()?;
        let drive = system_drive(&root)?;
        let expanded = expand_profile_image(&raw, &root, &drive).map_err(|_| "hive")?;
        let profile = Path::new(&expanded);
        let file = profile.join("NTUSER.DAT");
        let profile_reparse = path_is_reparse(profile)?;
        let file_reparse = path_is_reparse(&file)?;
        hive_target_allowed(&expanded, profile_reparse, file_reparse, false).map_err(|_| "link")?;
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
    let Some(opened) = open_checked(HKEY_LOCAL_MACHINE, &key_path, KEY_QUERY_VALUE)? else {
        return Err("hive");
    };
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
    open_checked(HKEY_USERS, path, access)
}

fn open_checked(
    root: HKEY,
    path: &str,
    access: windows::Win32::System::Registry::REG_SAM_FLAGS,
) -> Result<Option<HKEY>, &'static str> {
    let mut current = root;
    let mut owned: Option<HKEY> = None;
    for part in path.split('\\').filter(|part| !part.is_empty()) {
        let name = wide_str(part);
        let mut opened = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                current,
                PCWSTR(name.as_ptr()),
                Some(REG_OPTION_OPEN_LINK),
                access | KEY_QUERY_VALUE,
                &mut opened,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            if let Some(previous) = owned {
                close(previous);
            }
            return Ok(None);
        }
        if let Err(error) = win32(status) {
            if let Some(previous) = owned {
                close(previous);
            }
            return Err(error);
        }
        if key_is_link(opened) {
            close(opened);
            if let Some(previous) = owned {
                close(previous);
            }
            return Err("link");
        }
        if let Some(previous) = owned {
            close(previous);
        }
        owned = Some(opened);
        current = opened;
    }
    Ok(owned)
}

fn key_is_link(key: HKEY) -> bool {
    let name = wide_str("SymbolicLinkValue");
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
    status == ERROR_SUCCESS && kind.0 == 6
}

fn path_is_reparse(path: &Path) -> Result<bool, &'static str> {
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, GetFileAttributesW};
    let wide = wide_str(&path.display().to_string());
    let attr = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    if attr == u32::MAX {
        return Err("hive");
    }
    Ok(attr & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
}

fn windows_directory() -> Result<String, &'static str> {
    let value = std::env::var("SystemRoot").map_err(|_| "hive")?;
    if value.contains("..") || value.contains('%') || value.contains('\0') {
        return Err("hive");
    }
    Ok(value)
}

fn system_drive(root: &str) -> Result<String, &'static str> {
    let drive = root.get(..2).ok_or("hive")?;
    if !drive.as_bytes()[0].is_ascii_alphabetic() || drive.as_bytes()[1] != b':' {
        return Err("hive");
    }
    Ok(drive.to_string())
}

fn create_key(root: HKEY, path: &str) -> Result<HKEY, &'static str> {
    let parts: Vec<&str> = path.split('\\').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return Err("registry");
    }
    let mut current = root;
    let mut owned: Option<HKEY> = None;
    for (index, part) in parts.iter().enumerate() {
        let name = wide_str(part);
        let mut opened = HKEY::default();
        let status = unsafe {
            RegCreateKeyExW(
                current,
                PCWSTR(name.as_ptr()),
                None,
                None,
                windows::Win32::System::Registry::REG_OPTION_NON_VOLATILE
                    | windows::Win32::System::Registry::REG_OPTION_OPEN_LINK,
                KEY_SET_VALUE | KEY_QUERY_VALUE,
                None,
                &mut opened,
                None,
            )
        };
        if let Err(error) = win32(status) {
            if let Some(previous) = owned {
                close(previous);
            }
            return Err(error);
        }
        if key_is_link(opened) {
            close(opened);
            if let Some(previous) = owned {
                close(previous);
            }
            return Err("link");
        }
        if let Some(previous) = owned.take() {
            close(previous);
        }
        let _ = index;
        current = opened;
        owned = Some(opened);
    }
    owned.ok_or("registry")
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

pub fn owner_config_file() -> Result<PathBuf, &'static str> {
    let sid = console_sid()?;
    let raw = profile_path(&sid)?;
    let root = windows_directory()?;
    let drive = system_drive(&root)?;
    let expanded =
        softlock_core::expand_profile_image(&raw, &root, &drive).map_err(|_| "profile")?;
    let path = format!(
        "{expanded}\\AppData\\Roaming\\{}\\{}",
        softlock_core::SERVICE_NAME,
        softlock_core::CONFIG_FILE_NAME
    );
    if !softlock_core::config_path_allowed(&path) {
        return Err("profile");
    }
    Ok(PathBuf::from(path))
}

pub struct ConsoleImpersonation {
    token: HANDLE,
}

impl Drop for ConsoleImpersonation {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Security::RevertToSelf();
            if !self.token.0.is_null() {
                let _ = windows::Win32::Foundation::CloseHandle(self.token);
            }
        }
    }
}

/// The profile is the console user's. LocalSystem may be denied on the way
/// there, so the create runs as that user and then reverts.
pub fn impersonate_console_user() -> Result<ConsoleImpersonation, &'static str> {
    use windows::Win32::Security::ImpersonateLoggedOnUser;
    let session = crate::windows::console_session();
    if session == 0xFFFF_FFFF {
        return Err("sid");
    }
    let mut token = HANDLE::default();
    unsafe {
        windows::Win32::System::RemoteDesktop::WTSQueryUserToken(session, &mut token)
            .map_err(|_| "sid")?;
        if let Err(error) = ImpersonateLoggedOnUser(token) {
            let _ = windows::Win32::Foundation::CloseHandle(token);
            let _ = error;
            return Err("sid");
        }
    }
    Ok(ConsoleImpersonation { token })
}

pub fn console_sid() -> Result<String, &'static str> {
    let session = crate::windows::console_session();
    if session == 0xFFFF_FFFF {
        return Err("sid");
    }
    let queried = unsafe {
        let mut token = HANDLE::default();
        if windows::Win32::System::RemoteDesktop::WTSQueryUserToken(session, &mut token).is_err() {
            None
        } else {
            let sid = token_sid(token).ok();
            let _ = windows::Win32::Foundation::CloseHandle(token);
            sid
        }
    };
    let process = current_process_sid().ok();
    console_user_from(queried.as_deref(), process.as_deref())
        .map(str::to_string)
        .map_err(|_| "sid")
}

fn current_process_sid() -> Result<String, &'static str> {
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

/// Boot time and the console logon, recorded into the journal at lock time.
pub fn current_lock_identity() -> Option<LockIdentity> {
    let session = crate::windows::console_session();
    if session == u32::MAX {
        return None;
    }
    let boot = unbiased_uptime().ok()?;
    let logon = logon_luid(session).ok()?;
    if boot == 0 || logon == 0 {
        return None;
    }
    Some(LockIdentity {
        boot,
        session,
        logon,
    })
}

/// Compare the journaled uptime and logon with this boot. A failed query is
/// [`SessionFact::Unknown`], which startup fails closed on. A wall-clock
/// change is not a different boot: only a smaller unbiased uptime is.
///
/// A session id that is merely absent while the console is not active is a
/// disconnect, not a new logon. SPEC section 7: that recovery never reports
/// `Unlocked`.
pub fn observe_lock(saved: Option<LockIdentity>, console_active: bool) -> SessionFact {
    let Some(saved) = saved else {
        return SessionFact::Unknown;
    };
    let listed = session_listed(saved.session).ok();
    let same_logon = match listed {
        Some(true) => match logon_luid(saved.session) {
            Err(()) => None,
            Ok(logon) => Some(logon == saved.logon),
        },
        _ => None,
    };
    let session = fact_for_session(listed, console_active, same_logon);
    fact_after_boot(saved.boot, unbiased_uptime().ok(), session)
}

fn unbiased_uptime() -> Result<u64, ()> {
    use windows::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;
    let mut time = 0u64;
    let ok = unsafe { QueryUnbiasedInterruptTime(&mut time) };
    if ok.as_bool() && time != 0 {
        Ok(time)
    } else {
        Err(())
    }
}

fn session_listed(session: u32) -> Result<bool, ()> {
    use windows::Win32::System::RemoteDesktop::{WTSEnumerateSessionsW, WTSFreeMemory};
    unsafe {
        let mut info = std::ptr::null_mut();
        let mut count = 0u32;
        WTSEnumerateSessionsW(None, 0, 1, &mut info, &mut count).map_err(|_| ())?;
        if info.is_null() {
            return Err(());
        }
        let rows = std::slice::from_raw_parts(info, count as usize);
        let found = rows.iter().any(|row| row.SessionId == session);
        WTSFreeMemory(info.cast());
        Ok(found)
    }
}

fn logon_luid(session: u32) -> Result<u64, ()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_STATISTICS, TokenStatistics};
    use windows::Win32::System::RemoteDesktop::WTSQueryUserToken;
    unsafe {
        let mut token = HANDLE::default();
        WTSQueryUserToken(session, &mut token).map_err(|_| ())?;
        let mut stats = TOKEN_STATISTICS::default();
        let mut needed = 0u32;
        let read = GetTokenInformation(
            token,
            TokenStatistics,
            Some((&mut stats as *mut TOKEN_STATISTICS).cast()),
            std::mem::size_of::<TOKEN_STATISTICS>() as u32,
            &mut needed,
        );
        let _ = CloseHandle(token);
        read.map_err(|_| ())?;
        let low = stats.AuthenticationId.LowPart as u64;
        let high = stats.AuthenticationId.HighPart as u32 as u64;
        let logon = (high << 32) | low;
        if logon == 0 { Err(()) } else { Ok(logon) }
    }
}

pub fn watch_policy(sid: &str, stop: &AtomicBool, notified: &AtomicBool) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    let mut which = 0u8;
    while !stop.load(Ordering::Acquire) {
        let opened = match which % 3 {
            0 => open_users(&format!(r"{sid}\{POLICY_KEY}"), KEY_NOTIFY),
            1 => open_users(&format!(r"{sid}\{EXPLORER_KEY}"), KEY_NOTIFY),
            _ => open_checked(HKEY_LOCAL_MACHINE, POLICY_KEY, KEY_NOTIFY),
        };
        which = which.wrapping_add(1);
        let Ok(Some(key)) = opened else {
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

pub fn lock_host_running() -> HostPresence {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return HostPresence::Unknown;
    };
    let Ok(service) = std::env::current_exe() else {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(snapshot);
        }
        return HostPresence::Unknown;
    };
    let Some(expected) = service
        .parent()
        .map(|dir| dir.join("softlock-lockhost.exe"))
    else {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(snapshot);
        }
        return HostPresence::Unknown;
    };
    let console = crate::windows::console_session();
    let console_user = console_sid().ok();
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..PROCESSENTRY32W::default()
    };
    let mut found = HostPresence::Unknown;
    unsafe {
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            found = HostPresence::Dead;
            loop {
                let name = String::from_utf16_lossy(
                    &entry.szExeFile[..entry
                        .szExeFile
                        .iter()
                        .position(|unit| *unit == 0)
                        .unwrap_or(entry.szExeFile.len())],
                );
                if name.eq_ignore_ascii_case("softlock-lockhost.exe") {
                    match host_matches(
                        entry.th32ProcessID,
                        &expected,
                        console,
                        console_user.as_deref(),
                    ) {
                        HostPresence::Alive => {
                            found = HostPresence::Alive;
                            break;
                        }
                        HostPresence::Unknown => found = HostPresence::Unknown,
                        HostPresence::Dead => {}
                    }
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

fn host_matches(
    pid: u32,
    expected: &Path,
    console: u32,
    console_user: Option<&str>,
) -> HostPresence {
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows::core::PWSTR;
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return HostPresence::Unknown;
    };
    let mut buffer = vec![0u16; 1024];
    let mut len = buffer.len() as u32;
    let image = unsafe {
        let queried = QueryFullProcessImageNameW(
            process,
            windows::Win32::System::Threading::PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        if queried.is_err() {
            let _ = windows::Win32::Foundation::CloseHandle(process);
            return HostPresence::Unknown;
        }
        String::from_utf16_lossy(&buffer[..len as usize])
    };
    let mut session = 0u32;
    let session_ok = unsafe {
        windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(pid, &mut session).is_ok()
    };
    let token = process_user_sid(process);
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(process);
    }
    host_identity(
        same_exe_path(Path::new(&image), expected),
        session_ok && session == console,
        token.ok().as_deref(),
        console_user,
    )
}

fn process_user_sid(process: HANDLE) -> Result<String, &'static str> {
    use windows::Win32::Security::TOKEN_QUERY;
    use windows::Win32::System::Threading::OpenProcessToken;
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).map_err(|_| "sid")?;
        let sid = token_sid(token);
        let _ = windows::Win32::Foundation::CloseHandle(token);
        sid
    }
}
