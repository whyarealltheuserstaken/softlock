//! Lock host. Overlay windows and the input guard arrive in a later ticket.
//!
//! The test-only `simulated-physical` feature classifies nonce-tagged injected
//! events. Release builds leave the feature off, and those binaries must not
//! contain its marker string or symbol.

mod classify;

#[cfg(feature = "simulated-physical")]
mod marker;

pub use classify::{HookInput, InputClass, classify};
#[cfg(feature = "simulated-physical")]
pub use classify::{RunNonce, clear_run_nonce, set_run_nonce};
