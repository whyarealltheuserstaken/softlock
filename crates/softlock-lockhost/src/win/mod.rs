//! Windows lock host. The dev-launch path is the in-process session the seam
//! tests drive. The service spawns the same process and talks to it on the
//! private channel.

mod failclosed;
mod guard;
mod overlay;
mod platform;
mod queue;
mod session;

pub use failclosed::{current_session_id, disconnect_session};
pub use session::{DevConfig, DevError, DevLock, OVERLAY_MARKER, run_service_host};
