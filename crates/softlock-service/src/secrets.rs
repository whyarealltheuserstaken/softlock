//! PIN hash on disk. The running service keeps the hash in memory. The file
//! is the copy that survives a restart. On Windows its DACL is
//! `D:P(A;;FA;;;SY)`: LocalSystem only, protected, with no deny ACEs. A deny
//! for Administrators also denies SYSTEM, so the service could not read the
//! hash back.
//!
//! The directory is the LocalSystem profile
//! (`%SystemRoot%\System32\config\systemprofile\AppData\Local\<service>`).
//! A standard user cannot create anything there. The service creates that
//! directory only when `CreateDirectoryW` itself succeeds, then checks the
//! owner, the DACL, and the reparse bit on the handle it opened. An existing
//! directory is used only when that check already passes. A failure is fatal
//! for the PIN store: nothing is deleted, nothing is adopted, and no hash is
//! installed.

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

    pub fn remove(&self) -> io::Result<()> {
        #[cfg(windows)]
        {
            remove_pin(&self.path)
        }
        #[cfg(not(windows))]
        {
            match fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        }
    }

    #[cfg(windows)]
    fn write_hash(&self, parent: &Path, hash: &str) -> io::Result<()> {
        if !running_as_system() {
            return write_hash_without_system(parent, &self.path, hash);
        }
        verify_service_directory(parent)?;
        let dir = open_verified_directory(parent)?;
        let outcome = write_hash_in_directory(dir, hash);
        drop_handle(dir);
        outcome
    }

    #[cfg(not(windows))]
    fn write_hash(&self, parent: &Path, hash: &str) -> io::Result<()> {
        ensure_pin_directory(parent)?;
        let mut last_exists = None;
        for _ in 0..PIN_TMP_ATTEMPTS {
            let tmp = parent.join(pin_tmp_name()?);
            let mut file = match create_pin_temp(&tmp) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    last_exists = Some(error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            let outcome = (|| {
                use std::io::Write;
                file.write_all(hash.as_bytes())?;
                file.sync_all()?;
                drop(file);
                fs::rename(&tmp, &self.path)?;
                fsync_directory(parent)
            })();
            if let Err(error) = outcome {
                let _ = fs::remove_file(&tmp);
                return Err(error);
            }
            return Ok(());
        }
        Err(last_exists.unwrap_or_else(|| io::Error::other("pin temp name collided")))
    }

    pub fn load(&self) -> io::Result<Option<String>> {
        #[cfg(windows)]
        {
            load_verified(&self.path)
        }
        #[cfg(not(windows))]
        {
            match fs::read_to_string(&self.path) {
                Ok(text) => Ok(Some(text)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }
    }
}

#[cfg(not(windows))]
const PIN_TMP_ATTEMPTS: usize = 8;

#[cfg(not(windows))]
fn ensure_pin_directory(path: &Path) -> io::Result<()> {
    match pin_directory_ok(path) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut built = PathBuf::new();
    let parts: Vec<_> = path.components().collect();
    for (index, component) in parts.iter().enumerate() {
        built.push(component);
        if index + 1 != parts.len() && fs::symlink_metadata(&built).is_ok() {
            continue;
        }
        let mut builder = fs::DirBuilder::new();
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
        match builder.create(&built) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    pin_directory_ok(path)
}

/// `lstat` the pin directory. A symlink is not a directory. Group and other
/// write, or an owner other than this process, is refused. Nothing is deleted.
#[cfg(not(windows))]
fn pin_directory_ok(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_dir() {
        return Err(io::Error::other("pin directory is not a real directory"));
    }
    if meta.uid() != current_euid() {
        return Err(io::Error::other(
            "pin directory owner is not the service user",
        ));
    }
    if meta.permissions().mode() & 0o022 != 0 {
        return Err(io::Error::other(
            "pin directory is writable by group or other",
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn current_euid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(not(windows))]
fn create_pin_temp(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(windows))]
fn fsync_directory(path: &Path) -> io::Result<()> {
    let directory = fs::File::open(path)?;
    directory.sync_all()
}

#[cfg(not(windows))]
fn pin_tmp_name() -> io::Result<String> {
    #[cfg(test)]
    if let Some(name) = forced_pin_tmp_name() {
        return Ok(name);
    }
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    let mut name = String::from(".pin.");
    for byte in bytes {
        name.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        name.push(char::from(b"0123456789abcdef"[(byte & 0xf) as usize]));
    }
    name.push_str(".tmp");
    Ok(name)
}

#[cfg(all(test, not(windows)))]
fn forced_pin_tmp_name() -> Option<String> {
    PIN_TMP_NAMES.with(|names| names.borrow_mut().pop())
}

#[cfg(all(test, not(windows)))]
thread_local! {
    static PIN_TMP_NAMES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
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

/// Open `dir` without following a reparse point. Use it when the handle
/// already proves LocalSystem owns it and the DACL is exactly
/// [`SYSTEM_ONLY_SDDL`]. Create it only when it is absent and
/// `CreateDirectoryW` is the call that creates it. Never delete, and never
/// take ownership of a directory that was already there.
#[cfg(windows)]
pub fn verify_service_directory(dir: &Path) -> io::Result<()> {
    if !running_as_system() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "service directory requires LocalSystem",
        ));
    }
    ensure_profile_parent(dir)?;
    match open_directory(dir, false) {
        Ok(handle) => {
            let verified = verify_handle(handle);
            drop_handle(handle);
            verified
        }
        Err(error) if not_found(&error) => create_service_directory(dir),
        Err(error) => Err(error),
    }
}

/// The Windows event log, used when the service directory cannot be verified
/// and the file log would have to be created under an untrusted path.
#[cfg(windows)]
pub(crate) fn report_event(message: &str) {
    use windows::Win32::System::EventLog::{
        DeregisterEventSource, EVENTLOG_ERROR_TYPE, RegisterEventSourceW, ReportEventW,
    };
    use windows::core::PCWSTR;
    let name = wide_str(softlock_core::SERVICE_NAME);
    let Ok(source) = (unsafe { RegisterEventSourceW(PCWSTR::null(), PCWSTR(name.as_ptr())) })
    else {
        return;
    };
    let text = wide_str(message);
    let strings = [PCWSTR(text.as_ptr())];
    unsafe {
        let _ = ReportEventW(
            source,
            EVENTLOG_ERROR_TYPE,
            0,
            1,
            None,
            0,
            Some(&strings),
            None,
        );
        let _ = DeregisterEventSource(source);
    }
}

/// Append the service log under a directory that already verified. A planted
/// file or reparse point is left in place and this call fails.
#[cfg(windows)]
pub(crate) fn open_secured_log(path: &Path) -> io::Result<fs::File> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::other("log path has no directory"));
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(io::Error::other("log path has no file name"));
    };
    verify_service_directory(parent)?;
    let dir = open_verified_directory(parent)?;
    let opened = open_log_in_directory(dir, name);
    drop_handle(dir);
    opened
}

#[cfg(windows)]
fn create_service_directory(dir: &Path) -> io::Result<()> {
    match create_directory(dir) {
        Ok(()) => {}
        Err(error) if already_exists_io(&error) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "refusing to adopt an existing service directory",
            ));
        }
        Err(error) => return Err(error),
    }
    let handle = open_directory(dir, true)?;
    let verified = finish_created_handle(handle);
    drop_handle(handle);
    verified
}

#[cfg(windows)]
fn ensure_profile_parent(leaf: &Path) -> io::Result<()> {
    let Some(local) = leaf.parent() else {
        return Err(io::Error::other("service directory has no parent"));
    };
    let Some(appdata) = local.parent() else {
        return Err(io::Error::other("LocalSystem profile is missing"));
    };
    let Some(profile) = appdata.parent() else {
        return Err(io::Error::other("LocalSystem profile is missing"));
    };
    // `systemprofile` has to already exist. AppData and Local may be created
    // beneath it. Nothing above the profile is created.
    ensure_directory_present(profile)
        .map_err(|_| io::Error::other("LocalSystem profile is missing"))?;
    ensure_created(appdata)?;
    ensure_created(local)
}

#[cfg(windows)]
fn ensure_created(path: &Path) -> io::Result<()> {
    if !directory_openable(path) {
        create_directory_default(path).or_else(|error| {
            if already_exists_io(&error) {
                Ok(())
            } else {
                Err(error)
            }
        })?;
    }
    ensure_directory_present(path)
}

#[cfg(windows)]
fn directory_openable(path: &Path) -> bool {
    match open_directory(path, false) {
        Ok(handle) => {
            drop_handle(handle);
            true
        }
        Err(_) => false,
    }
}

#[cfg(windows)]
fn ensure_directory_present(path: &Path) -> io::Result<()> {
    let handle = open_directory(path, false)?;
    let reparse = handle_is_reparse(handle)?;
    drop_handle(handle);
    if reparse {
        Err(io::Error::other("reparse point in the LocalSystem profile"))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn load_verified(path: &Path) -> io::Result<Option<String>> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::other("pin path has no directory"));
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(io::Error::other("pin path has no file name"));
    };
    let dir = match open_directory(parent, false) {
        Ok(dir) => dir,
        Err(error) if not_found(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    if handle_is_reparse(dir)? {
        drop_handle(dir);
        return Err(io::Error::other("refusing a reparse point"));
    }
    let opened = open_relative(dir, name);
    drop_handle(dir);
    let handle = match opened {
        Ok(handle) => handle,
        Err(error) if not_found(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    if let Err(error) = verify_handle(handle) {
        drop_handle(handle);
        return Err(error);
    }
    read_handle_text(handle).map(Some)
}

#[cfg(windows)]
fn write_hash_in_directory(dir: windows::Win32::Foundation::HANDLE, hash: &str) -> io::Result<()> {
    let name = random_temp_name()?;
    let file = create_relative(&name, dir, SYSTEM_ONLY_SDDL)?;
    if let Err(error) = finish_created_handle(file) {
        let _ = delete_by_handle(file);
        drop_handle(file);
        return Err(error);
    }
    if let Err(error) = write_all_handle(file, hash.as_bytes()) {
        let _ = delete_by_handle(file);
        drop_handle(file);
        return Err(error);
    }
    if let Err(error) = displace_untrusted(dir, "pin.phc") {
        let _ = delete_by_handle(file);
        drop_handle(file);
        return Err(error);
    }
    if let Err(error) = rename_relative(file, dir, "pin.phc", true) {
        let _ = delete_by_handle(file);
        drop_handle(file);
        return Err(error);
    }
    drop_handle(file);
    let final_handle = open_relative(dir, "pin.phc")?;
    let verified = verify_handle(final_handle);
    drop_handle(final_handle);
    verified
}

#[cfg(windows)]
fn open_log_in_directory(
    dir: windows::Win32::Foundation::HANDLE,
    name: &str,
) -> io::Result<fs::File> {
    match open_relative(dir, name) {
        Ok(handle) => {
            if let Err(error) = verify_handle(handle) {
                drop_handle(handle);
                return Err(error);
            }
            file_at_end(handle)
        }
        Err(error) if not_found(&error) => {
            let handle = create_relative(name, dir, SYSTEM_ONLY_SDDL)?;
            if let Err(error) = finish_created_handle(handle) {
                let _ = delete_by_handle(handle);
                drop_handle(handle);
                return Err(error);
            }
            file_at_end(handle)
        }
        Err(error) => Err(error),
    }
}

/// A name that already exists is moved aside with one rename against the
/// open directory handle. The rename fails closed when the entry is held
/// open without delete sharing. Children are never walked.
#[cfg(windows)]
fn displace_untrusted(dir: windows::Win32::Foundation::HANDLE, name: &str) -> io::Result<()> {
    let existing = match open_relative(dir, name) {
        Ok(handle) => handle,
        Err(error) if not_found(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    let trusted = !handle_is_reparse(existing)?
        && !handle_is_directory(existing)?
        && verify_handle(existing).is_ok();
    if trusted {
        drop_handle(existing);
        return Ok(());
    }
    let aside = random_temp_name()?;
    let renamed = rename_relative(existing, dir, &aside, false);
    drop_handle(existing);
    renamed
}

#[cfg(windows)]
fn finish_created_handle(handle: windows::Win32::Foundation::HANDLE) -> io::Result<()> {
    if verify_handle(handle).is_ok() {
        return Ok(());
    }
    // The object was created by this call. Setting the owner and DACL is
    // finishing that create, scoped to the call, not adopting a planted one.
    let restore = enable_privilege_scoped(windows::Win32::Security::SE_RESTORE_NAME);
    let take = enable_privilege_scoped(windows::Win32::Security::SE_TAKE_OWNERSHIP_NAME);
    let applied = apply_security_on_handle(handle);
    drop(restore);
    drop(take);
    applied?;
    verify_handle(handle)
}

#[cfg(all(windows, feature = "simulated-physical"))]
fn write_handle(handle: windows::Win32::Foundation::HANDLE, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::windows::io::FromRawHandle;
    let mut file = unsafe { fs::File::from_raw_handle(handle.0) };
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn write_all_handle(
    handle: windows::Win32::Foundation::HANDLE,
    bytes: &[u8],
) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::{FlushFileBuffers, WriteFile};
    let mut offset = 0;
    while offset < bytes.len() {
        let mut wrote = 0u32;
        unsafe {
            WriteFile(handle, Some(&bytes[offset..]), Some(&mut wrote), None).map_err(os_error)?;
        }
        if wrote == 0 {
            return Err(io::Error::other("short write"));
        }
        offset += wrote as usize;
    }
    unsafe {
        FlushFileBuffers(handle).map_err(os_error)?;
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn read_handle_text(handle: windows::Win32::Foundation::HANDLE) -> io::Result<String> {
    use std::io::Read;
    use std::os::windows::io::FromRawHandle;
    let mut file = unsafe { fs::File::from_raw_handle(handle.0) };
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text)
}

#[cfg(windows)]
fn file_at_end(handle: windows::Win32::Foundation::HANDLE) -> io::Result<fs::File> {
    use std::io::Seek;
    use std::os::windows::io::FromRawHandle;
    let mut file = unsafe { fs::File::from_raw_handle(handle.0) };
    file.seek(std::io::SeekFrom::End(0))?;
    Ok(file)
}

#[cfg(all(windows, feature = "simulated-physical"))]
fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;
    let source = wide(from);
    let dest = wide(to);
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(dest.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(os_error)?;
    }
    let _ = fs::remove_file(from);
    Ok(())
}

#[cfg(windows)]
fn create_directory(path: &Path) -> io::Result<()> {
    create_directory_with(path, Some(SYSTEM_ONLY_SDDL))
}

#[cfg(windows)]
fn create_directory_default(path: &Path) -> io::Result<()> {
    create_directory_with(path, None)
}

#[cfg(windows)]
pub(crate) fn create_directory_with(path: &Path, sddl: Option<&str>) -> io::Result<()> {
    use windows::Win32::Security::SECURITY_ATTRIBUTES;
    use windows::Win32::Storage::FileSystem::CreateDirectoryW;
    use windows::core::PCWSTR;
    let descriptor = sddl.map(security_descriptor_from).transpose()?;
    let sa = descriptor.as_ref().map(|descriptor| SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    });
    let wide_path = wide(path);
    let created = unsafe {
        if let Some(sa) = sa.as_ref() {
            CreateDirectoryW(
                PCWSTR(wide_path.as_ptr()),
                Some(sa as *const SECURITY_ATTRIBUTES),
            )
        } else {
            CreateDirectoryW(PCWSTR(wide_path.as_ptr()), None)
        }
    };
    if let Some(descriptor) = descriptor.as_ref() {
        free_descriptor(descriptor);
    }
    created.map_err(os_error)
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
        Err(error) => Err(os_error(error)),
    }
}

#[cfg(all(windows, feature = "simulated-physical"))]
fn already_exists(error: &windows::core::Error) -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};
    let code = error.code().0 as u32;
    code == ERROR_FILE_EXISTS.0
        || code == ERROR_ALREADY_EXISTS.0
        || code & 0xFFFF == ERROR_FILE_EXISTS.0
        || code & 0xFFFF == ERROR_ALREADY_EXISTS.0
}

#[cfg(windows)]
pub(crate) fn already_exists_io(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::AlreadyExists
        || matches!(error.raw_os_error(), Some(80) | Some(183))
}

#[cfg(windows)]
pub(crate) fn not_found(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || matches!(error.raw_os_error(), Some(2) | Some(3))
}

/// `windows::core::Error::code` is an HRESULT. `ERROR_FILE_NOT_FOUND` arrives
/// as `0x80070002`, which `from_raw_os_error` does not treat as not-found,
/// so a missing directory was never created.
#[cfg(windows)]
pub(crate) fn os_error(error: windows::core::Error) -> io::Error {
    let code = error.code().0 as u32;
    let win32 = if code & 0xFFFF_0000 == 0x8007_0000 {
        (code & 0xFFFF) as i32
    } else {
        error.code().0
    };
    io::Error::from_raw_os_error(win32)
}

#[cfg(windows)]
fn security_descriptor() -> io::Result<windows::Win32::Security::PSECURITY_DESCRIPTOR> {
    security_descriptor_from(SYSTEM_SECURITY_SDDL)
}

#[cfg(windows)]
pub(crate) fn security_descriptor_from(
    sddl: &str,
) -> io::Result<windows::Win32::Security::PSECURITY_DESCRIPTOR> {
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::PSECURITY_DESCRIPTOR;
    use windows::core::PCWSTR;
    let sddl = wide_str(sddl);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1,
            &mut descriptor,
            None,
        )
        .map_err(os_error)?;
    }
    Ok(descriptor)
}

#[cfg(windows)]
pub(crate) fn free_descriptor(descriptor: &windows::Win32::Security::PSECURITY_DESCRIPTOR) {
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
pub(crate) struct PrivilegeGuard {
    token: windows::Win32::Foundation::HANDLE,
    previous: windows::Win32::Security::TOKEN_PRIVILEGES,
}

#[cfg(windows)]
impl Drop for PrivilegeGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Security::AdjustTokenPrivileges(
                self.token,
                false,
                Some(&self.previous),
                0,
                None,
                None,
            );
            let _ = windows::Win32::Foundation::CloseHandle(self.token);
        }
    }
}

/// Enable one privilege for the current thread token and put the previous
/// state back when the guard drops. Callers hold the guard across the single
/// call that needs it.
#[cfg(windows)]
pub(crate) fn enable_privilege_scoped(name: windows::core::PCWSTR) -> io::Result<PrivilegeGuard> {
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
        .map_err(os_error)?;
        let mut luid = windows::Win32::Foundation::LUID::default();
        if let Err(error) = LookupPrivilegeValueW(windows::core::PCWSTR::null(), name, &mut luid) {
            let _ = windows::Win32::Foundation::CloseHandle(token);
            return Err(os_error(error));
        }
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let mut previous = TOKEN_PRIVILEGES::default();
        let mut returned = 0u32;
        let adjusted = AdjustTokenPrivileges(
            token,
            false,
            Some(&privileges as *const TOKEN_PRIVILEGES),
            std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
            Some(&mut previous as *mut TOKEN_PRIVILEGES),
            Some(&mut returned as *mut u32),
        );
        let last = GetLastError();
        if let Err(error) = adjusted {
            let _ = windows::Win32::Foundation::CloseHandle(token);
            return Err(os_error(error));
        }
        if last == ERROR_NOT_ALL_ASSIGNED {
            let _ = windows::Win32::Foundation::CloseHandle(token);
            return Err(io::Error::other("privilege missing"));
        }
        Ok(PrivilegeGuard { token, previous })
    }
}

#[cfg(windows)]
pub fn is_reparse_point(path: &Path) -> io::Result<bool> {
    use windows::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, READ_CONTROL};
    let _backup = enable_privilege_scoped(windows::Win32::Security::SE_BACKUP_NAME).ok();
    let handle = open_reparse(path, FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0, true)?;
    let reparse = handle_is_reparse(handle);
    drop_handle(handle);
    reparse
}

#[cfg(windows)]
pub(crate) fn handle_is_reparse(handle: windows::Win32::Foundation::HANDLE) -> io::Result<bool> {
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
        .map_err(os_error)?;
    }
    Ok(info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
}

#[cfg(windows)]
fn handle_is_directory(handle: windows::Win32::Foundation::HANDLE) -> io::Result<bool> {
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_TAG_INFO, FileAttributeTagInfo,
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
        .map_err(os_error)?;
    }
    Ok(info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0)
}

#[cfg(windows)]
fn open_verified_directory(path: &Path) -> io::Result<windows::Win32::Foundation::HANDLE> {
    let dir = open_directory(path, true)?;
    if let Err(error) = verify_handle(dir) {
        drop_handle(dir);
        return Err(error);
    }
    Ok(dir)
}

#[cfg(windows)]
fn remove_pin(path: &Path) -> io::Result<()> {
    if !running_as_system() {
        return remove_pin_without_system(path);
    }
    let Some(parent) = path.parent() else {
        return Err(io::Error::other("pin path has no directory"));
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(io::Error::other("pin path has no file name"));
    };
    verify_service_directory(parent)?;
    let dir = open_verified_directory(parent)?;
    let file = match open_relative(dir, name) {
        Ok(file) => file,
        Err(error) if not_found(&error) => {
            drop_handle(dir);
            return Ok(());
        }
        Err(error) => {
            drop_handle(dir);
            return Err(error);
        }
    };
    if let Err(error) = verify_handle(file) {
        drop_handle(file);
        drop_handle(dir);
        return Err(error);
    }
    let deleted = delete_by_handle(file);
    drop_handle(file);
    drop_handle(dir);
    deleted
}

#[cfg(windows)]
fn remove_pin_without_system(path: &Path) -> io::Result<()> {
    #[cfg(not(feature = "simulated-physical"))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pin store requires LocalSystem",
        ))
    }
    #[cfg(feature = "simulated-physical")]
    {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if not_found(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(windows)]
pub(crate) fn open_directory(
    path: &Path,
    control: bool,
) -> io::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Storage::FileSystem::{
        FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
        FILE_TRAVERSE, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
    };
    let mut access = FILE_LIST_DIRECTORY.0
        | FILE_TRAVERSE.0
        | FILE_READ_ATTRIBUTES.0
        | READ_CONTROL.0
        | 0x0010_0000;
    if control {
        access |= FILE_ADD_FILE.0 | FILE_ADD_SUBDIRECTORY.0 | WRITE_DAC.0 | WRITE_OWNER.0;
    }
    open_reparse(path, access, true)
}

#[cfg(all(windows, feature = "simulated-physical"))]
fn open_for_control(path: &Path) -> io::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Storage::FileSystem::{
        FILE_GENERIC_READ, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
    };
    open_reparse(
        path,
        FILE_GENERIC_READ.0 | READ_CONTROL.0 | WRITE_DAC.0 | WRITE_OWNER.0,
        false,
    )
}

#[cfg(windows)]
fn open_reparse(
    path: &Path,
    access: u32,
    directory: bool,
) -> io::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::core::PCWSTR;
    let flags = if directory {
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    } else {
        FILE_FLAG_OPEN_REPARSE_POINT
    };
    let wide_path = wide(path);
    unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
        .map_err(os_error)
    }
}

#[cfg(windows)]
pub(crate) fn drop_handle(handle: windows::Win32::Foundation::HANDLE) {
    if !handle.0.is_null() {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
    }
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
    applied.map_err(os_error)
}

#[cfg(windows)]
pub fn dacl_sddl(path: &Path) -> io::Result<String> {
    use windows::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, READ_CONTROL};
    // Reading a SYSTEM-only DACL back from the test process needs backup
    // semantics. The privilege is enabled for this read and restored after.
    let _backup = enable_privilege_scoped(windows::Win32::Security::SE_BACKUP_NAME)?;
    let handle = open_reparse(path, FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0, true)?;
    let text = dacl_sddl_handle(handle);
    drop_handle(handle);
    text
}

#[cfg(windows)]
pub(crate) fn dacl_sddl_handle(handle: windows::Win32::Foundation::HANDLE) -> io::Result<String> {
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
        .map_err(os_error)?;
    }
    Ok(sid_buf)
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
pub(crate) fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
pub(crate) fn wide_str(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
const FILE_CREATE: u32 = 2;
#[cfg(windows)]
const FILE_OPEN: u32 = 1;
#[cfg(windows)]
const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
#[cfg(windows)]
const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
#[cfg(windows)]
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
#[cfg(windows)]
const FILE_OPEN_REPARSE_POINT_OPTION: u32 = 0x0020_0000;
#[cfg(windows)]
const OBJ_CASE_INSENSITIVE: u32 = 0x0000_0040;
#[cfg(windows)]
const FILE_READ_ACCESS: u32 = 0x8000_0000;
#[cfg(windows)]
const FILE_WRITE_ACCESS: u32 = 0x4000_0000;

#[cfg(windows)]
#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[cfg(windows)]
#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: *mut core::ffi::c_void,
    object_name: *mut UnicodeString,
    attributes: u32,
    security_descriptor: *mut core::ffi::c_void,
    security_quality_of_service: *mut core::ffi::c_void,
}

#[cfg(all(windows, target_pointer_width = "64"))]
const _: () = assert!(std::mem::size_of::<ObjectAttributes>() == 48);

#[cfg(windows)]
#[repr(C)]
struct IoStatusBlock {
    status: isize,
    information: usize,
}

#[cfg(windows)]
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtCreateFile(
        file_handle: *mut *mut core::ffi::c_void,
        desired_access: u32,
        object_attributes: *mut ObjectAttributes,
        io_status_block: *mut IoStatusBlock,
        allocation_size: *mut i64,
        file_attributes: u32,
        share_access: u32,
        create_disposition: u32,
        create_options: u32,
        ea_buffer: *mut core::ffi::c_void,
        ea_length: u32,
    ) -> i32;

    fn NtSetInformationFile(
        file_handle: *mut core::ffi::c_void,
        io_status_block: *mut IoStatusBlock,
        file_information: *mut core::ffi::c_void,
        length: u32,
        file_information_class: u32,
    ) -> i32;

    fn RtlNtStatusToDosError(status: i32) -> u32;

    fn NtQueryDirectoryFile(
        file_handle: *mut core::ffi::c_void,
        event: *mut core::ffi::c_void,
        apc_routine: *mut core::ffi::c_void,
        apc_context: *mut core::ffi::c_void,
        io_status_block: *mut IoStatusBlock,
        file_information: *mut core::ffi::c_void,
        length: u32,
        file_information_class: u32,
        return_single_entry: u8,
        file_name: *mut UnicodeString,
        restart_scan: u8,
    ) -> i32;
}

#[cfg(windows)]
pub(crate) fn nt_result(status: i32) -> io::Result<()> {
    if status >= 0 {
        return Ok(());
    }
    let code = unsafe { RtlNtStatusToDosError(status) };
    Err(io::Error::from_raw_os_error(code as i32))
}

#[cfg(windows)]
pub(crate) fn create_relative(
    name: &str,
    directory: windows::Win32::Foundation::HANDLE,
    sddl: &str,
) -> io::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    let descriptor = security_descriptor_from(sddl)?;
    let mut wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut unicode = UnicodeString {
        length: ((wide_name.len() - 1) * 2) as u16,
        maximum_length: (wide_name.len() * 2) as u16,
        buffer: wide_name.as_mut_ptr(),
    };
    let mut attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: directory.0,
        object_name: &mut unicode,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: descriptor.0,
        security_quality_of_service: std::ptr::null_mut(),
    };
    let mut iosb = IoStatusBlock {
        status: 0,
        information: 0,
    };
    let mut raw = std::ptr::null_mut();
    let access = FILE_READ_ACCESS
        | FILE_WRITE_ACCESS
        | 0x0002_0000
        | 0x0010_0000
        | 0x0001_0000
        | 0x0004_0000
        | 0x0008_0000;
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            access,
            &mut attributes,
            &mut iosb,
            std::ptr::null_mut(),
            FILE_ATTRIBUTE_NORMAL.0,
            0,
            FILE_CREATE,
            FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT_OPTION,
            std::ptr::null_mut(),
            0,
        )
    };
    free_descriptor(&descriptor);
    nt_result(status).map_err(|error| {
        if already_exists_io(&error) {
            io::Error::new(io::ErrorKind::AlreadyExists, "file exists")
        } else {
            error
        }
    })?;
    Ok(HANDLE(raw))
}

#[cfg(windows)]
pub(crate) fn create_directory_relative(
    name: &str,
    directory: windows::Win32::Foundation::HANDLE,
    sddl: &str,
) -> io::Result<()> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    let descriptor = security_descriptor_from(sddl)?;
    let mut wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut unicode = UnicodeString {
        length: ((wide_name.len() - 1) * 2) as u16,
        maximum_length: (wide_name.len() * 2) as u16,
        buffer: wide_name.as_mut_ptr(),
    };
    let mut attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: directory.0,
        object_name: &mut unicode,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: descriptor.0,
        security_quality_of_service: std::ptr::null_mut(),
    };
    let mut iosb = IoStatusBlock {
        status: 0,
        information: 0,
    };
    let mut raw = std::ptr::null_mut();
    let access = FILE_READ_ACCESS | FILE_WRITE_ACCESS | 0x0002_0000 | 0x0010_0000 | 0x0001_0000;
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            access,
            &mut attributes,
            &mut iosb,
            std::ptr::null_mut(),
            FILE_ATTRIBUTE_NORMAL.0,
            0,
            FILE_CREATE,
            FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT_OPTION,
            std::ptr::null_mut(),
            0,
        )
    };
    free_descriptor(&descriptor);
    nt_result(status).map_err(|error| {
        if already_exists_io(&error) {
            io::Error::new(io::ErrorKind::AlreadyExists, "directory exists")
        } else {
            error
        }
    })?;
    drop_handle(HANDLE(raw));
    Ok(())
}

#[cfg(windows)]
pub(crate) fn open_relative(
    directory: windows::Win32::Foundation::HANDLE,
    name: &str,
) -> io::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Foundation::HANDLE;
    let mut wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut unicode = UnicodeString {
        length: ((wide_name.len() - 1) * 2) as u16,
        maximum_length: (wide_name.len() * 2) as u16,
        buffer: wide_name.as_mut_ptr(),
    };
    let mut attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: directory.0,
        object_name: &mut unicode,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: std::ptr::null_mut(),
        security_quality_of_service: std::ptr::null_mut(),
    };
    let mut iosb = IoStatusBlock {
        status: 0,
        information: 0,
    };
    let mut raw = std::ptr::null_mut();
    let access = FILE_READ_ACCESS | FILE_WRITE_ACCESS | 0x0002_0000 | 0x0010_0000 | 0x0001_0000;
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            access,
            &mut attributes,
            &mut iosb,
            std::ptr::null_mut(),
            0,
            0x0000_0001 | 0x0000_0002 | 0x0000_0004,
            FILE_OPEN,
            FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT_OPTION,
            std::ptr::null_mut(),
            0,
        )
    };
    nt_result(status)?;
    Ok(HANDLE(raw))
}

#[cfg(windows)]
pub(crate) fn rename_relative(
    file: windows::Win32::Foundation::HANDLE,
    directory: windows::Win32::Foundation::HANDLE,
    name: &str,
    replace: bool,
) -> io::Result<()> {
    let wide_name: Vec<u16> = name.encode_utf16().collect();
    let name_bytes = wide_name.len() * 2;
    let mut buffer = vec![0u8; 20 + name_bytes];
    buffer[0] = u8::from(replace);
    let root = directory.0 as usize;
    buffer[8..16].copy_from_slice(&root.to_ne_bytes());
    buffer[16..20].copy_from_slice(&(name_bytes as u32).to_ne_bytes());
    let mut offset = 20;
    for unit in wide_name {
        buffer[offset..offset + 2].copy_from_slice(&unit.to_ne_bytes());
        offset += 2;
    }
    let mut iosb = IoStatusBlock {
        status: 0,
        information: 0,
    };
    let status = unsafe {
        NtSetInformationFile(
            file.0,
            &mut iosb,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
            10,
        )
    };
    nt_result(status)
}

#[cfg(windows)]
pub(crate) fn delete_by_handle(file: windows::Win32::Foundation::HANDLE) -> io::Result<()> {
    let mut info = [1u8];
    let mut iosb = IoStatusBlock {
        status: 0,
        information: 0,
    };
    let status = unsafe {
        NtSetInformationFile(
            file.0,
            &mut iosb,
            info.as_mut_ptr().cast(),
            info.len() as u32,
            13,
        )
    };
    nt_result(status)
}

#[cfg(windows)]
pub(crate) fn owner_is_trusted(handle: windows::Win32::Foundation::HANDLE) -> io::Result<bool> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows::Win32::Security::{
        EqualSid, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    };
    let mut owner = PSID::default();
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
    let system = well_known_sid(windows::Win32::Security::WinLocalSystemSid)?;
    let admins = well_known_sid(windows::Win32::Security::WinBuiltinAdministratorsSid)?;
    let trusted = unsafe {
        EqualSid(owner, PSID(system.as_ptr() as *mut _)).is_ok()
            || EqualSid(owner, PSID(admins.as_ptr() as *mut _)).is_ok()
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    Ok(trusted)
}

#[cfg(windows)]
fn well_known_sid(kind: windows::Win32::Security::WELL_KNOWN_SID_TYPE) -> io::Result<Vec<u8>> {
    use windows::Win32::Security::{CreateWellKnownSid, PSID};
    let mut sid_buf = vec![0u8; 68];
    let mut len = sid_buf.len() as u32;
    unsafe {
        CreateWellKnownSid(
            kind,
            None,
            Some(PSID(sid_buf.as_mut_ptr().cast())),
            &mut len,
        )
        .map_err(os_error)?;
    }
    Ok(sid_buf)
}

#[cfg(windows)]
pub(crate) fn link_count(handle: windows::Win32::Foundation::HANDLE) -> io::Result<u32> {
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle, &mut info) }.map_err(os_error)?;
    Ok(info.nNumberOfLinks)
}

/// Names in a directory opened with `FILE_FLAG_OPEN_REPARSE_POINT`. `.` and `..` are omitted.
#[cfg(windows)]
pub(crate) fn list_directory_names(
    directory: windows::Win32::Foundation::HANDLE,
) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    let mut restart = 1u8;
    loop {
        let mut buffer = vec![0u8; 16 * 1024];
        let mut iosb = IoStatusBlock {
            status: 0,
            information: 0,
        };
        let status = unsafe {
            NtQueryDirectoryFile(
                directory.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut iosb,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                12,
                0,
                std::ptr::null_mut(),
                restart,
            )
        };
        restart = 0;
        if status == STATUS_NO_MORE_FILES || status == STATUS_NO_SUCH_FILE {
            break;
        }
        nt_result(status)?;
        let mut offset = 0usize;
        loop {
            if offset + 12 > buffer.len() {
                break;
            }
            let next = u32::from_le_bytes([
                buffer[offset],
                buffer[offset + 1],
                buffer[offset + 2],
                buffer[offset + 3],
            ]);
            let name_len = u32::from_le_bytes([
                buffer[offset + 8],
                buffer[offset + 9],
                buffer[offset + 10],
                buffer[offset + 11],
            ]);
            let start = offset + 12;
            let end = start + name_len as usize;
            if end > buffer.len() || !name_len.is_multiple_of(2) {
                return Err(io::Error::other("directory entry was truncated"));
            }
            let units: Vec<u16> = buffer[start..end]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            let name = String::from_utf16(&units).unwrap_or_default();
            if name != "." && name != ".." {
                names.push(name);
            }
            if next == 0 {
                break;
            }
            offset += next as usize;
        }
    }
    Ok(names)
}

#[cfg(windows)]
const STATUS_NO_MORE_FILES: i32 = 0x8000_0006_u32 as i32;
#[cfg(windows)]
const STATUS_NO_SUCH_FILE: i32 = 0xC000_000F_u32 as i32;

#[cfg(all(test, not(windows)))]
mod unix_pin_store {
    use std::fs;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{PIN_TMP_NAMES, PinStore};

    fn scratch() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("softlock-pin-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(&path).expect("scratch");
        path
    }

    fn push_names(names: &[&str]) {
        PIN_TMP_NAMES.with(|slot| {
            slot.borrow_mut().clear();
            for name in names.iter().rev() {
                slot.borrow_mut().push((*name).to_string());
            }
        });
    }

    struct Umask(libc::mode_t);
    impl Umask {
        fn set(mode: libc::mode_t) -> Self {
            Self(unsafe { libc::umask(mode) })
        }
    }
    impl Drop for Umask {
        fn drop(&mut self) {
            unsafe { libc::umask(self.0) };
        }
    }

    fn mode_of(path: &Path) -> u32 {
        fs::symlink_metadata(path)
            .expect("lstat")
            .permissions()
            .mode()
            & 0o777
    }

    #[test]
    fn a_preplanted_symlink_temp_is_not_followed() {
        let dir = scratch();
        let target = dir.join("target");
        fs::write(&target, b"planted").unwrap();
        let link = dir.join("planted.tmp");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        push_names(&["planted.tmp"; 8]);
        let error = PinStore::in_dir(&dir).save("hash-must-not-land");
        assert!(error.is_err(), "a symlinked temp name was opened");
        assert_eq!(fs::read(&target).unwrap(), b"planted");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert!(!dir.join("pin.phc").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hash_file_is_created_0600_under_umask_022() {
        let dir = scratch();
        let _umask = Umask::set(0o022);
        PinStore::in_dir(&dir).save("hash").expect("save");
        assert_eq!(mode_of(&dir.join("pin.phc")), 0o600);
        assert_eq!(fs::read(dir.join("pin.phc")).unwrap(), b"hash");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_existing_temp_name_is_retried() {
        let dir = scratch();
        fs::write(dir.join("taken-a.tmp"), b"a").unwrap();
        fs::write(dir.join("taken-b.tmp"), b"b").unwrap();
        push_names(&["taken-a.tmp", "taken-b.tmp", "fresh.tmp"]);
        PinStore::in_dir(&dir).save("hash").expect("save");
        assert_eq!(fs::read(dir.join("pin.phc")).unwrap(), b"hash");
        assert_eq!(fs::read(dir.join("taken-a.tmp")).unwrap(), b"a");
        assert_eq!(fs::read(dir.join("taken-b.tmp")).unwrap(), b"b");
        assert!(!dir.join("fresh.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_rename_leaves_no_temp_file() {
        let dir = scratch();
        fs::create_dir(dir.join("pin.phc")).unwrap();
        let error = PinStore::in_dir(&dir).save("hash");
        assert!(error.is_err(), "renaming onto a directory succeeded");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".pin.") || name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp left behind: {leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlinked_pin_directory_is_refused() {
        let root = scratch();
        let real = root.join("real");
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(&real).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let error = PinStore::in_dir(&link).save("hash");
        assert!(error.is_err(), "a symlinked pin directory was used");
        assert!(!real.join("pin.phc").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_group_writable_pin_directory_is_refused() {
        let dir = scratch();
        let mut permissions = fs::metadata(&dir).unwrap().permissions();
        permissions.set_mode(0o777);
        fs::set_permissions(&dir, permissions).unwrap();
        assert_eq!(mode_of(&dir), 0o777);
        let error = PinStore::in_dir(&dir).save("hash");
        assert!(error.is_err(), "mode 0777 was accepted");
        assert!(!dir.join("pin.phc").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pin_directory_owned_by_someone_else_is_refused() {
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let error = PinStore::in_dir(Path::new("/")).save("hash");
        assert!(error.is_err(), "a root-owned directory was accepted");
        assert!(!Path::new("/pin.phc").exists());
    }
}
