//! Background service. The lock authority.
//!
//! The display name is [`softlock_core::SERVICE_DISPLAY_NAME`]. Public clients
//! speak the documented pipe. The Lock Host speaks a private channel and
//! cannot unlock itself.

mod authority;
#[cfg(windows)]
mod journal_store;
#[cfg(feature = "live-fail-closed")]
mod live_marker;
mod log;
#[cfg(feature = "simulated-physical")]
mod pincheck;
mod platform;
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
#[cfg(feature = "simulated-physical")]
pub use pincheck::PinChecker;
#[cfg(feature = "simulated-physical")]
pub use pincheck::marker_bytes;
pub use platform::{ArmWait, OsLock, ServicePlatform};
pub use secrets::PinStore;

#[cfg(windows)]
pub use secrets::{SYSTEM_ONLY_SDDL, dacl_sddl, is_reparse_point, user_read_denied};

#[cfg(windows)]
pub use settings::repair_main;

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
