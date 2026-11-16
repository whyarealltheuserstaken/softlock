//! Decision logic for the Windows Hello consent spike.
//!
//! No operating-system calls. The Windows binary applies these decisions.
//! This crate is not a product dependency and must not ship in a release.

mod identity;
mod limit;
mod passthrough;

pub use identity::{
    CREDENTIAL_UI_RELATIVE_PATHS, DialogGate, ImageFact, SignatureCheck,
    foreground_is_credential_ui, path_is_system_credential_ui,
};
pub use limit::{MAX_BLOCK, PASSTHROUGH_TIMEOUT, clamp_block_limit};
pub use passthrough::{
    Action, DeviceEvent, InputOrigin, ModifierKeys, Modifiers, PassthroughWindow, on_key,
    on_pointer, vk,
};

/// ASCII marker embedded in this spike and absent from product release binaries.
pub const NOT_FOR_RELEASE_MARKER: &str = "SPIKE_P6_NOT_FOR_RELEASE";

#[used]
static RELEASE_MARKER: &str = NOT_FOR_RELEASE_MARKER;
