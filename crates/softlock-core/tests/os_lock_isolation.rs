//! The OS-lock spike stays out of the product workspace.

mod common;

use std::fs;
use std::process::Command;

use softlock_core::PRODUCT_DISPLAY_NAME;

const SPIKE_MANIFEST: &str = "spikes/os-lock/Cargo.toml";
const SPIKE_PACKAGE: &str = "os-lock-proof";
const PROOF_WORKFLOW: &str = ".github/workflows/os-lock-proof.yml";

#[test]
fn os_lock_proof_is_excluded_from_the_product_workspace() {
    let root = common::repo_root();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
    assert!(
        manifest.contains("spikes/os-lock"),
        "workspace exclude list must name the OS-lock spike"
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
            "OS-lock proof must not be a workspace member"
        );
        let manifest_path = package["manifest_path"].as_str().unwrap_or("");
        assert!(
            !manifest_path.replace('\\', "/").contains("spikes/os-lock"),
            "{name} resolves inside the OS-lock spike"
        );
    }
}

#[test]
fn os_lock_proof_is_unpublished_and_product_crates_do_not_depend_on_it() {
    let root = common::repo_root();
    let spike = fs::read_to_string(root.join(SPIKE_MANIFEST)).expect("spike manifest");
    assert!(spike.contains("publish = false"));
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
            !text.contains(SPIKE_PACKAGE) && !text.contains("spikes/os-lock"),
            "{rel} must not depend on the OS-lock spike"
        );
    }
}

#[test]
fn os_lock_workflow_is_last_isolated_and_pins_actions() {
    let root = common::repo_root();
    let text = fs::read_to_string(root.join(PROOF_WORKFLOW)).expect("proof workflow");
    assert!(text.contains("windows-2022"));
    assert!(text.contains("windows-2025"));
    assert!(text.contains("needs: checks"));
    assert!(text.contains("if: always()"));
    assert!(text.contains("OsLockProbe"));
    assert!(text.contains("spikes/os-lock"));
    assert!(text.contains("workflow_dispatch:"));
    assert!(text.contains("pull_request:"));
    assert!(text.contains("contents: read"));
    assert!(!text.contains(PRODUCT_DISPLAY_NAME));
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
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }
    assert!(uses >= 3, "proof workflow should pin its actions");
}

#[test]
fn os_lock_script_restores_and_does_not_record_a_serial() {
    let root = common::repo_root();
    let run = fs::read_to_string(root.join("spikes/os-lock/run.ps1")).expect("run.ps1");
    let restore = fs::read_to_string(root.join("spikes/os-lock/restore.ps1")).expect("restore");
    assert!(run.contains("try {"));
    assert!(run.contains("} finally {"));
    assert!(run.contains("pending.json"));
    assert!(run.contains("RunOnce"));
    assert!(run.contains("stay-disconnected"));
    assert!(run.contains("thinkstation p7"));
    assert!(restore.contains("Invoke-PendingRestore"));
    assert!(restore.contains("OsLockProbe"));
    for text in [&run, &restore] {
        assert!(!text.contains("SerialNumber"));
        assert!(!text.contains("SystemSerial"));
        assert!(!text.contains(PRODUCT_DISPLAY_NAME));
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
