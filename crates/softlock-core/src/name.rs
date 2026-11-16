/// User-visible product name. Rename this constant and the string resources together.
pub const PRODUCT_DISPLAY_NAME: &str = "SoftLock";

/// Stable identifier for the Windows service name, named pipe, and registry key.
/// It is not the product display name and stays put when the product is renamed.
pub const INTERNAL_ID: &str = "deskguard";

/// Windows service name. This is the internal identifier, not the display name.
pub fn service_name() -> &'static str {
    INTERNAL_ID
}

/// Named-pipe path. This is the internal identifier, not the display name.
pub fn pipe_name() -> String {
    format!(r"\\.\pipe\{INTERNAL_ID}")
}

/// Registry subkey under HKLM or HKCU. This is the internal identifier, not the display name.
pub fn registry_key() -> String {
    format!(r"Software\{INTERNAL_ID}")
}
