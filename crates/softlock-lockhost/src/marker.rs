//! Marker bytes and the exported symbol. This file is compiled only when the
//! `simulated-physical` feature is on, so a release build never parses the
//! names the binary check rejects.

/// Hyphenated marker. The release-binary check rejects this byte string.
pub const SIMULATED_PHYSICAL_MARKER: &[u8] = b"softlock-simulated-physical-marker";

/// Symbol the release-binary check rejects. Present only with the test feature.
#[used]
#[unsafe(no_mangle)]
pub static softlock_simulated_physical_marker: [u8; SIMULATED_PHYSICAL_MARKER.len()] =
    *b"softlock-simulated-physical-marker";

/// Keeps the marker bytes reachable from the crate's public surface.
#[inline(never)]
pub fn marker_bytes() -> &'static [u8] {
    &softlock_simulated_physical_marker
}
