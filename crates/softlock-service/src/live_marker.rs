//! Marker bytes. This file is compiled only with `live-fail-closed`, so a
//! release, universal, or product build never contains the names the binary
//! check rejects.

const MARKER_STRING: &[u8] = b"softlock-live-fail-closed-marker";
const MARKER_SYMBOL: &[u8] = b"softlock_live_fail_closed_marker";

#[inline(never)]
pub fn marker_bytes() -> &'static [u8] {
    let _ = std::hint::black_box(MARKER_SYMBOL);
    std::hint::black_box(MARKER_STRING)
}
