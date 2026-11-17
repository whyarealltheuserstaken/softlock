//! Ticket m11. The x86_64 slice runs natively on GitHub Intel macOS runners.

mod common;

use std::fs;

use softlock_core::PRODUCT_DISPLAY_NAME;

const WORKFLOW: &str = ".github/workflows/universal.yml";
const FINDING: &str = "docs/findings/m11-x86-slice.md";
const SPIKE: &str = "spikes/macos-desktop-proof";
const TICKET: &str = "docs/tickets/macos/m11-x86-slice-intel.md";

const BANNED: &[&str] = &[
    "IOPlatformSerialNumber",
    "Serial Number",
    "system_profiler",
    "ioreg",
];

#[test]
fn native_job_downloads_m01_fat_artifacts_on_github_intel() {
    let text = workflow();
    let build = job_block(&text, "universal:");
    let native = job_block(&text, "native_x86:");
    assert!(build.contains("runs-on: macos-latest"));
    assert!(
        !build.contains("macos-15-intel") && !build.contains("macos-26-intel"),
        "Intel runner labels stay off the m01 build job"
    );
    assert!(native.contains("needs: universal"));
    assert!(native.contains("if: github.event_name != 'push'"));
    assert!(native.contains("runs-on: ${{ matrix.image }}"));
    assert!(native.contains("macos-15-intel"));
    assert!(native.contains("macos-26-intel"));
    assert!(!native.contains("arch -x86_64"));
    assert!(native.contains("hw.cputype"));
    assert!(native.contains("machdep.cpu.brand_string"));
    assert!(native.contains("*Intel*"));
    assert!(native.contains("sysctl.proc_translated"));
    assert!(native.contains("hw.optional.arm64"));
    assert!(native.contains("this is Rosetta"));
    assert!(native.contains("lipo -info"));
    assert!(native.contains("lipo -archs"));
    assert!(native.contains("--smoke"));
    assert!(native.contains("native-intel=true"));
    assert!(native.contains("rosetta=false"));
    assert!(native.contains("cg-main-display-id="));
    assert!(native.contains("sac-lock-called=false"));
    assert!(native.contains("smoke=pass"));
    assert!(native.contains("m01-fat/fat/*"));
    assert!(native.contains("m01-fat/tauri/*"));
    assert!(native.contains("m01-fat/probe/*"));
    assert!(build.contains("target/universal/probe"));
    assert!(build.contains(SPIKE));
    assert!(build.contains("actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02"));
    assert!(native.contains("actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093"));
    assert!(text.contains("runner-images/issues/13045"));
    assert!(text.contains(
        "github.blog/changelog/2025-09-19-github-actions-macos-13-runner-image-is-closing-down"
    ));
    assert!(text.contains("August 2027"));
    assert!(text.contains("contents: read"));
    assert!(!text.contains("actions: write"));
    assert!(!text.contains(PRODUCT_DISPLAY_NAME));
    for banned in BANNED {
        assert!(!text.contains(banned), "workflow contains {banned}");
    }
    assert_actions_are_pinned(&text);
}

#[test]
fn finding_says_native_intel_not_rosetta_and_keeps_henrys_mac() {
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
    assert!(text.contains("hw.cputype"));
    assert!(text.contains("August 2027"));
    assert!(text.contains("runner-images/issues/13045"));
    assert!(
        text.contains("does not run on push to main") || text.contains("no push trigger on main")
    );
    assert!(
        text.contains("Henry's Intel Mac"),
        "the finding has to keep his machine"
    );
    assert!(
        !text.contains("not required"),
        "the lead kept Henry's Mac on this ticket"
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

#[test]
fn ticket_keeps_henrys_mac_as_an_optional_kit_checkbox() {
    let text = fs::read_to_string(common::repo_root().join(TICKET)).expect("m11 ticket");
    assert!(text.contains("Henry's Intel Mac"));
    assert!(!text.contains("not required"));
    assert!(text.contains("optional"));
    assert!(text.contains("macos-15-intel"));
    assert!(text.contains("--smoke"));
}

fn workflow() -> String {
    fs::read_to_string(common::repo_root().join(WORKFLOW)).expect("universal workflow")
}

fn job_block(text: &str, heading: &str) -> String {
    let marker = format!("  {heading}\n");
    let start = text
        .find(&marker)
        .unwrap_or_else(|| panic!("missing job {heading}"));
    let rest = &text[start + marker.len()..];
    if heading == "native_x86:" {
        return rest.to_string();
    }
    let native = rest
        .find("\n  native_x86:")
        .unwrap_or_else(|| panic!("native_x86 should follow {heading}"));
    rest[..native].to_string()
}

fn assert_actions_are_pinned(text: &str) {
    let mut uses = 0;
    for line in text.lines() {
        let trimmed = line.trim().trim_start_matches("- ").trim();
        let Some(rest) = trimmed.strip_prefix("uses:") else {
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
        uses >= 5,
        "universal workflow should pin checkout, toolchain, cache, upload, and download"
    );
}
