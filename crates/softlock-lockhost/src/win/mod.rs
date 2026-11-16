//! Windows dev launch. Absent from builds that do not enable `dev-launch`.

mod failclosed;
mod guard;
mod overlay;
mod platform;
mod queue;
mod session;

pub use session::{DevConfig, DevError, DevLock, OVERLAY_MARKER};
