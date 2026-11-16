//! The macOS desktop-runner spike stays out of the product workspace and release binaries.

mod common;

use std::fs;
use std::process::Command;

use softlock_core::PRODUCT_DISPLAY_NAME;

const SPIKE_MANIFEST: &str = "spikes/macos-desktop-proof/Cargo.toml";
const SPIKE_PACKAGE: &str = "macos-desktop-proof";
const PROOF_WORKFLOW: &str = ".github/workflows/macos-desktop-proof.yml";
const AGENT_LABEL: &str = "com.runner.macosproof.agent";
const DAEMON_LABEL: &str = "com.runner.macosproof.daemon";

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
fn macos_desktop_proof_is_excluded_from_the_product_workspace() {
    let root = common::repo_root();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
    assert!(
        manifest.contains("spikes/macos-desktop-proof"),
        "workspace exclude list must name the macOS desktop proof spike"
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
            "macos desktop proof must not be a workspace member"
        );
        let manifest_path = package["manifest_path"].as_str().unwrap_or("");
        assert!(
            !manifest_path
                .replace('\\', "/")
                .contains("spikes/macos-desktop-proof"),
            "{name} resolves inside the macos desktop proof spike"
        );
    }
}

#[test]
fn macos_desktop_proof_is_unpublished_and_product_crates_do_not_depend_on_it() {
    let root = common::repo_root();
    let spike = fs::read_to_string(root.join(SPIKE_MANIFEST)).expect("spike manifest");
    assert!(
        spike.contains("publish = false"),
        "macos desktop proof must set publish = false"
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
            !text.contains(SPIKE_PACKAGE) && !text.contains("spikes/macos-desktop-proof"),
            "{rel} must not depend on the macos desktop proof spike"
        );
    }
}

#[test]
fn macos_desktop_proof_workflow_targets_arm64_and_pins_actions() {
    let root = common::repo_root();
    let text = fs::read_to_string(root.join(PROOF_WORKFLOW)).expect("proof workflow");
    assert!(text.contains("macos-latest"));
    assert!(text.contains("sw_vers"));
    assert!(text.contains("uname -m"));
    assert!(text.contains("spikes/macos-desktop-proof"));
    assert!(text.contains("workflow_dispatch:"));
    assert!(text.contains("pull_request:"));
    assert!(text.contains(".github/workflows/macos-desktop-proof.yml"));
    assert!(text.contains("contents: read"));
    assert!(text.contains("if: always()"));
    assert!(text.contains(AGENT_LABEL));
    assert!(text.contains(DAEMON_LABEL));
    assert!(text.contains("bootout"));
    let run = text.find("cargo run").expect("proof command");
    let bootout = text.rfind("bootout").expect("bootout");
    assert!(
        bootout > run,
        "the unload step has to run after the proof command"
    );
    assert!(
        !text.contains("actions: write"),
        "proof workflow must not request actions: write"
    );
    assert!(
        !text.contains(PRODUCT_DISPLAY_NAME),
        "proof workflow must not hard-code the product name"
    );
    let trigger = text.split("permissions:").next().expect("permissions");
    assert!(
        !trigger
            .lines()
            .any(|line| line.trim() == "push:" || line.trim().starts_with("push:")),
        "proof workflow must not run on every push"
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
        "proof workflow should pin checkout, toolchain, and cache"
    );
}

#[test]
fn macos_desktop_proof_does_not_record_a_serial_or_lock_the_session() {
    let root = common::repo_root();
    let mut files = Vec::new();
    collect_text(&root.join("spikes/macos-desktop-proof"), &mut files);
    files.push(root.join(PROOF_WORKFLOW));
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
