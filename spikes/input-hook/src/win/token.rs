use windows::Win32::Foundation::{CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE};
use windows::Win32::Security::{
    AdjustTokenPrivileges, CreateWellKnownSid, DuplicateTokenEx, GetLengthSid, GetSidSubAuthority,
    GetSidSubAuthorityCount, GetTokenInformation, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, PSID,
    SE_PRIVILEGE_ENABLED, SID_AND_ATTRIBUTES, SecurityImpersonation, SetTokenInformation,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_ALL_ACCESS, TOKEN_ELEVATION_TYPE, TOKEN_LINKED_TOKEN,
    TOKEN_MANDATORY_LABEL, TOKEN_PRIVILEGES, TOKEN_QUERY, TokenElevationType, TokenIntegrityLevel,
    TokenLinkedToken, TokenPrimary, TokenSessionId, WinMediumLabelSid,
};
use windows::Win32::System::SystemServices::SE_GROUP_INTEGRITY;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PCWSTR;

use super::util::explain;

pub struct OwnedToken(pub HANDLE);

impl Drop for OwnedToken {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
            self.0 = HANDLE::default();
        }
    }
}

impl OwnedToken {
    pub fn handle(&self) -> HANDLE {
        self.0
    }
}

pub fn enable_privilege(name: PCWSTR) -> Result<(), String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|error| explain(error, "OpenProcessToken"))?;
        let mut luid = windows::Win32::Foundation::LUID::default();
        let looked = LookupPrivilegeValueW(PCWSTR::null(), name, &mut luid);
        if let Err(error) = looked {
            let _ = CloseHandle(token);
            return Err(explain(error, "LookupPrivilegeValueW"));
        }
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let adjusted = AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None);
        let last = GetLastError();
        let _ = CloseHandle(token);
        adjusted.map_err(|error| explain(error, "AdjustTokenPrivileges"))?;
        if last == ERROR_NOT_ALL_ASSIGNED {
            return Err("privilege is not present on the token".to_string());
        }
        Ok(())
    }
}

pub fn current_process_token() -> Result<OwnedToken, String> {
    open_token(TOKEN_ALL_ACCESS)
}

pub fn current_query_token() -> Result<OwnedToken, String> {
    open_token(TOKEN_QUERY)
}

fn open_token(access: windows::Win32::Security::TOKEN_ACCESS_MASK) -> Result<OwnedToken, String> {
    let mut token = HANDLE::default();
    unsafe {
        OpenProcessToken(GetCurrentProcess(), access, &mut token)
            .map_err(|error| explain(error, "OpenProcessToken"))?;
    }
    Ok(OwnedToken(token))
}

pub fn duplicate_primary(token: HANDLE) -> Result<OwnedToken, String> {
    let mut out = HANDLE::default();
    unsafe {
        DuplicateTokenEx(
            token,
            TOKEN_ALL_ACCESS,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut out,
        )
        .map_err(|error| explain(error, "DuplicateTokenEx"))?;
    }
    Ok(OwnedToken(out))
}

pub fn set_session(token: HANDLE, session: u32) -> Result<(), String> {
    unsafe {
        SetTokenInformation(
            token,
            TokenSessionId,
            &session as *const u32 as *const _,
            std::mem::size_of::<u32>() as u32,
        )
        .map_err(|error| explain(error, "TokenSessionId"))
    }
}

pub fn integrity_rid(token: HANDLE) -> Result<u32, String> {
    let mut raw = [0u64; 32];
    let mut returned = 0u32;
    unsafe {
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(raw.as_mut_ptr().cast()),
            (raw.len() * std::mem::size_of::<u64>()) as u32,
            &mut returned,
        )
        .map_err(|error| explain(error, "TokenIntegrityLevel"))?;
        let label = &*(raw.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        if label.Label.Sid.0.is_null() {
            return Err("integrity sid is null".to_string());
        }
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        if count == 0 {
            return Err("integrity sid has no subauthority".to_string());
        }
        Ok(*GetSidSubAuthority(label.Label.Sid, u32::from(count - 1)))
    }
}

pub fn elevation_type(token: HANDLE) -> Result<i32, String> {
    let mut value = TOKEN_ELEVATION_TYPE::default();
    let mut returned = 0u32;
    unsafe {
        GetTokenInformation(
            token,
            TokenElevationType,
            Some((&mut value as *mut TOKEN_ELEVATION_TYPE).cast()),
            std::mem::size_of::<TOKEN_ELEVATION_TYPE>() as u32,
            &mut returned,
        )
        .map_err(|error| explain(error, "TokenElevationType"))?;
    }
    Ok(value.0)
}

pub fn linked_token(token: HANDLE) -> Result<OwnedToken, String> {
    let mut linked = TOKEN_LINKED_TOKEN::default();
    let mut returned = 0u32;
    unsafe {
        GetTokenInformation(
            token,
            TokenLinkedToken,
            Some((&mut linked as *mut TOKEN_LINKED_TOKEN).cast()),
            std::mem::size_of::<TOKEN_LINKED_TOKEN>() as u32,
            &mut returned,
        )
        .map_err(|error| explain(error, "TokenLinkedToken"))?;
    }
    if linked.LinkedToken.0.is_null() {
        return Err("linked token is null".to_string());
    }
    Ok(OwnedToken(linked.LinkedToken))
}

pub fn duplicate_at_medium(token: HANDLE) -> Result<OwnedToken, String> {
    let dup = duplicate_primary(token)?;
    let mut sid = [0u32; 17];
    let mut sid_len = (sid.len() * std::mem::size_of::<u32>()) as u32;
    unsafe {
        CreateWellKnownSid(
            WinMediumLabelSid,
            None,
            Some(PSID(sid.as_mut_ptr().cast())),
            &mut sid_len,
        )
        .map_err(|error| explain(error, "CreateWellKnownSid"))?;
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: PSID(sid.as_mut_ptr().cast()),
                Attributes: SE_GROUP_INTEGRITY as u32,
            },
        };
        let size =
            std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(label.Label.Sid);
        SetTokenInformation(
            dup.handle(),
            TokenIntegrityLevel,
            &label as *const TOKEN_MANDATORY_LABEL as *const _,
            size,
        )
        .map_err(|error| explain(error, "TokenIntegrityLevel"))?;
    }
    Ok(dup)
}
