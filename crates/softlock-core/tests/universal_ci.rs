//! Ticket m01. The macOS job builds both slices, merges them, and scans each one.
//!
//! The release-binary scanner does not understand a fat Mach-O. The job thins
//! each slice into its own directory and points `SOFTLOCK_RELEASE_SCAN_DIR` at
//! that directory, then at the fat file. A marker that sits outside a slice
//! still fails the fat scan.

mod common;

use std::fs;

use softlock_core::PRODUCT_DISPLAY_NAME;

const WORKFLOW: &str = ".github/workflows/universal.yml";

const PRODUCT_CRATES: &[&str] = &[
    "softlock-app",
    "softlock-service",
    "softlock-lockhost",
    "softlock-broker",
    "softlock-windows",
    "softlock-macos",
    "softlock-linux",
];

#[test]
fn universal_job_builds_both_slices_on_macos_latest() {
    let text = workflow();
    assert!(
        text.contains("runs-on: macos-latest"),
        "the arm64 slice is built on macos-latest"
    );
    let build = universal_build_job(&text);
    assert!(
        !build.contains("macos-15-intel") && !build.contains("macos-26-intel"),
        "Intel runner labels stay off the m01 build job"
    );
    assert!(text.contains("sw_vers"), "the job prints sw_vers");
    assert!(text.contains("MACOSX_DEPLOYMENT_TARGET"));
    assert!(text.contains("11.0"), "the deployment target is macOS 11");
    assert!(text.contains("rustup target add x86_64-apple-darwin aarch64-apple-darwin"));
    assert!(text.contains("cargo build --release --locked --target x86_64-apple-darwin"));
    assert!(text.contains("cargo build --release --locked --target aarch64-apple-darwin"));
    for package in PRODUCT_CRATES {
        assert!(
            text.contains(&format!("-p {package}")),
            "product crate {package} is missing from the release build"
        );
    }
    assert!(
        !text.contains("--features") && !text.contains("--all-features"),
        "release builds do not enable test-only features"
    );
    assert!(!text.contains("simulated-physical"));
    assert!(!text.contains("dev-launch"));
    assert!(!text.contains("rustfmt"));
    assert!(!text.contains("clippy"));
    assert!(!text.contains("both_archs() {"));
    assert!(!text.contains("D3"));
    assert!(text.contains("Decision D2"));
    assert!(text.matches(".github/universal-scan.sh").count() >= 2);
    assert!(text.contains("scan_fails_when_fat_binary_has_markers"));
    assert!(
        !text.contains(PRODUCT_DISPLAY_NAME),
        "the workflow must use the name constant's crate, not a hard-coded display name"
    );
    assert!(!text.contains("IOPlatformSerialNumber"));
    assert!(!text.contains("system_profiler"));
    assert!(!text.contains("Developer ID"));
    assert!(text.contains("contents: read"));
    assert!(!text.contains("actions: write"));
    assert!(text.contains("pull_request:"));
    assert!(text.contains("push:"));
    assert_actions_are_pinned(&text);
}

#[test]
fn universal_job_lipos_thins_and_scans_each_slice_and_the_app() {
    let text = workflow();
    assert!(text.contains("lipo -create"));
    assert!(text.contains("lipo -info"));
    assert!(text.contains("lipo -archs"));
    assert!(text.contains("lipo -thin x86_64"));
    assert!(text.contains("lipo -thin arm64"));
    assert!(text.contains("scan_release target/universal/x86_64 "));
    assert!(text.contains("scan_release target/universal/arm64 "));
    assert!(text.contains("scan_release target/universal/fat "));
    assert!(text.contains("scan_release target/x86_64-apple-darwin/release "));
    assert!(text.contains("scan_release target/aarch64-apple-darwin/release "));
    let script = scan_script();
    assert!(script.contains("both_archs() {"));
    assert!(script.contains("scan_release() {"));
    assert!(script.contains("require_minos() {"));
    assert!(script.contains("-arch all"));
    assert!(script.contains("for slice_arch in x86_64 arm64"));
    assert!(script.contains("otool -arch \"${slice_arch}\" -l"));
    assert!(script.contains("vtool -arch \"${slice_arch}\" -show-build"));
    assert!(script.contains("echo \"${slice_arch}: minos 11.0\""));
    assert!(script.contains("echo \"${slice_arch}: version 11.0\""));
    assert!(!script.contains("minos_count"));
    assert!(script.contains("=== otool minos "));
    assert!(script.contains("minos 11.0"));
    assert!(script.contains("SOFTLOCK_RELEASE_SCAN_DIR"));
    assert!(script.contains(
        "cargo test --locked -p softlock-lockhost --test release_binary -- --nocapture product_release_binaries"
    ));
    assert!(script.contains("scanned release binaries"));
    assert!(script.contains("softlock-dev-launch-marker"));
    assert!(script.contains("softlock_dev_launch_marker"));
    assert!(script.contains("softlock-simulated-physical-marker"));
    assert!(script.contains("softlock_simulated_physical_marker"));
    assert!(script.contains("softlock-live-fail-closed-marker"));
    assert!(script.contains("softlock_live_fail_closed_marker"));
    assert!(script.contains("string=true symbol=true dev_string=true dev_symbol=true"));
    assert!(script.contains("Decision D2"));
    assert!(!script.contains("D3"));
    let scanner = fs::read_to_string(
        common::repo_root().join("crates/softlock-lockhost/tests/release_binary.rs"),
    )
    .expect("read release_binary.rs");
    assert!(scanner.contains("softlock-dev-launch-marker"));
    assert!(scanner.contains("softlock_dev_launch_marker"));
    assert!(text.contains("npm run tauri build -- --target universal-apple-darwin"));
    assert!(
        text.contains("scan_release target/universal/tauri "),
        "the universal app executable has to be in a release scan"
    );
}

#[test]
fn universal_job_smoke_launches_only_the_arm64_slice() {
    let text = workflow();
    let smoke = step(&text, "Smoke launch the arm64 slice");
    assert!(smoke.contains("uname -m"), "the smoke step prints uname -m");
    assert!(
        smoke.contains("target/universal/arm64"),
        "the smoke step launches the thin arm64 slice"
    );
    assert!(smoke.contains("x86_64 slice is not executed"));
    assert!(
        !smoke.contains("target/universal/x86_64"),
        "the smoke step must not treat the x86_64 slice as tested"
    );
    assert!(!smoke.contains("x86_64-apple-darwin"));
    assert!(!smoke.contains("lipo -thin x86_64"));
    assert!(!smoke.contains("arch -x86_64"));
}

fn step(text: &str, name: &str) -> String {
    let marker = format!("- name: {name}\n");
    let start = text
        .find(&marker)
        .unwrap_or_else(|| panic!("missing step {name}"));
    let rest = &text[start + marker.len()..];
    let end = rest.find("\n      - name:").unwrap_or(rest.len());
    rest[..end].to_string()
}

fn universal_build_job(text: &str) -> String {
    let marker = "  universal:\n";
    let start = text.find(marker).expect("universal job");
    let rest = &text[start + marker.len()..];
    let end = rest.find("\n  native_x86:").expect("native_x86 job");
    rest[..end].to_string()
}

fn workflow() -> String {
    let path = common::repo_root().join(WORKFLOW);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn scan_script() -> String {
    let path = common::repo_root().join(".github/universal-scan.sh");
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn assert_actions_are_pinned(text: &str) {
    let mut uses = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("- uses:") else {
            continue;
        };
        uses += 1;
        let reference = rest.split('#').next().unwrap_or(rest).trim();
        let Some(sha) = reference.rsplit('@').next() else {
            panic!("action is not pinned: {trimmed}");
        };
        assert_eq!(sha.len(), 40, "action pin must be a full SHA: {trimmed}");
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "action pin must be hexadecimal: {trimmed}"
        );
    }
    assert!(
        uses >= 3,
        "universal workflow should pin checkout, toolchain, and cache"
    );
}
