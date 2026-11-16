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
pub const SYSTEM_ONLY_SDDL: &str = "D:P(A;;FA;;;SY)";

/// Owner plus that DACL. The DACL string alone has no owner, and applying it
/// by pointer drops the protected flag.
#[cfg(windows)]
const SYSTEM_SECURITY_SDDL: &str = "O:SYD:P(A;;FA;;;SY)";

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
        if !running_as_system() {
            return write_hash_without_system(parent, &self.path, hash);
        }
        secure_service_directory(parent)
            .map_err(|error| io::Error::other(format!("service directory: {error}")))?;
        // CREATE_NEW plus a random name. A planted file keeps its own DACL
        // under CREATE_ALWAYS, so a user who already holds a read handle
        // would see the hash. An existing name is a failure, not a reuse.
        let tmp = parent.join(random_temp_name()?);
        let handle = create_file_new_system(&tmp)?;
        if let Err(error) = apply_security_on_handle(handle)
            .map_err(|error| io::Error::other(format!("protect temp: {error}")))
            .and_then(|()| verify_handle(handle))
        {
            drop_handle(handle);
            let _ = remove_path_nofollow(&tmp);
            return Err(error);
        }
        if let Err(error) = write_handle(handle, hash.as_bytes()) {
            let _ = remove_path_nofollow(&tmp);
            return Err(error);
        }
        if let Err(error) = replace_file(&tmp, &self.path) {
            let _ = remove_path_nofollow(&tmp);
            return Err(error);
        }
        if is_reparse_point(&self.path)? || !system_dacl_exact(&self.path)? {
            let _ = remove_path_nofollow(&self.path);
            return Err(io::Error::other("pin dacl is not system-only"));
        }
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

/// A user process must not write the hash. The simulated-physical test
/// build is the only exception, and it is not linked into a release binary.
#[cfg(windows)]
fn write_hash_without_system(parent: &Path, dest: &Path, hash: &str) -> io::Result<()> {
    #[cfg(not(feature = "simulated-physical"))]
    {
        let _ = (parent, dest, hash);
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pin store requires LocalSystem",
        ))
    }
    #[cfg(feature = "simulated-physical")]
    {
        fs::create_dir_all(parent)?;
        let tmp = parent.join(random_temp_name()?);
        let handle = create_file_new_user(&tmp)?;
        if let Err(error) = write_handle(handle, hash.as_bytes()) {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
        replace_file(&tmp, dest)?;
        let handle = open_for_control(dest)?;
        let secured = apply_security_on_handle(handle).and_then(|()| verify_handle(handle));
        drop_handle(handle);
        if let Err(error) = secured {
            let _ = fs::remove_file(dest);
            return Err(error);
        }
        Ok(())
    }
}

/// Create the service directory, or delete and recreate it when it is a
/// reparse point, not owned by LocalSystem, or not exactly `D:P(A;;FA;;;SY)`.
/// The DACL and owner are applied through a handle opened with
/// `FILE_FLAG_OPEN_REPARSE_POINT`, so a junction is never followed.
#[cfg(windows)]
pub fn secure_service_directory(dir: &Path) -> io::Result<()> {
    if !running_as_system() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "service directory requires LocalSystem",
        ));
    }
    let _ = enable_privilege(windows::Win32::Security::SE_TAKE_OWNERSHIP_NAME);
    let _ = enable_privilege(windows::Win32::Security::SE_RESTORE_NAME);
    let _ = enable_privilege(windows::Win32::Security::SE_BACKUP_NAME);
    if directory_needs_replace(dir)? {
        remove_tree_nofollow(dir)?;
    }
    if is_reparse_point(dir).is_err() && !dir.exists() {
        create_directory(dir)?;
    }
    let handle = open_for_control(dir)?;
    if handle_is_reparse(handle)? {
        drop_handle(handle);
        return Err(io::Error::other("refusing a reparse point"));
    }
    apply_security_on_handle(handle)?;
    let verified = verify_handle(handle);
    drop_handle(handle);
    verified
}

#[cfg(windows)]
fn directory_needs_replace(dir: &Path) -> io::Result<bool> {
    match is_reparse_point(dir) {
        Ok(true) => Ok(true),
        Ok(false) => {
            let owner_ok = owner_is_system(dir).unwrap_or(false);
            let dacl_ok = system_dacl_exact(dir).unwrap_or(false);
            Ok(!owner_ok || !dacl_ok)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Append the service log under the secured directory. A planted file or
/// reparse point is removed rather than adopted.
#[cfg(windows)]
pub(crate) fn open_secured_log(path: &Path) -> io::Result<fs::File> {
    if is_reparse_point(path).unwrap_or(false)
        || (path.exists()
            && (owner_is_system(path).ok() != Some(true)
                || !system_dacl_exact(path).unwrap_or(false)))
    {
        remove_path_nofollow(path)?;
    }
    if path.exists() {
        return open_append(path);
    }
    let handle = create_file_new_system(path)?;
    if let Err(error) = apply_security_on_handle(handle).and_then(|()| verify_handle(handle)) {
        drop_handle(handle);
        let _ = remove_path_nofollow(path);
        return Err(error);
    }
    Ok(owned_file(handle))
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
    // A junction at the destination would make the rename write the hash
    // through to the target. Remove the reparse point itself.
    if is_reparse_point(to).unwrap_or(false) {
        remove_path_nofollow(to)?;
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
fn create_file_new_system(path: &Path) -> io::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Foundation::ERROR_FILE_EXISTS;
    use windows::Win32::Security::SECURITY_ATTRIBUTES;
    use windows::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_SHARE_MODE, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
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
            FILE_GENERIC_READ.0
                | FILE_GENERIC_WRITE.0
                | READ_CONTROL.0
                | WRITE_DAC.0
                | WRITE_OWNER.0,
            FILE_SHARE_MODE(0),
            Some(&sa),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };
    free_descriptor(&descriptor);
    match handle {
        Ok(handle) => Ok(handle),
        Err(error) if already_exists(&error) || error.code().0 == ERROR_FILE_EXISTS.0 as i32 => {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "pin temp file exists",
            ))
        }
        Err(error) => Err(io::Error::from_raw_os_error(error.code().0)),
    }
}

#[cfg(all(windows, feature = "simulated-physical"))]
fn create_file_new_user(path: &Path) -> io::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_SHARE_MODE,
    };
    use windows::core::PCWSTR;
    let wide_path = wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };
    match handle {
        Ok(handle) => Ok(handle),
        Err(error) if already_exists(&error) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "pin temp file exists",
        )),
        Err(error) => Err(io::Error::from_raw_os_error(error.code().0)),
    }
}

#[cfg(windows)]
fn already_exists(error: &windows::core::Error) -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};
    let code = error.code().0 as u32;
    code == ERROR_FILE_EXISTS.0
        || code == ERROR_ALREADY_EXISTS.0
        || code & 0xFFFF == ERROR_FILE_EXISTS.0
        || code & 0xFFFF == ERROR_ALREADY_EXISTS.0
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
fn security_descriptor() -> io::Result<windows::Win32::Security::PSECURITY_DESCRIPTOR> {
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::PSECURITY_DESCRIPTOR;
    use windows::core::PCWSTR;
    let sddl = wide_str(SYSTEM_SECURITY_SDDL);
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
pub(crate) fn running_as_system() -> bool {
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
pub fn is_reparse_point(path: &Path) -> io::Result<bool> {
    let handle = open_nofollow(path)?;
    let reparse = handle_is_reparse(handle);
    drop_handle(handle);
    reparse
}

#[cfg(windows)]
fn handle_is_reparse(handle: windows::Win32::Foundation::HANDLE) -> io::Result<bool> {
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FileAttributeTagInfo,
        GetFileInformationByHandleEx,
    };
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            &mut info as *mut FILE_ATTRIBUTE_TAG_INFO as *mut core::ffi::c_void,
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
        .map_err(|error| io::Error::from_raw_os_error(error.code().0))?;
    }
    Ok(info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
}

#[cfg(windows)]
fn open_nofollow(path: &Path) -> io::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Storage::FileSystem::{
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL,
    };
    open_reparse(
        path,
        FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )
}

#[cfg(windows)]
fn open_for_control(path: &Path) -> io::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Storage::FileSystem::{
        FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL,
        WRITE_DAC, WRITE_OWNER,
    };
    open_reparse(
        path,
        FILE_GENERIC_READ.0 | READ_CONTROL.0 | WRITE_DAC.0 | WRITE_OWNER.0,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )
}

#[cfg(windows)]
fn open_reparse(
    path: &Path,
    access: u32,
    share: windows::Win32::Storage::FileSystem::FILE_SHARE_MODE,
) -> io::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, OPEN_EXISTING,
    };
    use windows::core::PCWSTR;
    let wide_path = wide(path);
    unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            access,
            share,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
        .map_err(|error| io::Error::from_raw_os_error(error.code().0))
    }
}

#[cfg(windows)]
fn open_append(path: &Path) -> io::Result<fs::File> {
    use std::io::Seek;
    use std::os::windows::io::FromRawHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_APPEND_DATA, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_EXISTING,
    };
    use windows::core::PCWSTR;
    let wide_path = wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_APPEND_DATA.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(|error| io::Error::from_raw_os_error(error.code().0))?
    };
    let mut file = unsafe { fs::File::from_raw_handle(handle.0) };
    file.seek(std::io::SeekFrom::End(0))?;
    Ok(file)
}

#[cfg(windows)]
fn drop_handle(handle: windows::Win32::Foundation::HANDLE) {
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(handle);
    }
}

#[cfg(windows)]
fn owned_file(handle: windows::Win32::Foundation::HANDLE) -> fs::File {
    use std::os::windows::io::FromRawHandle;
    unsafe { fs::File::from_raw_handle(handle.0) }
}

#[cfg(windows)]
fn verify_handle(handle: windows::Win32::Foundation::HANDLE) -> io::Result<()> {
    if handle_is_reparse(handle)? {
        return Err(io::Error::other("refusing a reparse point"));
    }
    let dacl = dacl_sddl_handle(handle)?;
    if canonical_dacl(&dacl) != SYSTEM_ONLY_SDDL {
        return Err(io::Error::other(format!("dacl is not system-only: {dacl}")));
    }
    if !owner_is_system_handle(handle)? {
        return Err(io::Error::other("owner is not LocalSystem"));
    }
    Ok(())
}

#[cfg(windows)]
fn apply_security_on_handle(handle: windows::Win32::Foundation::HANDLE) -> io::Result<()> {
    use windows::Win32::Security::{
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        SetKernelObjectSecurity,
    };
    let descriptor = security_descriptor()?;
    let applied = unsafe {
        SetKernelObjectSecurity(
            handle,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    free_descriptor(&descriptor);
    applied.map_err(|error| io::Error::from_raw_os_error(error.code().0))
}

#[cfg(windows)]
pub fn dacl_sddl(path: &Path) -> io::Result<String> {
    // The directory grants no access except LocalSystem. Reading it back
    // from a test uses backup semantics, which still requires the privilege
    // when the caller is not SYSTEM.
    let _ = enable_privilege(windows::Win32::Security::SE_BACKUP_NAME);
    let _ = enable_privilege(windows::Win32::Security::SE_RESTORE_NAME);
    let _ = enable_privilege(windows::Win32::Security::SE_SECURITY_NAME);
    let handle = open_nofollow(path)?;
    let text = dacl_sddl_handle(handle);
    drop_handle(handle);
    text
}

#[cfg(windows)]
fn dacl_sddl_handle(handle: windows::Win32::Foundation::HANDLE) -> io::Result<String> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
    use windows::core::PWSTR;
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            Some(&mut descriptor),
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
    if rendered.is_empty() {
        Err(io::Error::other("dacl could not be read"))
    } else if canonical_dacl(&rendered) == SYSTEM_ONLY_SDDL {
        Ok(SYSTEM_ONLY_SDDL.to_string())
    } else {
        Ok(rendered)
    }
}

#[cfg(windows)]
fn canonical_dacl(text: &str) -> String {
    text.trim().replace("S-1-5-18", "SY")
}

#[cfg(windows)]
fn system_dacl_exact(path: &Path) -> io::Result<bool> {
    Ok(dacl_sddl(path)? == SYSTEM_ONLY_SDDL)
}

#[cfg(windows)]
fn owner_is_system_handle(handle: windows::Win32::Foundation::HANDLE) -> io::Result<bool> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows::Win32::Security::{EqualSid, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
    let mut owner = windows::Win32::Security::PSID::default();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            Some(&mut descriptor),
        )
    };
    if status.0 != 0 {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    let sid_buf = system_sid()?;
    let equal = unsafe {
        EqualSid(
            owner,
            windows::Win32::Security::PSID(sid_buf.as_ptr() as *mut core::ffi::c_void),
        )
        .is_ok()
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    Ok(equal)
}

#[cfg(windows)]
fn system_sid() -> io::Result<Vec<u8>> {
    use windows::Win32::Security::{CreateWellKnownSid, PSID, WinLocalSystemSid};
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
    Ok(sid_buf)
}

#[cfg(windows)]
fn remove_tree_nofollow(path: &Path) -> io::Result<()> {
    match is_reparse_point(path) {
        Ok(true) => return remove_path_nofollow(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(false) => {}
    }
    if directory_nofollow(path)? {
        for child in children_nofollow(path)? {
            remove_tree_nofollow(&child)?;
        }
    }
    remove_path_nofollow(path)
}

#[cfg(windows)]
fn directory_nofollow(path: &Path) -> io::Result<bool> {
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_TAG_INFO, FileAttributeTagInfo,
        GetFileInformationByHandleEx,
    };
    let handle = open_nofollow(path)?;
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    let queried = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            &mut info as *mut FILE_ATTRIBUTE_TAG_INFO as *mut core::ffi::c_void,
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    drop_handle(handle);
    queried.map_err(|error| io::Error::from_raw_os_error(error.code().0))?;
    Ok(info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0)
}

#[cfg(windows)]
fn children_nofollow(path: &Path) -> io::Result<Vec<std::path::PathBuf>> {
    use windows::Win32::Storage::FileSystem::{
        FindClose, FindFirstFileW, FindNextFileW, WIN32_FIND_DATAW,
    };
    use windows::core::PCWSTR;
    let pattern = wide(&path.join("*"));
    let mut data = WIN32_FIND_DATAW::default();
    let handle = unsafe { FindFirstFileW(PCWSTR(pattern.as_ptr()), &mut data) }
        .map_err(|error| io::Error::from_raw_os_error(error.code().0))?;
    let mut names = Vec::new();
    loop {
        let name = file_name(&data.cFileName);
        if name != "." && name != ".." {
            names.push(path.join(name));
        }
        data = WIN32_FIND_DATAW::default();
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindClose(handle);
    }
    Ok(names)
}

#[cfg(windows)]
fn file_name(raw: &[u16]) -> String {
    let end = raw.iter().position(|unit| *unit == 0).unwrap_or(raw.len());
    String::from_utf16_lossy(&raw[..end])
}

#[cfg(windows)]
pub(crate) fn remove_path_nofollow(path: &Path) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::{DeleteFileW, RemoveDirectoryW};
    use windows::core::PCWSTR;
    let wide_path = wide(path);
    let directory = directory_nofollow(path).unwrap_or(false);
    let removed = unsafe {
        if directory {
            RemoveDirectoryW(PCWSTR(wide_path.as_ptr()))
        } else {
            DeleteFileW(PCWSTR(wide_path.as_ptr()))
        }
    };
    removed.map_err(|error| io::Error::from_raw_os_error(error.code().0))
}

#[cfg(windows)]
fn random_temp_name() -> io::Result<String> {
    use windows::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };
    let mut bytes = [0u8; 16];
    let status = unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status.is_err() {
        return Err(io::Error::other("random name failed"));
    }
    let mut name = String::from("pin.");
    for byte in bytes {
        name.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        name.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
    }
    name.push_str(".tmp");
    Ok(name)
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
