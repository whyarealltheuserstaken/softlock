//! The journal test presents a PIN as physical keyboard input. That path is
//! not in a product build. This file is compiled only with
//! `simulated-physical` or `live-fail-closed`, and the release scan rejects
//! the marker below.

const MARKER_STRING: &[u8] = b"softlock-present-physical-pin";

#[inline(never)]
pub fn marker_bytes() -> &'static [u8] {
    std::hint::black_box(MARKER_STRING)
}
