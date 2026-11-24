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
//! A local-account rejection does increment the bad-password count. The
//! password budget therefore stays at `threshold - 2` attempts per observation
//! window, and at zero when the threshold is 1 or 2. PIN failures are a
//! separate counter and never call this function. For a Microsoft account or
//! a domain account the local bad-password count may not be authoritative.
//! Spike 9 still has to show whether an offline Microsoft account counts the
//! same way, and whether this logon works offline for that account.
//! The UTF-16 copy is wiped with `zeroize` before return. This module does
//! not log the secret.

use std::sync::Mutex;

use softlock_core::{LockoutPolicy, Secret, unreadable_lockout_policy};
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, HLOCAL, LocalFree};
use windows::Win32::NetworkManagement::NetManagement::{
    NetApiBufferFree, NetUserGetInfo, NetUserModalsGet, USER_INFO_3, USER_MODALS_INFO_3,
};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSidToSidW};
#[cfg(feature = "simulated-physical")]
use windows::Win32::Security::LookupAccountNameW;
use windows::Win32::Security::{
    GetTokenInformation, LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT, LogonUserW,
    LookupAccountSidW, PSID, SID_NAME_USE, TOKEN_USER, TokenUser,
};
use windows::core::{PCWSTR, PWSTR};
use zeroize::Zeroize;

use crate::password_shape::{PasswordLogon, password_accepted_shape};

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
        let mut units = Vec::with_capacity(text.encode_utf16().count() + 1);
        units.extend(text.encode_utf16());
        units.push(0);
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

pub fn verify_session_password(secret: &Secret) -> PasswordLogon {
    let _gate = ATTEMPTS.lock().unwrap_or_else(|poison| poison.into_inner());
    if !password_accepted_shape(secret.expose()) {
        return PasswordLogon::Failed(1326);
    }
    let Some(password) = Wide::from_bytes(secret.expose()) else {
        return PasswordLogon::Failed(1326);
    };
    match session_identity() {
        Ok(identity) => {
            network_logon_matches(&identity.user, &identity.domain, &password, &identity.sid)
        }
        Err(()) => PasswordLogon::Failed(1311),
    }
}

/// Read the session user's bad-password count before calling `LogonUser`.
/// A locked account, a count that would reach `threshold - 1` on the next
/// failure, or a failed read refuses the attempt.
pub(crate) fn account_allows_logon(policy: LockoutPolicy) -> bool {
    let Ok(identity) = session_identity() else {
        return false;
    };
    let user = wide_as_string(&identity.user);
    let Ok(info) = read_account_lockout(&user) else {
        return false;
    };
    if info.locked_out {
        return false;
    }
    if policy.threshold == 0 {
        return true;
    }
    let stop_at = policy.threshold.saturating_sub(1);
    info.bad_password_count.saturating_add(1) < stop_at
}

/// Network logon for a named account. Seam 3 uses this on a local test
/// account. It closes the token immediately and does not unlock. It is not
/// in a default build.
#[cfg(feature = "simulated-physical")]
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

#[cfg(feature = "simulated-physical")]
pub fn account_lockout(user: &str) -> Result<AccountLockout, &'static str> {
    read_account_lockout(user)
}

fn read_account_lockout(user: &str) -> Result<AccountLockout, &'static str> {
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

#[cfg(all(test, feature = "simulated-physical"))]
mod gate_tests {
    use std::sync::Mutex;

    use super::overridden_account;

    static ENV: Mutex<()> = Mutex::new(());

    #[test]
    fn overridden_account_returns_none_when_the_runner_may_not_lock() {
        let _guard = ENV.lock().unwrap_or_else(|poison| poison.into_inner());
        let user = std::env::var("SOFTLOCK_SESSION_USER").ok();
        let domain = std::env::var("SOFTLOCK_SESSION_DOMAIN").ok();
        let actions = std::env::var("GITHUB_ACTIONS").ok();
        let runner = std::env::var("RUNNER_ENVIRONMENT").ok();
        struct Restore {
            user: Option<String>,
            domain: Option<String>,
            actions: Option<String>,
            runner: Option<String>,
        }
        impl Drop for Restore {
            fn drop(&mut self) {
                restore("SOFTLOCK_SESSION_USER", &self.user);
                restore("SOFTLOCK_SESSION_DOMAIN", &self.domain);
                restore("GITHUB_ACTIONS", &self.actions);
                restore("RUNNER_ENVIRONMENT", &self.runner);
            }
        }
        let _restore = Restore {
            user,
            domain,
            actions,
            runner,
        };
        unsafe {
            std::env::set_var("SOFTLOCK_SESSION_USER", "softlock-gate-shut");
            std::env::set_var("SOFTLOCK_SESSION_DOMAIN", ".");
            std::env::remove_var("GITHUB_ACTIONS");
            std::env::remove_var("RUNNER_ENVIRONMENT");
        }
        assert!(
            softlock_windows::runner_may_lock().is_err(),
            "the runner gate stayed open"
        );
        assert!(
            overridden_account().is_none(),
            "SOFTLOCK_SESSION_USER was used while the runner gate was shut"
        );
    }

    fn restore(name: &str, value: &Option<String>) {
        unsafe {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

fn network_logon_matches(
    user: &Wide,
    domain: &Wide,
    password: &Wide,
    expected_sid: &str,
) -> PasswordLogon {
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
        // Read the code before any other call replaces it.
        let code = unsafe { GetLastError() }.0;
        return PasswordLogon::Failed(code);
    }
    let matched = token_sid(token).ok().is_some_and(|sid| sid == expected_sid);
    let _ = unsafe { CloseHandle(token) };
    if matched {
        PasswordLogon::Verified
    } else {
        PasswordLogon::Mismatch
    }
}

fn wide_as_string(wide: &Wide) -> String {
    let units = wide
        .0
        .iter()
        .copied()
        .take_while(|unit| *unit != 0)
        .collect::<Vec<_>>();
    String::from_utf16_lossy(&units)
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

#[cfg(feature = "simulated-physical")]
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

struct ModalRead {
    policy: LockoutPolicy,
    threshold: u32,
    window_minutes: u32,
    duration_minutes: u32,
}

/// The machine lockout policy. A failed read fails closed: threshold 2, so
/// the password path allows no attempt, and a long observation window.
pub fn read_lockout_policy() -> LockoutPolicy {
    match read_modals() {
        Some(read) => {
            let threshold = read.threshold.to_string();
            let window = read.window_minutes.to_string();
            let duration = read.duration_minutes.to_string();
            crate::audit(
                "lockout",
                &[
                    ("result", "read"),
                    ("threshold", &threshold),
                    ("window_min", &window),
                    ("duration_min", &duration),
                ],
            );
            read.policy
        }
        None => {
            crate::audit("lockout", &[("result", "unreadable"), ("threshold", "2")]);
            unreadable_lockout_policy()
        }
    }
}

fn read_modals() -> Option<ModalRead> {
    let mut buffer: *mut u8 = std::ptr::null_mut();
    let status = unsafe { NetUserModalsGet(PCWSTR::null(), 3, &mut buffer) };
    if status != 0 || buffer.is_null() {
        return None;
    }
    let info = unsafe { &*buffer.cast::<USER_MODALS_INFO_3>() };
    let threshold = info.usrmod3_lockout_threshold;
    let window_minutes = info.usrmod3_lockout_observation_window;
    let duration_minutes = info.usrmod3_lockout_duration;
    unsafe {
        let _ = NetApiBufferFree(Some(buffer.cast()));
    }
    Some(ModalRead {
        policy: policy_from_minutes(threshold, window_minutes),
        threshold,
        window_minutes,
        duration_minutes,
    })
}

fn policy_from_minutes(threshold: u32, window_minutes: u32) -> LockoutPolicy {
    if threshold == 0 {
        return LockoutPolicy {
            threshold: 0,
            window: std::time::Duration::ZERO,
        };
    }
    // A zero window with lockout enabled would age every failure immediately.
    // Treat that as the long fail-closed window.
    let minutes = if window_minutes == 0 {
        99_999
    } else {
        window_minutes
    };
    LockoutPolicy {
        threshold,
        window: std::time::Duration::from_secs(u64::from(minutes) * 60),
    }
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
