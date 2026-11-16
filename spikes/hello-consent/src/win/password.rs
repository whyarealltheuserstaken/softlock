//! Offline-capable password check for the signed-in account.
//! Network logon only. The token is closed immediately. The password is wiped.

use std::io::{self, Write};

use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows::Win32::Security::Authentication::Identity::{
    EXTENDED_NAME_FORMAT, GetUserNameExW, NameSamCompatible, NameUserPrincipal,
};
use windows::Win32::Security::{LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT, LogonUserW};
use windows::Win32::System::Console::{
    CONSOLE_MODE, ENABLE_ECHO_INPUT, GetConsoleMode, GetStdHandle, ReadConsoleW, STD_INPUT_HANDLE,
    SetConsoleMode,
};
use windows::core::{PCWSTR, PWSTR};
use zeroize::Zeroize;

pub fn run() -> i32 {
    println!("Password check for the account you are signed in as.");
    println!("This uses a Windows network logon and then closes the token.");
    println!("A wrong password counts toward the Windows account-lockout counter.");
    println!("Use the correct password. If you try a wrong one on purpose, do it once, then stop.");
    println!("If the result says the account is locked, do not try again.");
    println!("Type yes to continue.");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_err() || answer.trim() != "yes" {
        println!("cancelled");
        return 1;
    }

    let account = match session_account() {
        Some(account) => account,
        None => {
            eprintln!("could not read the signed-in account name");
            return 2;
        }
    };
    println!("account kind: {}", account.kind);
    println!("logon domain: {}", account.domain_display);
    println!("logon user: {}", account.user_display);
    println!("logon type: network (LOGON32_LOGON_NETWORK = 3)");
    println!("Type the password and press Enter. It will not be shown.");
    let _ = io::stdout().flush();

    let mut password = match read_hidden_password() {
        Ok(password) => password,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    if password.len() <= 1 {
        password.zeroize();
        println!("empty password was not sent");
        return 1;
    }

    let mut token = HANDLE::default();
    // SAFETY: the password buffer is a null-terminated wide string and is wiped
    // after the call. A token, if one is returned, is closed immediately.
    let logged_on = unsafe {
        LogonUserW(
            PCWSTR(account.user.as_ptr()),
            PCWSTR(account.domain.as_ptr()),
            PCWSTR(password.as_ptr()),
            LOGON32_LOGON_NETWORK,
            LOGON32_PROVIDER_DEFAULT,
            &mut token,
        )
    };
    password.zeroize();

    match logged_on {
        Ok(()) => {
            // SAFETY: LogonUserW wrote a token we own.
            unsafe {
                let _ = CloseHandle(token);
            }
            println!("result: password accepted");
            println!("token: closed");
            0
        }
        Err(_) => {
            // SAFETY: reads the Win32 code from the failed logon.
            let code = unsafe { GetLastError().0 };
            println!("result: rejected");
            println!("win32 error: {code} {}", error_name(code));
            if code == 1909 {
                println!("STOP. The account is locked. Do not try again.");
            }
            1
        }
    }
}

struct SessionAccount {
    kind: &'static str,
    domain_display: String,
    user_display: String,
    domain: Vec<u16>,
    user: Vec<u16>,
}

fn session_account() -> Option<SessionAccount> {
    let upn = lookup_name(NameUserPrincipal);
    let sam = lookup_name(NameSamCompatible)?;
    let (sam_domain, sam_user) = split_sam(&sam)?;
    if let Some(email) = upn.filter(|value| value.contains('@')) {
        return Some(SessionAccount {
            kind: "microsoft",
            domain_display: "MicrosoftAccount".to_string(),
            user_display: email.clone(),
            domain: wide("MicrosoftAccount"),
            user: wide(&email),
        });
    }
    if sam_domain.eq_ignore_ascii_case("MicrosoftAccount") {
        return Some(SessionAccount {
            kind: "microsoft",
            domain_display: sam_domain.clone(),
            user_display: sam_user.clone(),
            domain: wide(&sam_domain),
            user: wide(&sam_user),
        });
    }
    Some(SessionAccount {
        kind: "local",
        domain_display: sam_domain.clone(),
        user_display: sam_user.clone(),
        domain: wide(&sam_domain),
        user: wide(&sam_user),
    })
}

fn split_sam(sam: &str) -> Option<(String, String)> {
    let (domain, user) = sam.split_once('\\')?;
    if domain.is_empty() || user.is_empty() {
        return None;
    }
    Some((domain.to_string(), user.to_string()))
}

fn lookup_name(format: EXTENDED_NAME_FORMAT) -> Option<String> {
    let mut size = 0u32;
    // SAFETY: a null buffer asks for the required length.
    unsafe {
        let _ = GetUserNameExW(format, None, &mut size);
    }
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u16; size as usize];
    // SAFETY: buf is writable and size was reported by the previous call.
    let ok = unsafe { GetUserNameExW(format, Some(PWSTR(buf.as_mut_ptr())), &mut size) };
    if !ok || size == 0 {
        return None;
    }
    let text = String::from_utf16_lossy(&buf[..size as usize]);
    let text = text.trim_end_matches('\0').to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn read_hidden_password() -> Result<Vec<u16>, String> {
    // SAFETY: the console mode is restored before this function returns.
    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE).map_err(|error| error.to_string())?;
        let mut mode = CONSOLE_MODE::default();
        GetConsoleMode(input, &mut mode)
            .map_err(|_| "this prompt needs a console, so the password is not shown".to_string())?;
        let hidden = CONSOLE_MODE(mode.0 & !ENABLE_ECHO_INPUT.0);
        SetConsoleMode(input, hidden).map_err(|error| error.to_string())?;
        let mut buf = [0u16; 512];
        let mut read = 0u32;
        let result = ReadConsoleW(
            input,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut read,
            None,
        );
        let _ = SetConsoleMode(input, mode);
        result.map_err(|error| error.to_string())?;
        let end = read as usize;
        let mut password: Vec<u16> = buf[..end]
            .iter()
            .copied()
            .filter(|ch| *ch != u16::from(b'\r') && *ch != u16::from(b'\n'))
            .collect();
        password.push(0);
        buf.zeroize();
        Ok(password)
    }
}

fn wide(text: &str) -> Vec<u16> {
    let mut value: Vec<u16> = text.encode_utf16().collect();
    value.push(0);
    value
}

fn error_name(code: u32) -> &'static str {
    match code {
        1326 => "ERROR_LOGON_FAILURE",
        1311 => "ERROR_NO_LOGON_SERVERS",
        1909 => "ERROR_ACCOUNT_LOCKED_OUT",
        1331 => "ERROR_ACCOUNT_DISABLED",
        1793 => "ERROR_ACCOUNT_EXPIRED",
        1327 => "ERROR_ACCOUNT_RESTRICTION",
        1385 => "ERROR_LOGON_TYPE_NOT_GRANTED",
        1330 => "ERROR_PASSWORD_EXPIRED",
        _ => "OTHER",
    }
}
