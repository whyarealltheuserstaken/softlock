#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(feature = "live-fail-closed")]
    let _ = live_fail_closed_marker();
    if std::env::args().any(|arg| arg == "--owner-session") {
        let code = softlock_app_lib::run_owner_session();
        std::process::exit(code);
    }
    softlock_app_lib::run();
}

#[cfg(feature = "live-fail-closed")]
fn live_fail_closed_marker() -> &'static [u8] {
    let symbol: &[u8] = b"softlock_live_fail_closed_marker";
    let text: &[u8] = b"softlock-live-fail-closed-marker";
    let _ = std::hint::black_box(symbol);
    std::hint::black_box(text)
}
