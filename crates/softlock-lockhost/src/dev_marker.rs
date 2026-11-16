//! Marker bytes and the exported symbol. This file is compiled only when the
//! `dev-launch` feature is on, so a release build never contains the names
//! the binary check rejects.

/// Hyphenated marker. The release-binary check rejects this byte string.
pub const DEV_LAUNCH_MARKER: &[u8] = b"softlock-dev-launch-marker";

/// Symbol the release-binary check rejects. Present only with the dev feature.
#[used]
#[unsafe(no_mangle)]
pub static softlock_dev_launch_marker: [u8; DEV_LAUNCH_MARKER.len()] =
    *b"softlock-dev-launch-marker";

/// Keeps the marker bytes reachable from the dev-launch entry point.
#[inline(never)]
pub fn dev_launch_marker_bytes() -> &'static [u8] {
    &softlock_dev_launch_marker
}
