//! Background service. The lock authority.
//!
//! The display name is [`softlock_core::SERVICE_DISPLAY_NAME`]. Public clients
//! speak the documented pipe. The Lock Host speaks a private channel and
//! cannot unlock itself.

mod authority;
mod log;
mod pincheck;
mod platform;
mod secrets;

#[cfg(windows)]
mod windows;

pub use authority::{
    Authority, HostAction, HostFault, HostLoss, admit_client, dev_pin_allowed, image_is_inside,
    log_path, pin_directory, pipe_path, role_for_image, stub_os_lock,
};
pub use log::{Capture, FileLog, allowed_field, audit};
pub use pincheck::PinChecker;
pub use platform::{ArmWait, OsLock, ServicePlatform};
pub use secrets::PinStore;

#[cfg(windows)]
pub use secrets::user_read_denied;

#[cfg(windows)]
pub use windows::{run_foreground, run_service, run_session_probe};

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
