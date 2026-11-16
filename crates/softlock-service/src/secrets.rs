//! PIN hash on disk. The running service keeps the hash in memory. The file
//! is the copy that survives a restart, and on Windows its DACL denies the
//! interactive user and Administrators.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, hash.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&self.path)?.permissions();
            permissions.set_mode(0o600);
            fs::set_permissions(&self.path, permissions)?;
        }
        #[cfg(windows)]
        protect_from_the_user(&self.path)?;
        Ok(())
    }

    pub fn load(&self) -> io::Result<Option<String>> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// True when a normal read of the hash file is denied.
#[cfg(windows)]
pub fn user_read_denied(path: &Path) -> bool {
    matches!(fs::File::open(path), Err(error) if error.raw_os_error() == Some(5))
}

#[cfg(windows)]
fn protect_from_the_user(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SE_FILE_OBJECT, SetNamedSecurityInfoW,
    };
    use windows::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };
    use windows::core::PCWSTR;

    let sddl: Vec<u16> = "D:P(D;;FA;;;BA)(D;;FA;;;IU)(A;;FA;;;SY)"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1,
            &mut descriptor,
            None,
        )
        .map_err(|error| io::Error::from_raw_os_error(error.code().0))?;
        let mut present = windows::core::BOOL::default();
        let mut dacl = std::ptr::null_mut();
        let mut defaulted = windows::core::BOOL::default();
        if GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted).is_err()
            || !present.as_bool()
        {
            return Err(io::Error::other("security descriptor has no dacl"));
        }
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let status = SetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl as *const ACL),
            None,
        );
        if status.0 != 0 {
            return Err(io::Error::from_raw_os_error(status.0 as i32));
        }
    }
    Ok(())
}
