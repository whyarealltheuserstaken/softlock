//! Keeps the dev-launch dependencies referenced on hosts that do not compile
//! the Windows seam. The lock host itself runs on Windows.

#![cfg(all(not(windows), feature = "dev-launch"))]

#[test]
fn dev_launch_runs_on_windows() {
    let _ = softlock_lockhost::dev_launch_marker_bytes();
    let _ = std::any::type_name::<softlock_core::Secret>();
}
