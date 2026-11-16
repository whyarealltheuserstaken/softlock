//! Foreground image path and Authenticode check. Runs off the hook thread.
//! Each process id is inspected once per Hello dialog.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use softlock_hello_spike::{DialogGate, ImageFact, PassthroughWindow, SignatureCheck};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::Cryptography::{
    CERT_CONTEXT, CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW,
};
use windows::Win32::Security::WinTrust::{
    CRYPT_PROVIDER_SGNR, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
    WINTRUST_DATA_PROVIDER_FLAGS, WINTRUST_DATA_UICONTEXT, WINTRUST_FILE_INFO, WTD_CHOICE_FILE,
    WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust,
};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::{PCWSTR, PWSTR};

use super::hooks::{self, HELLO_RUNNING};

static POLLING: AtomicBool = AtomicBool::new(false);

pub fn spawn_poller() {
    POLLING.store(true, Ordering::SeqCst);
    thread::spawn(poll_loop);
}

pub fn stop_poller() {
    POLLING.store(false, Ordering::SeqCst);
}

fn poll_loop() {
    let windows_dir = windows_directory();
    let mut gate = DialogGate::new(windows_dir);
    let mut clock = PassthroughWindow::spec();
    let started = Instant::now();
    let mut announced = false;
    while POLLING.load(Ordering::SeqCst) && HELLO_RUNNING.load(Ordering::SeqCst) {
        let pid = foreground_pid();
        let allowed = gate.foreground_allowed(pid, inspect_process);
        let open = clock.poll(started.elapsed(), true, allowed);
        hooks::set_passthrough(open && HELLO_RUNNING.load(Ordering::SeqCst));
        if open && !announced {
            println!("passthrough open");
            announced = true;
        } else if !open && announced {
            println!("passthrough closed");
            announced = false;
        }
        thread::sleep(Duration::from_millis(50));
    }
    hooks::set_passthrough(false);
}

fn inspect_process(pid: u32) -> ImageFact {
    let path = image_path(pid);
    let signature = if path.is_empty() {
        SignatureCheck::NotTrusted
    } else if trusted_microsoft_signature(&path) {
        SignatureCheck::Trusted
    } else {
        SignatureCheck::NotTrusted
    };
    println!(
        "foreground pid={pid} path={path} trusted={}",
        signature == SignatureCheck::Trusted
    );
    ImageFact { path, signature }
}

fn foreground_pid() -> Option<u32> {
    // SAFETY: reads the foreground window of the interactive desktop.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 { None } else { Some(pid) }
    }
}

fn image_path(pid: u32) -> String {
    // SAFETY: the process handle is closed before this function returns.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        if result.is_err() || len == 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

fn windows_directory() -> String {
    let mut buf = [0u16; 260];
    // SAFETY: the buffer is writable and large enough for a Windows path.
    let written = unsafe { GetWindowsDirectoryW(Some(&mut buf)) };
    if written == 0 || written as usize >= buf.len() {
        return r"C:\Windows".to_string();
    }
    String::from_utf16_lossy(&buf[..written as usize])
}

fn trusted_microsoft_signature(path: &str) -> bool {
    // System binaries are catalog-signed. WinVerifyTrust checks that catalog,
    // and the signer chain is read from the same verify state.
    authenticode_is_microsoft(&wide_path(path))
}

fn wide_path(path: &str) -> Vec<u16> {
    let mut wide: Vec<u16> = path.encode_utf16().collect();
    wide.push(0);
    wide
}

fn authenticode_is_microsoft(path: &[u16]) -> bool {
    // SAFETY: WinVerifyTrust reads the path and the structs built below.
    // The state handle is closed before this function returns.
    unsafe {
        let mut file = WINTRUST_FILE_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: PCWSTR(path.as_ptr()),
            hFile: HANDLE::default(),
            pgKnownSubject: std::ptr::null_mut(),
        };
        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            pPolicyCallbackData: std::ptr::null_mut(),
            pSIPClientData: std::ptr::null_mut(),
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
            dwStateAction: WTD_STATEACTION_VERIFY,
            hWVTStateData: HANDLE::default(),
            pwszURLReference: PWSTR::null(),
            dwProvFlags: WINTRUST_DATA_PROVIDER_FLAGS(0),
            dwUIContext: WINTRUST_DATA_UICONTEXT(0),
            pSignatureSettings: std::ptr::null_mut(),
        };
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let status = WinVerifyTrust(
            HWND::default(),
            &mut action,
            &mut data as *mut WINTRUST_DATA as *mut std::ffi::c_void,
        );
        let microsoft = status == 0 && signer_chain_is_microsoft(data.hWVTStateData);
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = WinVerifyTrust(
            HWND::default(),
            &mut action,
            &mut data as *mut WINTRUST_DATA as *mut std::ffi::c_void,
        );
        microsoft
    }
}

fn signer_chain_is_microsoft(state: HANDLE) -> bool {
    // SAFETY: state is the handle WinVerifyTrust just filled in, and it is
    // still open. The chain pointers are only read.
    unsafe {
        let provider = WTHelperProvDataFromStateData(state);
        if provider.is_null() {
            return false;
        }
        let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
        chain_has_microsoft(signer)
    }
}

fn chain_has_microsoft(signer: *mut CRYPT_PROVIDER_SGNR) -> bool {
    if signer.is_null() {
        return false;
    }
    // SAFETY: signer and its certificate array belong to the open verify state.
    unsafe {
        let count = (*signer).csCertChain;
        let certs = (*signer).pasCertChain;
        if certs.is_null() {
            return false;
        }
        for index in 0..count {
            let context = (*certs.add(index as usize)).pCert;
            if !context.is_null() && certificate_name_has_microsoft(context.cast_mut()) {
                return true;
            }
        }
    }
    false
}

fn certificate_name_has_microsoft(cert: *mut CERT_CONTEXT) -> bool {
    let mut buf = [0u16; 256];
    // SAFETY: cert is a context owned by the WinVerifyTrust state, still open.
    let written =
        unsafe { CertGetNameStringW(cert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, Some(&mut buf)) };
    if written <= 1 {
        return false;
    }
    let name = String::from_utf16_lossy(&buf[..written as usize - 1]);
    name.to_ascii_lowercase().contains("microsoft")
}
