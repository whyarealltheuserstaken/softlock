//! Foreground image path and Authenticode check. Runs off the hook thread.
//! Each process id is inspected once per Hello dialog.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use softlock_hello_spike::{DialogGate, ImageFact, PassthroughWindow, SignatureCheck};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, HWND};
use windows::Win32::Security::Cryptography::Catalog::{
    CATALOG_INFO, CryptCATAdminAcquireContext2, CryptCATAdminCalcHashFromFileHandle2,
    CryptCATAdminEnumCatalogFromHash, CryptCATAdminReleaseCatalogContext,
    CryptCATAdminReleaseContext, CryptCATCatalogInfoFromContext,
};
use windows::Win32::Security::Cryptography::{
    BCRYPT_SHA1_ALGORITHM, BCRYPT_SHA256_ALGORITHM, CERT_CONTEXT, CERT_NAME_SIMPLE_DISPLAY_TYPE,
    CertGetNameStringW,
};
use windows::Win32::Security::WinTrust::{
    CRYPT_PROVIDER_SGNR, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_CATALOG_INFO, WINTRUST_DATA,
    WINTRUST_DATA_0, WINTRUST_DATA_UICONTEXT, WINTRUST_FILE_INFO, WTD_CACHE_ONLY_URL_RETRIEVAL,
    WTD_CHOICE_CATALOG, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
    WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvSignerFromChain,
    WTHelperProvDataFromStateData, WinVerifyTrust,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
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
        let open = clock.poll(started.elapsed(), allowed);
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
    gate.end_dialog();
    hooks::set_passthrough(false);
}

fn inspect_process(pid: u32) -> ImageFact {
    let path = image_path(pid);
    let signature = if path.is_empty() {
        SignatureCheck::NotTrusted
    } else if microsoft_signed(&path) {
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

/// Embedded signature first. `WTD_CHOICE_FILE` does not consult a catalog, so
/// catalog-signed System32 binaries come back `TRUST_E_NOSIGNATURE` and need
/// the catalog fallback below.
pub fn microsoft_signed(path: &str) -> bool {
    let wide = wide_path(path);
    let embedded = verify_embedded(&wide);
    print_verify(path, "embedded", embedded);
    if embedded.trusted {
        return true;
    }
    // Only a missing embedded signature falls through. A bad embedded
    // signature is not rescued by a catalog.
    if embedded.status != TRUST_E_NOSIGNATURE {
        return false;
    }
    catalog_signed(path, &wide)
}

const TRUST_E_NOSIGNATURE: i32 = 0x800B_0100_u32 as i32;

#[derive(Clone, Copy)]
struct VerifyHit {
    status: i32,
    trusted: bool,
}

fn wide_path(path: &str) -> Vec<u16> {
    let mut wide: Vec<u16> = path.encode_utf16().collect();
    wide.push(0);
    wide
}

fn print_verify(path: &str, how: &str, hit: VerifyHit) {
    println!(
        "signature file={path} verify={how} status={} trusted={}",
        status_hex(hit.status),
        hit.trusted
    );
}

fn status_hex(status: i32) -> String {
    format!("0x{:08X}", status as u32)
}

fn verify_embedded(path: &[u16]) -> VerifyHit {
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(path.as_ptr()),
        hFile: HANDLE::default(),
        pgKnownSubject: std::ptr::null_mut(),
    };
    let mut data = trust_data(WTD_CHOICE_FILE, WINTRUST_DATA_0 { pFile: &mut file });
    call_verify(&mut data)
}

fn catalog_signed(display: &str, path: &[u16]) -> bool {
    // SHA256 catalogs first. SHA1 only when that hash is not in any catalog.
    match catalog_hash(display, path, "sha256", BCRYPT_SHA256_ALGORITHM) {
        CatalogResult::Trusted => true,
        CatalogResult::Rejected => false,
        CatalogResult::Missing => {
            catalog_hash(display, path, "sha1", BCRYPT_SHA1_ALGORITHM) == CatalogResult::Trusted
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CatalogResult {
    Trusted,
    Rejected,
    Missing,
}

fn catalog_hash(display: &str, path: &[u16], name: &str, algorithm: PCWSTR) -> CatalogResult {
    let how = format!("catalog-{name}");
    match catalog_verify(path, algorithm) {
        CatalogLookup::OpenFailed => {
            println!(
                "signature file={display} verify={how} status=none trusted=false reason=open-failed"
            );
            CatalogResult::Rejected
        }
        CatalogLookup::NoCatalog => {
            println!(
                "signature file={display} verify={how} status=none trusted=false reason=no-catalog"
            );
            CatalogResult::Missing
        }
        CatalogLookup::Checked(hit) => {
            let trusted = hit.trusted;
            print_verify(display, &how, hit);
            if trusted {
                CatalogResult::Trusted
            } else {
                CatalogResult::Rejected
            }
        }
    }
}

enum CatalogLookup {
    OpenFailed,
    NoCatalog,
    Checked(VerifyHit),
}

fn catalog_verify(path: &[u16], algorithm: PCWSTR) -> CatalogLookup {
    // SAFETY: the catalog admin, file handle, and catalog info are released
    // before this function returns. WinVerifyTrust only borrows them.
    unsafe {
        let mut admin: isize = 0;
        if CryptCATAdminAcquireContext2(&mut admin, None, algorithm, None, Some(0)).is_err() {
            return CatalogLookup::NoCatalog;
        }
        let file = CreateFileW(
            PCWSTR(path.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        );
        let Ok(file) = file else {
            let _ = CryptCATAdminReleaseContext(admin, 0);
            return CatalogLookup::OpenFailed;
        };
        let hash = match file_hash(admin, file) {
            Some(hash) => hash,
            None => {
                let _ = CloseHandle(file);
                let _ = CryptCATAdminReleaseContext(admin, 0);
                return CatalogLookup::NoCatalog;
            }
        };
        let catalog = CryptCATAdminEnumCatalogFromHash(admin, &hash, Some(0), None);
        if catalog == 0 {
            let _ = CloseHandle(file);
            let _ = CryptCATAdminReleaseContext(admin, 0);
            return CatalogLookup::NoCatalog;
        }
        let mut info = CATALOG_INFO {
            cbStruct: std::mem::size_of::<CATALOG_INFO>() as u32,
            wszCatalogFile: [0; 260],
        };
        let hit = if CryptCATCatalogInfoFromContext(catalog, &mut info, 0).is_ok() {
            CatalogLookup::Checked(verify_catalog_membership(path, file, admin, &info, &hash))
        } else {
            CatalogLookup::NoCatalog
        };
        let _ = CryptCATAdminReleaseCatalogContext(admin, catalog, 0);
        let _ = CloseHandle(file);
        let _ = CryptCATAdminReleaseContext(admin, 0);
        hit
    }
}

fn file_hash(admin: isize, file: HANDLE) -> Option<Vec<u8>> {
    // SAFETY: admin and file were just opened by the caller and are still live.
    unsafe {
        let mut len = 0u32;
        CryptCATAdminCalcHashFromFileHandle2(admin, file, &mut len, None, Some(0)).ok()?;
        if len == 0 {
            return None;
        }
        let mut hash = vec![0u8; len as usize];
        CryptCATAdminCalcHashFromFileHandle2(
            admin,
            file,
            &mut len,
            Some(hash.as_mut_ptr()),
            Some(0),
        )
        .ok()?;
        hash.truncate(len as usize);
        Some(hash)
    }
}

fn verify_catalog_membership(
    path: &[u16],
    file: HANDLE,
    admin: isize,
    info: &CATALOG_INFO,
    hash: &[u8],
) -> VerifyHit {
    let tag = member_tag(hash);
    let mut catalog = WINTRUST_CATALOG_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_CATALOG_INFO>() as u32,
        dwCatalogVersion: 0,
        pcwszCatalogFilePath: PCWSTR(info.wszCatalogFile.as_ptr()),
        pcwszMemberTag: PCWSTR(tag.as_ptr()),
        pcwszMemberFilePath: PCWSTR(path.as_ptr()),
        hMemberFile: file,
        pbCalculatedFileHash: hash.as_ptr().cast_mut(),
        cbCalculatedFileHash: hash.len() as u32,
        pcCatalogContext: std::ptr::null_mut(),
        hCatAdmin: admin,
    };
    let mut data = trust_data(
        WTD_CHOICE_CATALOG,
        WINTRUST_DATA_0 {
            pCatalog: &mut catalog,
        },
    );
    call_verify(&mut data)
}

fn member_tag(hash: &[u8]) -> Vec<u16> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut tag = Vec::with_capacity(hash.len() * 2 + 1);
    for byte in hash {
        tag.push(u16::from(HEX[(byte >> 4) as usize]));
        tag.push(u16::from(HEX[(byte & 0x0f) as usize]));
    }
    tag.push(0);
    tag
}

fn trust_data(
    choice: windows::Win32::Security::WinTrust::WINTRUST_DATA_UNION_CHOICE,
    anonymous: WINTRUST_DATA_0,
) -> WINTRUST_DATA {
    WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        pPolicyCallbackData: std::ptr::null_mut(),
        pSIPClientData: std::ptr::null_mut(),
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: choice,
        Anonymous: anonymous,
        dwStateAction: WTD_STATEACTION_VERIFY,
        hWVTStateData: HANDLE::default(),
        pwszURLReference: PWSTR::null(),
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        dwUIContext: WINTRUST_DATA_UICONTEXT(0),
        pSignatureSettings: std::ptr::null_mut(),
    }
}

fn call_verify(data: &mut WINTRUST_DATA) -> VerifyHit {
    // SAFETY: the caller owns the union target. The state is closed here.
    unsafe {
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let status = WinVerifyTrust(
            HWND::default(),
            &mut action,
            data as *mut WINTRUST_DATA as *mut std::ffi::c_void,
        );
        let trusted = status == 0 && signer_chain_is_microsoft(data.hWVTStateData);
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = WinVerifyTrust(
            HWND::default(),
            &mut action,
            data as *mut WINTRUST_DATA as *mut std::ffi::c_void,
        );
        VerifyHit { status, trusted }
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
