/// User-visible product name. The settings app and the docs use this.
/// Rename this constant and the string resources together.
pub const PRODUCT_DISPLAY_NAME: &str = "SoftLock";

/// Bundle id for the settings app. This is the product, not the background service.
pub const APP_BUNDLE_ID: &str = "com.softlock.app";

/// npm package name of the settings app. The package is private.
pub const APP_PACKAGE_NAME: &str = "softlock";

/// Display name of the background service.
pub const SERVICE_DISPLAY_NAME: &str = "SoftLock Agent";

/// Hyphenated id derived from the service display name.
/// Named pipes and other path-like ids use this form.
pub const SERVICE_ID: &str = "softlock-agent";

/// PascalCase id derived from the service display name.
/// The Windows service name and registry subkey use this form.
pub const SERVICE_ID_PASCAL: &str = "SoftLockAgent";

/// Windows service name.
pub fn service_name() -> &'static str {
    SERVICE_ID_PASCAL
}

/// Named-pipe path.
pub fn pipe_name() -> String {
    format!(r"\\.\pipe\{SERVICE_ID}")
}

/// Registry subkey under HKLM or HKCU.
pub fn registry_key() -> String {
    format!(r"Software\{SERVICE_ID_PASCAL}")
}
