mod capture;
mod devices;
mod error;
mod hook;
mod power;
mod probe;
mod registry;

pub use capture::{CaptureMethod, MethodCapture, capture_marker, capture_near, window_pixels};
pub use devices::{DeviceInfo, snapshot_devices};
pub use error::HarnessError;
#[cfg(feature = "simulated-physical")]
pub use hook::classify_observed;
pub use hook::{HookObservation, observe_injected_key};
pub use power::{PowerHold, snapshot_power_requests};
pub use probe::{
    FocusState, KeyboardFocus, Probe, RecordedClick, RecordedInput, cursor_position,
    keyboard_focus, send_left_click, send_mouse_absolute, send_mouse_move, send_virtual_key,
};
pub use registry::{delete_tree, put_dword, snapshot_key, snapshot_values};
