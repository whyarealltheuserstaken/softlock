//! Ticket m11. The x86_64 slice runs natively on GitHub Intel macOS runners.

mod common;

use std::fs;

use softlock_core::PRODUCT_DISPLAY_NAME;

const WORKFLOW: &str = ".github/workflows/macos-x86-slice.yml";
const FINDING: &str = "docs/findings/m11-x86-slice.md";
const SPIKE: &str = "spikes/macos-desktop-proof";

const BANNED: &[&str] = &[
    "IOPlatformSerialNumber",
    "Serial Number",
    "system_profiler",
    "ioreg",
];

#[test]
fn native_job_uses_github_intel_labels_not_rosetta() {
    let text = workflow();
    assert!(text.contains("runs-on: ${{ matrix.image }}"));
    assert!(text.contains("macos-15-intel"));
    assert!(text.contains("macos-26-intel"));
    assert!(
        !job_runs_on_latest(&text),
        "the native job must not run on macos-latest"
    );
    assert!(!text.contains("arch -x86_64"));
    assert!(text.contains("uname -m"));
    assert!(text.contains("sysctl.proc_translated"));
    assert!(text.contains("hw.optional.arm64"));
    assert!(text.contains("this is Rosetta"));
    assert!(text.contains("lipo -info"));
    assert!(text.contains("lipo -archs"));
    assert!(text.contains("lipo -create"));
    assert!(text.contains("--smoke"));
    assert!(text.contains("native-intel=true"));
    assert!(text.contains("rosetta=false"));
    assert!(text.contains("smoke=pass"));
    assert!(text.contains("MACOSX_DEPLOYMENT_TARGET"));
    assert!(text.contains("11.0"));
    assert!(text.contains("x86_64-apple-darwin"));
    assert!(text.contains("aarch64-apple-darwin"));
    assert!(text.contains(SPIKE));
    assert!(text.contains("Decision D2"));
    assert!(text.contains("contents: read"));
    assert!(!text.contains("actions: write"));
    assert!(!text.contains("bootout"));
    assert!(!text.contains(PRODUCT_DISPLAY_NAME));
    for banned in BANNED {
        assert!(!text.contains(banned), "workflow contains {banned}");
    }
    assert_actions_are_pinned(&text);
}

#[test]
fn finding_says_native_intel_not_rosetta_and_omits_a_serial() {
    let text = fs::read_to_string(common::repo_root().join(FINDING)).expect("m11 finding");
    assert!(text.contains("native Intel"));
    assert!(text.contains("not a Rosetta run"));
    assert!(text.contains("macos-15-intel"));
    assert!(text.contains("macos-26-intel"));
    assert!(text.contains("uname -m"));
    assert!(text.contains("x86_64"));
    assert!(text.contains("arm64"));
    assert!(text.contains("lipo -info"));
    assert!(text.contains("lipo -archs"));
    assert!(
        text.contains("Henry's Intel Mac"),
        "the finding has to say what still waits on his machine"
    );
    assert!(!text.contains(PRODUCT_DISPLAY_NAME));
    for banned in [
        "IOPlatformSerialNumber",
        "Serial Number:",
        "serial number",
        "system_profiler",
    ] {
        assert!(!text.contains(banned), "finding contains {banned}");
    }
}

fn workflow() -> String {
    fs::read_to_string(common::repo_root().join(WORKFLOW)).expect("x86 slice workflow")
}

fn job_runs_on_latest(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim() == "runs-on: macos-latest")
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
        "x86 slice workflow should pin checkout, toolchain, and cache"
    );
}
