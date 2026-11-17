//! The m07 keep-awake kit stays out of the product workspace and release binaries.

mod common;

use std::fs;
use std::process::Command;

use softlock_core::PRODUCT_DISPLAY_NAME;

const SPIKE_MANIFEST: &str = "spikes/macos-keep-awake/Cargo.toml";
const SPIKE_PACKAGE: &str = "macos-keep-awake";
const KIT_WORKFLOW: &str = ".github/workflows/macos-keep-awake.yml";
const FINDING: &str = "docs/findings/m07-keep-awake/FINDING.md";

const BANNED: &[&str] = &[
    "IOPlatformSerialNumber",
    "Serial Number",
    "system_profiler",
    "ioreg",
    "SACLockScreenImmediate",
    "login.framework",
    "com.softlock.agent",
];

#[test]
fn macos_keep_awake_is_excluded_from_the_product_workspace() {
    let root = common::repo_root();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
    assert!(
        manifest.contains("spikes/macos-keep-awake"),
        "workspace exclude list must name the keep-awake kit"
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
            "keep-awake kit must not be a workspace member"
        );
        let manifest_path = package["manifest_path"].as_str().unwrap_or("");
        assert!(
            !manifest_path
                .replace('\\', "/")
                .contains("spikes/macos-keep-awake"),
            "{name} resolves inside the keep-awake kit"
        );
    }
}

#[test]
fn macos_keep_awake_is_unpublished_and_product_crates_do_not_depend_on_it() {
    let root = common::repo_root();
    let spike = fs::read_to_string(root.join(SPIKE_MANIFEST)).expect("spike manifest");
    assert!(
        spike.contains("publish = false"),
        "keep-awake kit must set publish = false"
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
            !text.contains(SPIKE_PACKAGE) && !text.contains("spikes/macos-keep-awake"),
            "{rel} must not depend on the keep-awake kit"
        );
    }
}

#[test]
fn macos_keep_awake_workflow_runs_the_runner_slice_and_pins_actions() {
    let root = common::repo_root();
    let text = fs::read_to_string(root.join(KIT_WORKFLOW)).expect("kit workflow");
    assert!(text.contains("macos-latest"));
    assert!(text.contains("sw_vers"));
    assert!(text.contains("uname -m"));
    assert!(text.contains("spikes/macos-keep-awake"));
    assert!(text.contains("workflow_dispatch:"));
    assert!(text.contains("pull_request:"));
    assert!(text.contains(".github/workflows/macos-keep-awake.yml"));
    assert!(text.contains("contents: read"));
    assert!(text.contains("--runner"));
    assert!(text.contains("--locked"));
    assert!(text.contains("IOPMAssertionCreateWithName") || text.contains("keep-awake"));
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
fn macos_keep_awake_does_not_record_a_serial_or_lock_the_session() {
    let root = common::repo_root();
    let mut files = Vec::new();
    collect_text(&root.join("spikes/macos-keep-awake"), &mut files);
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
}

#[test]
fn stop_handler_covers_hangup_and_the_overlay_follows_its_parent() {
    let root = common::repo_root();
    let proof =
        fs::read_to_string(root.join("spikes/macos-keep-awake/src/mac/proof.rs")).expect("proof");
    assert!(proof.contains("SIGHUP"));
    assert!(proof.contains("parent_alive(parent)"));
    assert!(proof.contains("timer-pid="));
    assert!(proof.contains("wait_for_app_nap_ready"));
    assert!(proof.contains("APP_NAP_READY_PROMPT"));
    let report =
        fs::read_to_string(root.join("spikes/macos-keep-awake/src/report.rs")).expect("report");
    assert!(report.contains("Press Return when Activity Monitor shows the App Nap column"));
    let timer = proof
        .split("fn run_timer")
        .nth(1)
        .expect("run_timer")
        .split("fn run_overlay")
        .next()
        .expect("timer body");
    assert!(
        timer.contains("parent_alive(parent)"),
        "timer child must exit when its parent dies"
    );
}

#[test]
fn the_finding_names_m16_and_idle_sleep_only() {
    let root = common::repo_root();
    let text = fs::read_to_string(root.join(FINDING)).expect("finding");
    assert!(text.contains("m16"));
    assert!(text.contains("idle"));
    assert!(text.contains("lid"));
    assert!(text.contains("laptop"));
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
