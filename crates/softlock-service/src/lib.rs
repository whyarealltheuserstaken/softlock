//! Background service. The lock authority.
//!
//! The display name is [`softlock_core::SERVICE_DISPLAY_NAME`]. Public clients
//! speak the documented pipe. The Lock Host speaks a private channel and
//! cannot unlock itself.

mod authority;
mod clock;
#[cfg(windows)]
mod config_file;
#[cfg(windows)]
mod journal_store;
#[cfg(feature = "live-fail-closed")]
mod live_marker;
mod log;
#[cfg(windows)]
mod password;
mod password_shape;
#[cfg(feature = "simulated-physical")]
mod pincheck;
mod platform;
#[cfg(any(feature = "simulated-physical", feature = "live-fail-closed"))]
mod present_physical;
mod secrets;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod taskmgr;

#[cfg(windows)]
mod windows;

pub use authority::{
    Authority, HostAction, HostFault, HostLoss, admit_client, dev_pin_allowed, fallback_is_new,
    image_is_inside, log_path, pin_directory, pipe_path, role_for_image, service_data_directory,
    stub_os_lock,
};
#[cfg(feature = "live-fail-closed")]
pub use live_marker::marker_bytes as live_fail_closed_marker_bytes;
pub use log::{Capture, FileLog, allowed_field, audit};
pub use password_shape::{PASSWORD_MAX_BYTES, PASSWORD_MAX_CHARS, password_accepted_shape};
#[cfg(feature = "simulated-physical")]
pub use pincheck::PinChecker;
#[cfg(feature = "simulated-physical")]
pub use pincheck::marker_bytes;
pub use platform::{ArmWait, OsLock, ServicePlatform};
#[cfg(any(feature = "simulated-physical", feature = "live-fail-closed"))]
pub use present_physical::marker_bytes as present_physical_marker_bytes;
pub use secrets::PinStore;

#[cfg(windows)]
pub use config_file::{owner_lock_rights, owner_right_denied, owner_write_denied};
#[cfg(all(windows, feature = "live-fail-closed"))]
pub use config_file::{testing_dacl, testing_owner, testing_restore_dacl};
#[cfg(windows)]
pub use secrets::{SYSTEM_ONLY_SDDL, dacl_sddl, is_reparse_point, user_read_denied};

#[cfg(windows)]
pub use taskmgr::owner_config_file;

#[cfg(windows)]
pub use settings::{
    clear_disable_lock_workstation_for_os_lock, reapply_disable_lock_workstation, repair_main,
};

#[cfg(windows)]
pub use password::{AccountLockout, account_lockout, network_logon};
#[cfg(windows)]
pub use windows::{
    run_foreground, run_service, run_session_probe, server_image_is_agent, server_is_system_agent,
};

#[cfg(not(windows))]
pub fn run_foreground() -> i32 {
    eprintln!("the service runs on Windows");
    1
}

#[cfg(not(windows))]
pub fn run_service() -> i32 {
    eprintln!("the service runs on Windows");
    1
}

#[cfg(not(windows))]
pub fn run_session_probe(_path: &str) -> i32 {
    eprintln!("the service runs on Windows");
    1
}
