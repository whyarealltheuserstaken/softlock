//! Save and restore the user toast switch.
//!
//! This writes `ToastEnabled` only. It never writes a policy value. If a toast
//! policy value already exists, suppression refuses. A RunOnce entry runs
//! `restore-toasts` from this exe after a kill or a reboot.

use std::sync::Mutex;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, KEY_READ, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteKeyValueW, RegDeleteKeyW, RegEnumValueW, RegGetValueW, RegOpenKeyExW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, PWSTR};

use crate::toast_plan::{
    DISABLED_TOASTS, POLICY_SUBKEY, POLICY_VALUE, RUN_ONCE_SUBKEY, RUN_ONCE_VALUE, Restore,
    SENTINEL_SUBKEY, TOAST_ENABLED_SUBKEY, TOAST_ENABLED_VALUE, restore_of, run_once_command,
    suppression_blocked,
};
use crate::win::wide;

static REGISTRY: Mutex<()> = Mutex::new(());

pub fn windows_build() -> Result<u32, String> {
    let text = get_sz(
        HKEY_LOCAL_MACHINE,
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "CurrentBuildNumber",
    )?;
    text.trim()
        .parse()
        .map_err(|_| format!("could not read the Windows build from {text}"))
}

pub fn suppress(exe: &str) -> Result<(), String> {
    let _lock = lock();
    restore_locked()?;
    let policies = policy_value_count()?;
    suppression_blocked(policies).map_err(str::to_string)?;
    let toast = get_dword(TOAST_ENABLED_SUBKEY, TOAST_ENABLED_VALUE)?;
    write_sentinel("ToastPresent", u32::from(toast.is_some()))?;
    write_sentinel("ToastPrevious", toast.unwrap_or(0))?;
    write_sentinel("Active", 1)?;
    set_sz(RUN_ONCE_SUBKEY, RUN_ONCE_VALUE, &run_once_command(exe))?;
    set_dword(TOAST_ENABLED_SUBKEY, TOAST_ENABLED_VALUE, DISABLED_TOASTS)?;
    Ok(())
}

pub fn restore() -> Result<String, String> {
    let _lock = lock();
    restore_locked()
}

fn restore_locked() -> Result<String, String> {
    let active = get_dword(SENTINEL_SUBKEY, "Active")? == Some(1);
    let message = if active {
        let toast_present = get_dword(SENTINEL_SUBKEY, "ToastPresent")?.unwrap_or(0) == 1;
        let toast_previous = get_dword(SENTINEL_SUBKEY, "ToastPrevious")?;
        apply(
            TOAST_ENABLED_SUBKEY,
            TOAST_ENABLED_VALUE,
            restore_of(toast_present.then_some(toast_previous.unwrap_or(1))),
        )?;
        // Older builds of this spike wrote the policy value. Put that back too,
        // then stop. New runs do not write it.
        if get_dword(SENTINEL_SUBKEY, "PolicyPresent")?.unwrap_or(0) == 1 {
            let policy_previous = get_dword(SENTINEL_SUBKEY, "PolicyPrevious")?;
            apply(
                POLICY_SUBKEY,
                POLICY_VALUE,
                restore_of(Some(policy_previous.unwrap_or(0))),
            )?;
        }
        "toast settings restored"
    } else {
        "toast settings were already restored"
    };
    delete_key(SENTINEL_SUBKEY)?;
    delete_value(RUN_ONCE_SUBKEY, RUN_ONCE_VALUE)?;
    Ok(message.into())
}

fn policy_value_count() -> Result<usize, String> {
    let user = value_count(HKEY_CURRENT_USER, POLICY_SUBKEY)?;
    let machine = value_count(HKEY_LOCAL_MACHINE, POLICY_SUBKEY)?;
    Ok(user + machine)
}

fn value_count(root: HKEY, subkey: &str) -> Result<usize, String> {
    let subkey_w = wide(subkey);
    let mut key = HKEY::default();
    let status =
        unsafe { RegOpenKeyExW(root, PCWSTR(subkey_w.as_ptr()), Some(0), KEY_READ, &mut key) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(0);
    }
    if status != ERROR_SUCCESS {
        return Err("school IT controls this, skip".into());
    }
    let mut count = 0usize;
    let mut index = 0u32;
    loop {
        let mut buf = vec![0u16; 256];
        let mut len = buf.len() as u32;
        let status = unsafe {
            RegEnumValueW(
                key,
                index,
                Some(PWSTR(buf.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS {
            let _ = unsafe { RegCloseKey(key) };
            return Err("school IT controls this, skip".into());
        }
        count += 1;
        index += 1;
        if index > 64 {
            break;
        }
    }
    let _ = unsafe { RegCloseKey(key) };
    Ok(count)
}

fn apply(subkey: &str, value: &str, action: Restore) -> Result<(), String> {
    match action {
        Restore::Write(data) => set_dword(subkey, value, data),
        Restore::Delete => delete_value(subkey, value),
    }
}

fn write_sentinel(value: &str, data: u32) -> Result<(), String> {
    set_dword(SENTINEL_SUBKEY, value, data)
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    REGISTRY.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn get_dword(subkey: &str, value: &str) -> Result<Option<u32>, String> {
    let subkey = wide(subkey);
    let value = wide(value);
    let mut data = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(format!("could not read a toast value: {status:?}"));
    }
    Ok(Some(data))
}

fn set_dword(subkey: &str, value: &str, data: u32) -> Result<(), String> {
    let subkey_w = wide(subkey);
    let value_w = wide(value);
    unsafe {
        let mut key = Default::default();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_ALL_ACCESS,
            None,
            &mut key,
            None,
        );
        if status != ERROR_SUCCESS {
            return Err(format!("could not open {subkey}: {status:?}"));
        }
        let status = RegSetKeyValueW(
            key,
            PCWSTR::null(),
            PCWSTR(value_w.as_ptr()),
            REG_DWORD.0,
            Some((&data as *const u32).cast()),
            size_of::<u32>() as u32,
        );
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS {
            return Err(format!("could not write a toast value: {status:?}"));
        }
    }
    Ok(())
}

fn set_sz(subkey: &str, value: &str, data: &str) -> Result<(), String> {
    let subkey_w = wide(subkey);
    let value_w = wide(value);
    let data_w = wide(data);
    unsafe {
        let mut key = Default::default();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_ALL_ACCESS,
            None,
            &mut key,
            None,
        );
        if status != ERROR_SUCCESS {
            return Err(format!("could not open {subkey}: {status:?}"));
        }
        let status = RegSetKeyValueW(
            key,
            PCWSTR::null(),
            PCWSTR(value_w.as_ptr()),
            REG_SZ.0,
            Some(data_w.as_ptr().cast()),
            (data_w.len() * 2) as u32,
        );
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS {
            return Err(format!("could not write a toast value: {status:?}"));
        }
    }
    Ok(())
}

fn delete_value(subkey: &str, value: &str) -> Result<(), String> {
    let subkey = wide(subkey);
    let value = wide(value);
    let status = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
        )
    };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!("could not delete a toast value: {status:?}"))
    }
}

fn delete_key(subkey: &str) -> Result<(), String> {
    let subkey = wide(subkey);
    let status = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr())) };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!("could not delete the toast sentinel: {status:?}"))
    }
}

fn get_sz(root: HKEY, subkey: &str, value: &str) -> Result<String, String> {
    let subkey = wide(subkey);
    let value = wide(value);
    let mut buf = vec![0u16; 64];
    let mut size = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!("could not read a registry string: {status:?}"));
    }
    let chars = (size as usize / 2).saturating_sub(1);
    Ok(String::from_utf16_lossy(&buf[..chars.min(buf.len())]))
}
