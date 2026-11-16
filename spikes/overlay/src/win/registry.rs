//! Save and restore the toast switches. A crash leaves a sentinel the next run clears.

use std::sync::Mutex;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, REG_DWORD, REG_OPTION_NON_VOLATILE,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW,
    RegGetValueW, RegSetKeyValueW,
};
use windows::core::PCWSTR;

use crate::toast_plan::{
    DISABLED_TOASTS, POLICY_SUBKEY, POLICY_SUPPRESSED, POLICY_VALUE, Restore, SENTINEL_SUBKEY,
    TOAST_ENABLED_SUBKEY, TOAST_ENABLED_VALUE, restore_of,
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

pub fn suppress() -> Result<(), String> {
    let _lock = lock();
    restore_locked()?;
    let toast = get_dword(TOAST_ENABLED_SUBKEY, TOAST_ENABLED_VALUE)?;
    let policy = get_dword(POLICY_SUBKEY, POLICY_VALUE)?;
    write_sentinel("ToastPresent", u32::from(toast.is_some()))?;
    write_sentinel("ToastPrevious", toast.unwrap_or(0))?;
    write_sentinel("PolicyPresent", u32::from(policy.is_some()))?;
    write_sentinel("PolicyPrevious", policy.unwrap_or(0))?;
    write_sentinel("Active", 1)?;
    set_dword(TOAST_ENABLED_SUBKEY, TOAST_ENABLED_VALUE, DISABLED_TOASTS)?;
    set_dword(POLICY_SUBKEY, POLICY_VALUE, POLICY_SUPPRESSED)?;
    Ok(())
}

pub fn restore() -> Result<String, String> {
    let _lock = lock();
    restore_locked()
}

fn restore_locked() -> Result<String, String> {
    if get_dword(SENTINEL_SUBKEY, "Active")? != Some(1) {
        return Ok("toast settings were already restored".into());
    }
    let toast_present = get_dword(SENTINEL_SUBKEY, "ToastPresent")?.unwrap_or(0) == 1;
    let toast_previous = get_dword(SENTINEL_SUBKEY, "ToastPrevious")?;
    let policy_present = get_dword(SENTINEL_SUBKEY, "PolicyPresent")?.unwrap_or(0) == 1;
    let policy_previous = get_dword(SENTINEL_SUBKEY, "PolicyPrevious")?;
    apply(
        TOAST_ENABLED_SUBKEY,
        TOAST_ENABLED_VALUE,
        restore_of(toast_present.then_some(toast_previous.unwrap_or(1))),
    )?;
    apply(
        POLICY_SUBKEY,
        POLICY_VALUE,
        restore_of(policy_present.then_some(policy_previous.unwrap_or(0))),
    )?;
    for name in [
        "Active",
        "ToastPresent",
        "ToastPrevious",
        "PolicyPresent",
        "PolicyPrevious",
    ] {
        let _ = delete_value(SENTINEL_SUBKEY, name);
    }
    Ok("toast settings restored".into())
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
        return Err(format!("could not read {value:?}: {status:?}"));
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
            return Err(format!("could not write {value}: {status:?}"));
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

fn get_sz(
    root: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    value: &str,
) -> Result<String, String> {
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
