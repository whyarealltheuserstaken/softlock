//! The m06 overlay kit stays out of the product workspace and release binaries.

mod common;

use std::fs;
use std::process::Command;

use softlock_core::PRODUCT_DISPLAY_NAME;

const SPIKE_MANIFEST: &str = "spikes/macos-overlay-kit/Cargo.toml";
const SPIKE_PACKAGE: &str = "macos-overlay-kit";
const KIT_WORKFLOW: &str = ".github/workflows/macos-overlay-kit.yml";
const FINDING: &str = "docs/findings/m06-overlay/FINDING.md";

const BANNED: &[&str] = &[
    "IOPlatformSerialNumber",
    "Serial Number",
    "system_profiler",
    "ioreg",
    "SACLockScreenImmediate",
    "login.framework",
    "com.softlock.agent",
];

const ACCEPTANCE_TESTS: &[&str] = &[
    "fn each_capture_path_records_marker_counts_for_default_and_none_sharing",
    "fn every_attached_display_is_covered_and_a_hotplug_is_covered_or_a_precise_skip",
    "fn a_space_switch_and_a_full_screen_app_keep_the_overlay_or_name_the_cover",
    "fn a_click_falls_through_and_keyboard_focus_does_not_move",
    "fn command_tab_force_quit_and_power_key_each_have_a_row",
    "fn the_finding_tells_m14_which_capture_paths_it_may_claim",
    "fn stayed_up_covered_fell_through_held_and_swallowed_are_passes",
    "fn miss_covered_by_blocked_and_moved_are_fails",
    "fn presentation_records_swallowed_as_pass_and_desktop_reachable_with_d20_d6",
    "fn cover_count_must_match_screen_count_after_sync",
    "fn no_external_display_is_rejected_when_a_second_screen_was_attached",
    "fn screen_recording_preflight_never_passes_capture",
    "fn measured_and_typed_click_and_focus_must_agree",
    "fn ctrl_c_during_a_prompt_does_not_wait_for_enter",
    "fn timeout_watchdog_sets_stop_without_an_enter_key",
    "fn measured_blocked_with_no_typed_answer_fails",
];

#[test]
fn macos_overlay_kit_is_excluded_from_the_product_workspace() {
    let root = common::repo_root();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
    assert!(
        manifest.contains("spikes/macos-overlay-kit"),
        "workspace exclude list must name the overlay kit"
    );

    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(&root)
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let meta: serde_json::Value = serde_json::from_slice(&output.stdout).expect("metadata json");
    let packages = meta["packages"].as_array().expect("workspace packages");
    for package in packages {
        let name = package["name"].as_str().expect("package name");
        assert_ne!(
            name, SPIKE_PACKAGE,
            "overlay kit must not be a workspace member"
        );
        let manifest_path = package["manifest_path"].as_str().unwrap_or("");
        assert!(
            !manifest_path
                .replace('\\', "/")
                .contains("spikes/macos-overlay-kit"),
            "{name} resolves inside the overlay kit"
        );
    }
}

#[test]
fn macos_overlay_kit_is_unpublished_and_product_crates_do_not_depend_on_it() {
    let root = common::repo_root();
    let spike = fs::read_to_string(root.join(SPIKE_MANIFEST)).expect("spike manifest");
    assert!(
        spike.contains("publish = false"),
        "overlay kit must set publish = false"
    );
    assert!(
        !spike.contains(PRODUCT_DISPLAY_NAME),
        "spike manifest must not hard-code the product name"
    );

    let mut manifests = Vec::new();
    collect_manifests(&root.join("crates"), &mut manifests);
    assert!(!manifests.is_empty());
    for path in manifests {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        assert!(
            !text.contains(SPIKE_PACKAGE) && !text.contains("spikes/macos-overlay-kit"),
            "{rel} must not depend on the overlay kit"
        );
    }
}

#[test]
fn macos_overlay_workflow_runs_the_runner_slice_and_pins_actions() {
    let root = common::repo_root();
    let text = fs::read_to_string(root.join(KIT_WORKFLOW)).expect("kit workflow");
    assert!(text.contains("macos-latest"));
    assert!(text.contains("macos-15-intel"));
    assert!(text.contains("macos-26-intel"));
    assert!(text.contains("sw_vers"));
    assert!(text.contains("uname -m"));
    assert!(text.contains("spikes/macos-overlay-kit"));
    assert!(text.contains("workflow_dispatch:"));
    assert!(text.contains("pull_request:"));
    assert!(text.contains(".github/workflows/macos-overlay-kit.yml"));
    assert!(text.contains("contents: read"));
    assert!(text.contains("--runner"));
    assert!(text.contains("--locked"));
    assert!(text.contains("overlay-output"));
    assert!(text.contains("result.json"));
    assert!(text.contains("result.txt"));
    assert!(text.contains("sharing-m14-"));
    assert!(!text.contains("sudo"));
    assert!(!text.contains("launchctl"));
    assert!(
        !text.contains("actions: write"),
        "kit workflow must not request actions: write"
    );
    assert!(
        !text.contains(PRODUCT_DISPLAY_NAME),
        "kit workflow must not hard-code the product name"
    );
    let trigger = text.split("permissions:").next().expect("permissions");
    assert!(
        !trigger
            .lines()
            .any(|line| line.trim() == "push:" || line.trim().starts_with("push:")),
        "kit workflow must not run on every push"
    );

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
        "kit workflow should pin checkout, toolchain, and cache"
    );
}

#[test]
fn macos_overlay_does_not_record_a_serial_or_lock_the_session() {
    let root = common::repo_root();
    let mut files = Vec::new();
    collect_text(&root.join("spikes/macos-overlay-kit"), &mut files);
    files.push(root.join(KIT_WORKFLOW));
    assert!(!files.is_empty());
    for path in files {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        assert!(
            !text.contains(PRODUCT_DISPLAY_NAME),
            "{rel} hard-codes the product name"
        );
        for banned in BANNED {
            assert!(!text.contains(banned), "{rel} contains {banned}");
        }
    }
    let probe =
        fs::read_to_string(root.join("spikes/macos-overlay-kit/src/mac/probe.m")).expect("probe");
    assert!(!probe.contains("CGShieldingWindowLevel"));
    assert!(probe.contains("NSScreenSaverWindowLevel"));
    assert!(probe.contains("CanJoinAllSpaces"));
    assert!(probe.contains("FullScreenAuxiliary"));
    assert!(probe.contains("ignoresMouseEvents"));
    assert!(probe.contains("canBecomeKeyWindow"));
}

#[test]
fn each_acceptance_checkbox_has_a_named_test_and_the_overlay_is_always_torn_down() {
    let root = common::repo_root();
    let report =
        fs::read_to_string(root.join("spikes/macos-overlay-kit/src/report.rs")).expect("report");
    let proof =
        fs::read_to_string(root.join("spikes/macos-overlay-kit/src/mac/proof.rs")).expect("proof");
    let wait = fs::read_to_string(root.join("spikes/macos-overlay-kit/src/wait.rs")).expect("wait");
    let sources = format!("{report}\n{wait}\n{proof}");
    for name in ACCEPTANCE_TESTS {
        assert!(sources.contains(name), "missing {name}");
    }
    assert!(proof.contains("SIGHUP"));
    assert!(proof.contains("m06_overlay_close") || proof.contains("OverlayGuard::close"));
    assert!(proof.contains("no-input"));
    assert!(proof.contains("result.json"));
    assert!(proof.contains("result.txt"));
    assert!(proof.contains("no-external-display"));
    assert!(proof.contains("POWER KEY LAST"));
    assert!(proof.contains("poll_stdin"));
    assert!(proof.contains("wait_visible"));
    assert!(report.contains("CGPreflightScreenCaptureAccess"));
    let probe =
        fs::read_to_string(root.join("spikes/macos-overlay-kit/src/mac/probe.m")).expect("probe");
    assert!(probe.contains("CGPreflightScreenCaptureAccess"));
    assert!(probe.contains("m06_overlay_wait_visible"));
    let readme =
        fs::read_to_string(root.join("spikes/macos-overlay-kit/README.md")).expect("readme");
    assert!(readme.contains("cursor/m06-overlay-intel-kit-feda"));
    assert!(readme.contains("git checkout -B"));
    assert!(readme.contains("git rev-parse HEAD"));
    assert!(readme.contains("Accessibility"));
    assert!(readme.contains("timeout"));
    assert!(proof.contains("_exit"));
    assert!(probe.contains("screencapture timed out") || probe.contains("task.terminate"));
}

#[test]
fn the_finding_tells_m14_which_capture_paths_it_may_claim() {
    let root = common::repo_root();
    let text = fs::read_to_string(root.join(FINDING)).expect("finding");
    assert!(text.contains("m14"));
    assert!(text.contains("do-not-claim"));
    assert!(text.contains("D6"));
    assert!(text.contains("Decision 20"));
    assert!(!text.contains(PRODUCT_DISPLAY_NAME));
    assert!(!text.contains("IOPlatformSerialNumber"));
}

fn collect_text(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name() == "target" {
                continue;
            }
            collect_text(&path, files);
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".rs")
            || name.ends_with(".m")
            || name.ends_with(".md")
            || name.ends_with(".yml")
            || name == "Cargo.toml"
        {
            files.push(path);
        }
    }
}

fn collect_manifests(dir: &std::path::Path, manifests: &mut Vec<std::path::PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_manifests(&path, manifests);
            continue;
        }
        if entry.file_name() == "Cargo.toml" {
            manifests.push(path);
        }
    }
}
