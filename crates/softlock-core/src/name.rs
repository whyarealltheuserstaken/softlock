/// User-visible product name. The settings app and the docs use this.
/// Rename this constant and the string resources together.
pub const PRODUCT_DISPLAY_NAME: &str = "SoftLock";

/// Bundle id for the settings app. This is the product, not the background service.
pub const APP_BUNDLE_ID: &str = "com.softlock.app";

/// npm package name of the settings app. The package is private.
pub const APP_PACKAGE_NAME: &str = "softlock";

/// Display name of the background service. The service ids below are computed from this.
pub const SERVICE_DISPLAY_NAME: &str = "SoftLock Agent";

const SERVICE_DISPLAY_BYTES: &[u8] = SERVICE_DISPLAY_NAME.as_bytes();
const SERVICE_ID_LEN: usize = SERVICE_DISPLAY_BYTES.len();
const SERVICE_ID_PASCAL_LEN: usize = SERVICE_ID_LEN - count_spaces(SERVICE_DISPLAY_BYTES);

const SERVICE_ID_BYTES: [u8; SERVICE_ID_LEN] = kebab_case(SERVICE_DISPLAY_BYTES);
const SERVICE_ID_PASCAL_BYTES: [u8; SERVICE_ID_PASCAL_LEN] = without_spaces(SERVICE_DISPLAY_BYTES);

/// Hyphenated id computed from [`SERVICE_DISPLAY_NAME`]. Named pipes use this form.
pub const SERVICE_ID: &str = utf8(&SERVICE_ID_BYTES);

/// PascalCase id computed from [`SERVICE_DISPLAY_NAME`].
/// The Windows service name and registry subkey use this form.
pub const SERVICE_ID_PASCAL: &str = utf8(&SERVICE_ID_PASCAL_BYTES);

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

const fn count_spaces(bytes: &[u8]) -> usize {
    let mut spaces = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b' ' {
            spaces += 1;
        }
        index += 1;
    }
    spaces
}

/// Lowercase ASCII letters and replace spaces with hyphens. Length is unchanged.
const fn kebab_case(bytes: &[u8]) -> [u8; SERVICE_ID_LEN] {
    let mut out = [0; SERVICE_ID_LEN];
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        out[index] = if byte == b' ' {
            b'-'
        } else {
            byte.to_ascii_lowercase()
        };
        index += 1;
    }
    out
}

/// Drop spaces, keeping the original letter case.
const fn without_spaces(bytes: &[u8]) -> [u8; SERVICE_ID_PASCAL_LEN] {
    let mut out = [0; SERVICE_ID_PASCAL_LEN];
    let mut index = 0;
    let mut written = 0;
    while index < bytes.len() {
        if bytes[index] != b' ' {
            out[written] = bytes[index];
            written += 1;
        }
        index += 1;
    }
    out
}

const fn utf8(bytes: &[u8]) -> &str {
    match str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => panic!("service id is not utf-8"),
    }
}
