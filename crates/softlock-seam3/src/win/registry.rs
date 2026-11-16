use std::collections::BTreeMap;

use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_QUERY_VALUE, KEY_SET_VALUE,
    REG_BINARY, REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ, REG_QWORD, REG_SZ, REG_VALUE_TYPE,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW,
};
use windows::core::PCWSTR;

use crate::model::{RegistryData, RegistryPath, RegistryRoot, RegistrySnapshot};

use super::error::{HarnessError, win32};

pub fn snapshot_values(paths: &[RegistryPath]) -> Result<RegistrySnapshot, HarnessError> {
    let mut values = BTreeMap::new();
    for path in paths {
        if let Some(data) = read_value(path)? {
            values.insert(path.clone(), data);
        }
    }
    Ok(RegistrySnapshot { values })
}

pub fn snapshot_key(root: RegistryRoot, key: &str) -> Result<RegistrySnapshot, HarnessError> {
    let Some(opened) = open_key(root, key, KEY_QUERY_VALUE)? else {
        return Ok(RegistrySnapshot::empty());
    };
    let snapshot = enumerate(root, key, opened);
    close(opened);
    snapshot
}

pub fn put_dword(
    root: RegistryRoot,
    key: &str,
    value: &str,
    data: u32,
) -> Result<(), HarnessError> {
    let opened = create_key(root, key)?;
    let name = wide(value);
    let bytes = data.to_le_bytes();
    let result =
        unsafe { RegSetValueExW(opened, PCWSTR(name.as_ptr()), None, REG_DWORD, Some(&bytes)) };
    close(opened);
    win32(result, "RegSetValueExW")
}

pub fn delete_tree(root: RegistryRoot, key: &str) -> Result<(), HarnessError> {
    let name = wide(key);
    let result = unsafe { RegDeleteTreeW(root_handle(root), PCWSTR(name.as_ptr())) };
    if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        win32(result, "RegDeleteTreeW")
    }
}

fn read_value(path: &RegistryPath) -> Result<Option<RegistryData>, HarnessError> {
    let Some(opened) = open_key(path.root, &path.key, KEY_QUERY_VALUE)? else {
        return Ok(None);
    };
    let data = query(opened, &path.value);
    close(opened);
    data
}

fn enumerate(
    root: RegistryRoot,
    key: &str,
    opened: HKEY,
) -> Result<RegistrySnapshot, HarnessError> {
    let mut values = BTreeMap::new();
    let mut index = 0u32;
    loop {
        let mut name = vec![0u16; 256];
        let mut name_len = name.len() as u32;
        let mut kind_raw = 0u32;
        let status = unsafe {
            RegEnumValueW(
                opened,
                index,
                Some(windows::core::PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut kind_raw),
                None,
                None,
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS {
            return Err(win32(status, "RegEnumValueW").unwrap_err());
        }
        let value_name = String::from_utf16_lossy(&name[..name_len as usize]);
        let path = RegistryPath::new(root, key, value_name);
        if let Some(data) = query(opened, &path.value)? {
            values.insert(path, data);
        }
        index += 1;
    }
    Ok(RegistrySnapshot { values })
}

fn query(opened: HKEY, value: &str) -> Result<Option<RegistryData>, HarnessError> {
    let name = wide(value);
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            opened,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
        return Err(win32(status, "RegQueryValueExW").unwrap_err());
    }
    let mut buffer = vec![0u8; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            opened,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    win32(status, "RegQueryValueExW")?;
    buffer.truncate(size as usize);
    decode(kind, &buffer).map(Some)
}

fn decode(kind: REG_VALUE_TYPE, buffer: &[u8]) -> Result<RegistryData, HarnessError> {
    if kind == REG_DWORD {
        let bytes: [u8; 4] = buffer
            .get(..4)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| HarnessError::new("REG_DWORD was short"))?;
        return Ok(RegistryData::Dword(u32::from_le_bytes(bytes)));
    }
    if kind == REG_QWORD {
        let bytes: [u8; 8] = buffer
            .get(..8)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| HarnessError::new("REG_QWORD was short"))?;
        return Ok(RegistryData::Qword(u64::from_le_bytes(bytes)));
    }
    if kind == REG_SZ {
        return Ok(RegistryData::String(wide_bytes(buffer)));
    }
    if kind == REG_EXPAND_SZ {
        return Ok(RegistryData::ExpandString(wide_bytes(buffer)));
    }
    if kind == REG_MULTI_SZ {
        let text = wide_bytes(buffer);
        let parts = text
            .split('\u{0}')
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect();
        return Ok(RegistryData::MultiString(parts));
    }
    if kind == REG_BINARY {
        return Ok(RegistryData::Binary(buffer.to_vec()));
    }
    Ok(RegistryData::Binary(buffer.to_vec()))
}

fn open_key(
    root: RegistryRoot,
    key: &str,
    access: windows::Win32::System::Registry::REG_SAM_FLAGS,
) -> Result<Option<HKEY>, HarnessError> {
    let name = wide(key);
    let mut opened = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            root_handle(root),
            PCWSTR(name.as_ptr()),
            None,
            access,
            &mut opened,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    win32(status, "RegOpenKeyExW")?;
    Ok(Some(opened))
}

fn create_key(root: RegistryRoot, key: &str) -> Result<HKEY, HarnessError> {
    let name = wide(key);
    let mut opened = HKEY::default();
    let mut disposition = windows::Win32::System::Registry::REG_CREATE_KEY_DISPOSITION::default();
    let status = unsafe {
        RegCreateKeyExW(
            root_handle(root),
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
    win32(status, "RegCreateKeyExW")?;
    Ok(opened)
}

fn close(key: HKEY) {
    unsafe {
        let _ = RegCloseKey(key);
    }
}

fn root_handle(root: RegistryRoot) -> HKEY {
    match root {
        RegistryRoot::CurrentUser => HKEY_CURRENT_USER,
        RegistryRoot::LocalMachine => HKEY_LOCAL_MACHINE,
        RegistryRoot::Users => HKEY_USERS,
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_bytes(buffer: &[u8]) -> String {
    let units: Vec<u16> = buffer
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}
