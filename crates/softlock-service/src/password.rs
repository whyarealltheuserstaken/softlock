//! Account-password check for the locked session.
//!
//! The Service calls `LogonUserW` with `LOGON32_LOGON_NETWORK` (3) and
//! `LOGON32_PROVIDER_DEFAULT`. That logon type authenticates the password and
//! does not create an interactive session or keep a primary token we would
//! have to hold. The token handle is closed before this function returns.
//!
//! The account is the locked console session's user, resolved from that
//! session's SID. Another account's password is a failure because the logon
//! name is that session user, and a token whose SID does not match is rejected
//! as well. A missing user, a logon API error, or a secret that is not a
//! bounded UTF-8 password is a rejection. Rejections stay locked.
//!
//! Spike 9 still has to show whether this logon works offline for a Microsoft
//! account, and whether each rejection increments Windows' bad-password count.
//! The UTF-16 copy is wiped with `zeroize` before return. This module does
//! not log the secret.

use std::sync::Mutex;

use softlock_core::{Secret, Verdict};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::NetworkManagement::NetManagement::{
    NetApiBufferFree, NetUserGetInfo, USER_INFO_3,
};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSidToSidW};
use windows::Win32::Security::{
    GetTokenInformation, LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT, LogonUserW,
    LookupAccountNameW, LookupAccountSidW, PSID, SID_NAME_USE, TOKEN_USER, TokenUser,
};
use windows::core::{PCWSTR, PWSTR};
use zeroize::Zeroize;

use crate::password_shape::password_accepted_shape;

/// One password check at a time. Parallel attempts cannot race the counter.
static ATTEMPTS: Mutex<()> = Mutex::new(());

const UF_LOCKOUT: u32 = 0x0010;

pub struct AccountLockout {
    pub bad_password_count: u32,
    pub locked_out: bool,
}

struct Wide(Vec<u16>);

impl Wide {
    fn from_str(text: &str) -> Self {
        let mut units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        units.reserve(1);
        Self(units)
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        Some(Self::from_str(text))
    }

    fn as_pcwstr(&self) -> PCWSTR {
        PCWSTR(self.0.as_ptr())
    }
}

impl Drop for Wide {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub fn verify_session_password(secret: &Secret) -> Verdict {
    let _gate = ATTEMPTS.lock().unwrap_or_else(|poison| poison.into_inner());
    if !password_accepted_shape(secret.expose()) {
        return Verdict::Rejected;
    }
    let Some(password) = Wide::from_bytes(secret.expose()) else {
        return Verdict::Rejected;
    };
    match session_identity() {
        Ok(identity) => {
            if network_logon_matches(&identity.user, &identity.domain, &password, &identity.sid) {
                Verdict::Verified
            } else {
                Verdict::Rejected
            }
        }
        Err(()) => Verdict::Rejected,
    }
}

/// Network logon for a named account. The seam 3 measurement uses this on a
/// local test account. It closes the token immediately and does not unlock.
pub fn network_logon(user: &str, domain: &str, secret: &Secret) -> bool {
    let _gate = ATTEMPTS.lock().unwrap_or_else(|poison| poison.into_inner());
    if !password_accepted_shape(secret.expose()) {
        return false;
    }
    let Some(password) = Wide::from_bytes(secret.expose()) else {
        return false;
    };
    let user = Wide::from_str(user);
    let domain = Wide::from_str(domain);
    let mut token = HANDLE::default();
    let opened = unsafe {
        LogonUserW(
            user.as_pcwstr(),
            domain.as_pcwstr(),
            password.as_pcwstr(),
            LOGON32_LOGON_NETWORK,
            LOGON32_PROVIDER_DEFAULT,
            &raw mut token,
        )
    };
    if opened.is_ok() {
        let _ = unsafe { CloseHandle(token) };
        true
    } else {
        false
    }
}

pub fn account_lockout(user: &str) -> Result<AccountLockout, &'static str> {
    let wide = Wide::from_str(user);
    let mut buffer: *mut u8 = std::ptr::null_mut();
    let status = unsafe { NetUserGetInfo(PCWSTR::null(), wide.as_pcwstr(), 3, &mut buffer) };
    if status != 0 || buffer.is_null() {
        return Err("account");
    }
    let info = unsafe { &*buffer.cast::<USER_INFO_3>() };
    let bad = info.usri3_bad_pw_count;
    let flags = account_flags(info);
    unsafe {
        let _ = NetApiBufferFree(Some(buffer.cast()));
    }
    Ok(AccountLockout {
        bad_password_count: bad,
        locked_out: flags & UF_LOCKOUT != 0,
    })
}

fn account_flags(info: &USER_INFO_3) -> u32 {
    info.usri3_flags.0
}

struct Identity {
    user: Wide,
    domain: Wide,
    sid: String,
}

fn session_identity() -> Result<Identity, ()> {
    #[cfg(feature = "simulated-physical")]
    if let Some(identity) = overridden_account() {
        return identity;
    }
    let sid = crate::taskmgr::console_sid().map_err(|_| ())?;
    let (user, domain) = names_for_sid(&sid)?;
    Ok(Identity { user, domain, sid })
}

#[cfg(feature = "simulated-physical")]
fn overridden_account() -> Option<Result<Identity, ()>> {
    let user = std::env::var("SOFTLOCK_SESSION_USER").ok()?;
    let domain = std::env::var("SOFTLOCK_SESSION_DOMAIN").unwrap_or_else(|_| ".".to_string());
    if user.is_empty()
        || domain.is_empty()
        || user.len() > 104
        || domain.len() > 255
        || user.contains('\0')
        || domain.contains('\0')
    {
        return Some(Err(()));
    }
    let sid = match sid_for_account(&user, &domain) {
        Ok(sid) => sid,
        Err(()) => return Some(Err(())),
    };
    Some(Ok(Identity {
        user: Wide::from_str(&user),
        domain: Wide::from_str(&domain),
        sid,
    }))
}

fn network_logon_matches(user: &Wide, domain: &Wide, password: &Wide, expected_sid: &str) -> bool {
    let mut token = HANDLE::default();
    let opened = unsafe {
        LogonUserW(
            user.as_pcwstr(),
            domain.as_pcwstr(),
            password.as_pcwstr(),
            LOGON32_LOGON_NETWORK,
            LOGON32_PROVIDER_DEFAULT,
            &raw mut token,
        )
    };
    if opened.is_err() {
        return false;
    }
    let matched = token_sid(token).ok().is_some_and(|sid| sid == expected_sid);
    let _ = unsafe { CloseHandle(token) };
    matched
}

fn names_for_sid(sid: &str) -> Result<(Wide, Wide), ()> {
    let sid_wide = Wide::from_str(sid);
    let mut raw = PSID::default();
    unsafe { ConvertStringSidToSidW(sid_wide.as_pcwstr(), &mut raw) }.map_err(|_| ())?;
    let result = lookup_sid(raw);
    unsafe {
        if !raw.is_invalid() {
            let _ = LocalFree(Some(HLOCAL(raw.0)));
        }
    }
    result
}

fn lookup_sid(sid: PSID) -> Result<(Wide, Wide), ()> {
    let mut name_chars = 0u32;
    let mut domain_chars = 0u32;
    let mut kind = SID_NAME_USE::default();
    let _ = unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            sid,
            None,
            &mut name_chars,
            None,
            &mut domain_chars,
            &mut kind,
        )
    };
    if name_chars == 0 || domain_chars == 0 {
        return Err(());
    }
    let mut name = vec![0u16; name_chars as usize];
    let mut domain = vec![0u16; domain_chars as usize];
    unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            sid,
            Some(PWSTR(name.as_mut_ptr())),
            &mut name_chars,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut domain_chars,
            &mut kind,
        )
    }
    .map_err(|_| ())?;
    Ok((Wide(name), Wide(domain)))
}

fn sid_for_account(user: &str, domain: &str) -> Result<String, ()> {
    let qualified = if domain == "." {
        user.to_string()
    } else {
        format!("{domain}\\{user}")
    };
    let wide = Wide::from_str(&qualified);
    let mut sid_len = 0u32;
    let mut domain_chars = 0u32;
    let mut kind = SID_NAME_USE::default();
    let _ = unsafe {
        LookupAccountNameW(
            PCWSTR::null(),
            wide.as_pcwstr(),
            None,
            &mut sid_len,
            None,
            &mut domain_chars,
            &mut kind,
        )
    };
    if sid_len == 0 {
        return Err(());
    }
    let mut sid_buf = vec![0u8; sid_len as usize];
    let mut domain = vec![0u16; domain_chars.max(1) as usize];
    unsafe {
        LookupAccountNameW(
            PCWSTR::null(),
            wide.as_pcwstr(),
            Some(PSID(sid_buf.as_mut_ptr().cast())),
            &mut sid_len,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut domain_chars,
            &mut kind,
        )
    }
    .map_err(|_| ())?;
    domain.zeroize();
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(PSID(sid_buf.as_mut_ptr().cast()), &mut text) }
        .map_err(|_| ())?;
    let value = unsafe { text.to_string() }.unwrap_or_default();
    unsafe {
        if !text.is_null() {
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        }
    }
    sid_buf.zeroize();
    if value.is_empty() { Err(()) } else { Ok(value) }
}

fn token_sid(token: HANDLE) -> Result<String, ()> {
    unsafe {
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            return Err(());
        }
        let mut buffer = vec![0u8; needed as usize];
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .map_err(|_| ())?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).map_err(|_| ())?;
        let value = text.to_string().unwrap_or_default();
        if !text.is_null() {
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        }
        buffer.zeroize();
        if value.is_empty() { Err(()) } else { Ok(value) }
    }
}
