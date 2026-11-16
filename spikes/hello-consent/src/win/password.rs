//! Offline-capable password check for the signed-in account.
//! Network logon only. The token is closed immediately. The password is wiped.

use std::io::{self, Write};

use softlock_hello_spike::{LogonAccount, parse_explicit_account, session_logon};
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
use zeroize::Zeroizing;

pub fn run(explicit: Option<&str>) -> i32 {
    let account = match resolve_account(explicit) {
        Ok(account) => account,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    println!("Password check. This uses a Windows network logon and then closes the token.");
    println!("A wrong password counts toward the Windows account-lockout counter.");
    println!("Use the correct password. If you try a wrong one on purpose, do it once, then stop.");
    println!("If the result says the account is locked, do not try again.");
    println!("account kind: {}", account.kind.label());
    println!("logon form tried: {}", account.form);
    println!("logon type: network (LOGON32_LOGON_NETWORK = 3)");
    println!("Type yes to continue.");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_err() || answer.trim() != "yes" {
        println!("cancelled");
        return 1;
    }
    println!("Type the password and press Enter. It will not be shown.");
    let _ = io::stdout().flush();

    let password = match read_hidden_password() {
        Ok(password) => password,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    if password.len() <= 1 {
        println!("empty password was not sent");
        return 1;
    }

    let domain = wide(&account.domain);
    let user = wide(&account.user);
    let mut token = HANDLE::default();
    // SAFETY: the password buffer is a null-terminated wide string and is wiped
    // when `password` drops. A token, if one is returned, is closed immediately.
    let logged_on = unsafe {
        LogonUserW(
            PCWSTR(user.as_ptr()),
            PCWSTR(domain.as_ptr()),
            PCWSTR(password.as_ptr()),
            LOGON32_LOGON_NETWORK,
            LOGON32_PROVIDER_DEFAULT,
            &mut token,
        )
    };
    drop(password);

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

fn resolve_account(explicit: Option<&str>) -> Result<LogonAccount, String> {
    if let Some(raw) = explicit {
        return parse_explicit_account(raw);
    }
    let upn = lookup_name(NameUserPrincipal);
    let sam = lookup_name(NameSamCompatible)
        .ok_or_else(|| "could not read the signed-in account name".to_string())?;
    session_logon(upn.as_deref(), &sam)
        .ok_or_else(|| "could not read the signed-in account name".to_string())
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

fn read_hidden_password() -> Result<Zeroizing<Vec<u16>>, String> {
    // SAFETY: the console mode is restored before this function returns.
    // The stack buffer is wiped on every return, including a failed read.
    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE).map_err(|error| error.to_string())?;
        let mut mode = CONSOLE_MODE::default();
        GetConsoleMode(input, &mut mode)
            .map_err(|_| "this prompt needs a console, so the password is not shown".to_string())?;
        let hidden = CONSOLE_MODE(mode.0 & !ENABLE_ECHO_INPUT.0);
        SetConsoleMode(input, hidden).map_err(|error| error.to_string())?;
        let mut buf = Zeroizing::new([0u16; 512]);
        let mut read = 0u32;
        let result = ReadConsoleW(
            input,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut read,
            None,
        );
        let _ = SetConsoleMode(input, mode);
        if let Err(error) = result {
            return Err(error.to_string());
        }
        let end = (read as usize).min(buf.len());
        // Capacity covers every kept character plus the trailing NUL, so the
        // pushes below do not reallocate.
        let mut password = Zeroizing::new(Vec::with_capacity(end + 1));
        for &ch in &buf[..end] {
            if ch != u16::from(b'\r') && ch != u16::from(b'\n') {
                password.push(ch);
            }
        }
        password.push(0);
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
