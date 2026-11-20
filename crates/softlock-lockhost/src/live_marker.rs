//! Marker bytes and the exported symbol. Compiled only with `live-fail-closed`.

pub const LIVE_FAIL_CLOSED_MARKER: &[u8] = b"softlock-live-fail-closed-marker";

#[used]
#[unsafe(no_mangle)]
pub static softlock_live_fail_closed_marker: [u8; LIVE_FAIL_CLOSED_MARKER.len()] =
    *b"softlock-live-fail-closed-marker";

#[inline(never)]
pub fn marker_bytes() -> &'static [u8] {
    &softlock_live_fail_closed_marker
}
