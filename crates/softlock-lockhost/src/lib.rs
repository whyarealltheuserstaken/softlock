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

// The Windows modules are the real caller. This keeps the dependency in use
// on hosts that do not compile those modules.
#[cfg(feature = "dev-launch")]
const _: fn(&softlock_core::Secret, &str) -> softlock_core::Verdict =
    softlock_core::verify_softlock_pin;
#[cfg(feature = "simulated-physical")]
mod marker;
#[cfg(all(feature = "dev-launch", windows))]
mod win;

pub use classify::{HookInput, InputClass, classify};
#[cfg(feature = "simulated-physical")]
pub use classify::{RunNonce, clear_run_nonce, set_run_nonce};
#[cfg(feature = "dev-launch")]
pub use dev_marker::dev_launch_marker_bytes;
#[cfg(all(feature = "dev-launch", windows))]
pub use win::{
    DevConfig, DevError, DevLock, OVERLAY_MARKER, current_session_id, disconnect_session,
};
