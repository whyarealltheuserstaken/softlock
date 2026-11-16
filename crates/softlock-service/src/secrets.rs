//! PIN hash on disk. The running service keeps the hash in memory. The file
//! is the copy that survives a restart. On Windows its DACL is
//! `D:P(A;;FA;;;SY)`: LocalSystem only, protected, with no deny ACEs. A deny
//! for Administrators also denies SYSTEM, so the service could not read the
//! hash back.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Protected DACL, allow full control to LocalSystem, nothing else.
#[cfg(windows)]
const SYSTEM_ONLY_SDDL: &str = "D:P(A;;FA;;;SY)";

pub struct PinStore {
    path: PathBuf,
}

impl PinStore {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            path: dir.join("pin.phc"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn save(&self, hash: &str) -> io::Result<()> {
        let Some(parent) = self.path.parent() else {
            return Err(io::Error::other("pin path has no directory"));
        };
        self.write_hash(parent, hash)
    }

    #[cfg(windows)]
    fn write_hash(&self, parent: &Path, hash: &str) -> io::Result<()> {
        prepare_secret_directory(parent)?;
        let tmp = parent.join(format!("pin.{}.tmp", std::process::id()));
        if running_as_system() {
            // The temp file is created with the SYSTEM-only DACL, then renamed
            // over the destination. LocalSystem is in that DACL, so both steps
            // are allowed.
            write_secret_file(&tmp, hash.as_bytes())?;
            replace_file(&tmp, &self.path)?;
            ensure_system_owner(&self.path)?;
            if !system_only_dacl(&self.path)? {
                return Err(io::Error::other("pin dacl is not system-only"));
            }
            return Ok(());
        }
        // A user token is not in the SYSTEM-only DACL, so creating the temp
        // file with that DACL is denied (and newer Windows reports the deny
        // as an HRESULT, not raw error 5). Write and rename while the user
        // still has access, then replace the DACL.
        fs::write(&tmp, hash.as_bytes())?;
        replace_file(&tmp, &self.path)?;
        apply_system_dacl(&self.path)?;
        Ok(())
    }

    #[cfg(not(windows))]
    fn write_hash(&self, parent: &Path, hash: &str) -> io::Result<()> {
        fs::create_dir_all(parent)?;
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, hash.as_bytes())?;
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&tmp)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&tmp, permissions)?;
        fs::rename(&tmp, &self.path)
    }

    pub fn load(&self) -> io::Result<Option<String>> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// Save, read back, save a second value, read that back.
///
/// Compiled only into the simulated-physical test build. The release binary
/// does not contain this path.
#[cfg(all(windows, feature = "simulated-physical"))]
pub fn exercise_pin_store() -> String {
    let store = PinStore::in_dir(&crate::pin_directory());
    let first = "phc-round-1";
    let second = "phc-round-2";
    if let Err(error) = store.save(first) {
        return format!("err:save1:{error}");
    }
    if store.load().ok().flatten().as_deref() != Some(first) {
        return "err:load1".to_string();
    }
    if let Err(error) = store.save(second) {
        return format!("err:save2:{error}");
    }
    if store.load().ok().flatten().as_deref() != Some(second) {
        return "err:load2".to_string();
    }
    "ok".to_string()
}

/// True when a normal read of the hash file is denied.
#[cfg(windows)]
pub fn user_read_denied(path: &Path) -> bool {
    matches!(fs::File::open(path), Err(error) if error.raw_os_error() == Some(5))
}

#[cfg(windows)]
fn prepare_secret_directory(dir: &Path) -> io::Result<()> {
    if !running_as_system() {
        fs::create_dir_all(dir)?;
        return Ok(());
    }
    let _ = enable_privilege(windows::Win32::Security::SE_TAKE_OWNERSHIP_NAME);
    let _ = enable_privilege(windows::Win32::Security::SE_RESTORE_NAME);
    if !dir.exists() {
        create_directory(dir)?;
    }
    ensure_system_owner(dir)?;
    apply_system_dacl(dir)?;
    ensure_system_owner(dir)?;
    Ok(())
}

#[cfg(windows)]
fn write_secret_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let handle = create_file_with_dacl(path)?;
    write_handle(handle, bytes)
}

#[cfg(windows)]
fn write_handle(handle: windows::Win32::Foundation::HANDLE, bytes: &[u8]) -> io::Result<()> {
    use std::os::windows::io::FromRawHandle;
    let mut file = unsafe { fs::File::from_raw_handle(handle.0) };
    use std::io::Write;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;
    if to.exists() && running_as_system() {
        let _ = ensure_system_owner(to);
        let _ = apply_system_dacl(to);
    }
    let source = wide(from);
    let dest = wide(to);
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(dest.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|error| io::Error::from_raw_os_error(error.code().0))?;
    }
    let _ = fs::remove_file(from);
    Ok(())
}

#[cfg(windows)]
fn create_directory(path: &Path) -> io::Result<()> {
    use windows::Win32::Security::SECURITY_ATTRIBUTES;
    use windows::Win32::Storage::FileSystem::CreateDirectoryW;
    use windows::core::PCWSTR;
    let descriptor = security_descriptor()?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let wide_path = wide(path);
    let created = unsafe { CreateDirectoryW(PCWSTR(wide_path.as_ptr()), Some(&sa)) };
    free_descriptor(&descriptor);
    created.map_err(|error| io::Error::from_raw_os_error(error.code().0))
}

#[cfg(windows)]
fn create_file_with_dacl(path: &Path) -> io::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Security::SECURITY_ATTRIBUTES;
    use windows::Win32::Storage::FileSystem::{
        CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_SHARE_DELETE, FILE_SHARE_READ,
    };
    use windows::core::PCWSTR;
    let descriptor = security_descriptor()?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let wide_path = wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            Some(&sa),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };
    free_descriptor(&descriptor);
    handle.map_err(|error| io::Error::from_raw_os_error(error.code().0))
}

#[cfg(windows)]
fn apply_system_dacl(path: &Path) -> io::Result<()> {
    use windows::Win32::Security::Authorization::{SE_FILE_OBJECT, SetNamedSecurityInfoW};
    use windows::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl,
        PROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows::core::PCWSTR;
    let descriptor = security_descriptor()?;
    unsafe {
        let mut present = windows::core::BOOL::default();
        let mut dacl = std::ptr::null_mut();
        let mut defaulted = windows::core::BOOL::default();
        if GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted).is_err()
            || !present.as_bool()
        {
            free_descriptor(&descriptor);
            return Err(io::Error::other("security descriptor has no dacl"));
        }
        let wide_path = wide(path);
        let status = SetNamedSecurityInfoW(
            PCWSTR(wide_path.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl as *const ACL),
            None,
        );
        free_descriptor(&descriptor);
        if status.0 != 0 {
            return Err(io::Error::from_raw_os_error(status.0 as i32));
        }
    }
    Ok(())
}

/// Owner must be LocalSystem. A directory or file a user planted is repaired
/// by taking ownership. If that fails, the save is rejected.
#[cfg(windows)]
fn ensure_system_owner(path: &Path) -> io::Result<()> {
    if owner_is_system(path)? {
        return Ok(());
    }
    set_owner_system(path)?;
    if owner_is_system(path)? {
        Ok(())
    } else {
        Err(io::Error::other("owner is not LocalSystem"))
    }
}

#[cfg(windows)]
fn owner_is_system(path: &Path) -> io::Result<bool> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows::Win32::Security::{
        CreateWellKnownSid, EqualSid, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        WinLocalSystemSid,
    };
    use windows::core::PCWSTR;
    let wide_path = wide(path);
    let mut owner = PSID::default();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wide_path.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            &mut descriptor,
        )
    };
    if status.0 != 0 {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    let mut sid_buf = vec![0u8; 64];
    let mut len = sid_buf.len() as u32;
    let created = unsafe {
        CreateWellKnownSid(
            WinLocalSystemSid,
            None,
            Some(PSID(sid_buf.as_mut_ptr().cast())),
            &mut len,
        )
    };
    let equal = if created.is_ok() {
        unsafe { EqualSid(owner, PSID(sid_buf.as_mut_ptr().cast())).is_ok() }
    } else {
        false
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    Ok(equal)
}

#[cfg(windows)]
fn set_owner_system(path: &Path) -> io::Result<()> {
    use windows::Win32::Security::Authorization::{SE_FILE_OBJECT, SetNamedSecurityInfoW};
    use windows::Win32::Security::{
        CreateWellKnownSid, OWNER_SECURITY_INFORMATION, PSID, WinLocalSystemSid,
    };
    use windows::core::PCWSTR;
    let mut sid_buf = vec![0u8; 64];
    let mut len = sid_buf.len() as u32;
    unsafe {
        CreateWellKnownSid(
            WinLocalSystemSid,
            None,
            Some(PSID(sid_buf.as_mut_ptr().cast())),
            &mut len,
        )
        .map_err(|error| io::Error::from_raw_os_error(error.code().0))?;
    }
    let wide_path = wide(path);
    let status = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(wide_path.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(PSID(sid_buf.as_mut_ptr().cast())),
            None,
            None,
            None,
        )
    };
    if status.0 != 0 {
        Err(io::Error::from_raw_os_error(status.0 as i32))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn system_only_dacl(path: &Path) -> io::Result<bool> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
    use windows::core::{PCWSTR, PWSTR};
    let wide_path = wide(path);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wide_path.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut descriptor,
        )
    };
    if status.0 != 0 {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    let mut text = PWSTR::null();
    let converted = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            None,
        )
    };
    let rendered = if converted.is_ok() && !text.is_null() {
        unsafe { text.to_string() }.unwrap_or_default()
    } else {
        String::new()
    };
    unsafe {
        if !text.is_null() {
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        }
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    let protected = rendered.contains("(A;;FA;;;SY)") && !rendered.contains("(D;");
    Ok(protected)
}

#[cfg(windows)]
fn security_descriptor() -> io::Result<windows::Win32::Security::PSECURITY_DESCRIPTOR> {
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::PSECURITY_DESCRIPTOR;
    use windows::core::PCWSTR;
    let sddl = wide_str(SYSTEM_ONLY_SDDL);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1,
            &mut descriptor,
            None,
        )
        .map_err(|error| io::Error::from_raw_os_error(error.code().0))?;
    }
    Ok(descriptor)
}

#[cfg(windows)]
fn free_descriptor(descriptor: &windows::Win32::Security::PSECURITY_DESCRIPTOR) {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    if !descriptor.0.is_null() {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
    }
}

#[cfg(windows)]
fn running_as_system() -> bool {
    use windows::Win32::Security::{
        CheckTokenMembership, CreateWellKnownSid, PSID, WinLocalSystemSid,
    };
    unsafe {
        let mut sid_buf = vec![0u8; 64];
        let mut len = sid_buf.len() as u32;
        if CreateWellKnownSid(
            WinLocalSystemSid,
            None,
            Some(PSID(sid_buf.as_mut_ptr().cast())),
            &mut len,
        )
        .is_err()
        {
            return false;
        }
        let mut member = windows::core::BOOL::default();
        CheckTokenMembership(None, PSID(sid_buf.as_mut_ptr().cast()), &mut member).is_ok()
            && member.as_bool()
    }
}

#[cfg(windows)]
fn enable_privilege(name: windows::core::PCWSTR) -> io::Result<()> {
    use windows::Win32::Foundation::{ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE};
    use windows::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|error| io::Error::from_raw_os_error(error.code().0))?;
        let mut luid = windows::Win32::Foundation::LUID::default();
        if let Err(error) = LookupPrivilegeValueW(windows::core::PCWSTR::null(), name, &mut luid) {
            let _ = windows::Win32::Foundation::CloseHandle(token);
            return Err(io::Error::from_raw_os_error(error.code().0));
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
        let _ = windows::Win32::Foundation::CloseHandle(token);
        adjusted.map_err(|error| io::Error::from_raw_os_error(error.code().0))?;
        if last == ERROR_NOT_ALL_ASSIGNED {
            return Err(io::Error::other("privilege missing"));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn wide_str(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
