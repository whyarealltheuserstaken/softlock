//! Lock host.
//!
//! The test-only `simulated-physical` feature classifies nonce-tagged injected
//! events. The `dev-launch` feature is the developer entry point: it arms the
//! guard, shows the overlay, and checks a PIN through Core. Release builds
//! leave both features off, and those binaries must not contain their marker
//! strings or symbols.

mod classify;
mod real_lock;

pub use real_lock::{RealLock, decide};

#[cfg(feature = "dev-launch")]
mod dev_marker;

#[cfg(feature = "live-fail-closed")]
mod live_marker;

#[cfg(feature = "simulated-physical")]
mod marker;
#[cfg(windows)]
mod win;

pub use classify::{HookInput, InputClass, classify};
#[cfg(feature = "simulated-physical")]
pub use classify::{RunNonce, clear_run_nonce, set_run_nonce};
#[cfg(feature = "dev-launch")]
pub use dev_marker::dev_launch_marker_bytes;
#[cfg(feature = "live-fail-closed")]
pub use live_marker::marker_bytes as live_fail_closed_marker_bytes;
#[cfg(windows)]
pub use win::{
    DevConfig, DevError, DevLock, OVERLAY_MARKER, current_session_id, disconnect_session,
};

/// Private-channel entry point. The service spawns this process.
#[cfg(windows)]
pub fn run_service_host() -> i32 {
    win::run_service_host()
}
