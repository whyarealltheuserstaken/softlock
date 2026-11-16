//! The fail-closed spike stays out of the product workspace and release binaries.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use softlock_core::PRODUCT_DISPLAY_NAME;

const SPIKE_MANIFEST: &str = "spikes/fail-closed-proof/Cargo.toml";
const SPIKE_PACKAGE: &str = "fail-closed-proof";
const PROOF_WORKFLOW: &str = ".github/workflows/fail-closed-proof.yml";
const CHECKLIST: &str = "docs/findings/04-fail-closed/CHECKLIST.md";
const FINDING: &str = "docs/findings/04-fail-closed/finding.md";
const SERVICE_NAME: &str = "RunnerFailClosedProbe";

const POLICIES: [&str; 8] = [
    "DisableLockWorkstation",
    "DisableTaskMgr",
    "DisableChangePassword",
    "NoLogoff",
    "NoClose",
    "HideFastUserSwitching",
    "ConsentPromptBehaviorAdmin",
    "ConsentPromptBehaviorUser",
];

#[test]
fn fail_closed_proof_is_excluded_from_the_product_workspace() {
    let root = common::repo_root();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
    assert!(
        manifest.contains("spikes/fail-closed-proof"),
        "workspace exclude list must name the fail-closed proof spike"
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
            "fail-closed proof must not be a workspace member"
        );
        let manifest_path = package["manifest_path"].as_str().unwrap_or("");
        assert!(
            !manifest_path
                .replace('\\', "/")
                .contains("spikes/fail-closed-proof"),
            "{name} resolves inside the fail-closed proof spike"
        );
    }
}

#[test]
fn fail_closed_proof_is_unpublished_and_product_crates_do_not_depend_on_it() {
    let root = common::repo_root();
    let spike = fs::read_to_string(root.join(SPIKE_MANIFEST)).expect("spike manifest");
    assert!(
        spike.contains("publish = false"),
        "fail-closed proof must set publish = false"
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
            !text.contains(SPIKE_PACKAGE) && !text.contains("spikes/fail-closed-proof"),
            "{rel} must not depend on the fail-closed proof spike"
        );
    }
}

#[test]
fn fail_closed_workflow_covers_both_runner_images_and_pins_actions() {
    let root = common::repo_root();
    let text = fs::read_to_string(root.join(PROOF_WORKFLOW)).expect("proof workflow");
    assert!(text.contains("windows-2022"));
    assert!(text.contains("windows-2025"));
    assert!(text.contains("spikes/fail-closed-proof"));
    assert!(text.contains("workflow_dispatch:"));
    assert!(text.contains("pull_request:"));
    assert!(text.contains("spikes/fail-closed-proof/**"));
    assert!(text.contains(".github/workflows/fail-closed-proof.yml"));
    assert!(text.contains("contents: read"));
    assert!(text.contains("if: always()"));
    assert!(text.contains(SERVICE_NAME));
    assert!(
        text.contains("--revert"),
        "cleanup must run the tool's journal revert"
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
fn fail_closed_proof_journals_every_lockdown_policy() {
    let root = common::repo_root();
    let mut files = Vec::new();
    collect_rust(&root.join("spikes/fail-closed-proof"), &mut files);
    let source = files
        .iter()
        .map(|path| {
            fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        source.contains("sync_all"),
        "the journal must be flushed before a policy write"
    );
    for policy in POLICIES {
        assert!(source.contains(policy), "spike must journal {policy}");
    }
    assert!(
        !source.contains(PRODUCT_DISPLAY_NAME),
        "spike source must not hard-code the product name"
    );
    assert!(
        !source.contains("P7Desktop") && !source.contains("P7 desktop"),
        "spike must not use the school machine name"
    );
}

#[test]
fn checklist_and_finding_cover_cleanup_and_the_dependent_tickets() {
    let root = common::repo_root();
    let checklist = fs::read_to_string(root.join(CHECKLIST)).expect("checklist");
    assert!(checklist.contains("--allow-local-service"));
    assert!(checklist.contains("--revert"));
    assert!(checklist.contains(SERVICE_NAME));
    assert!(checklist.contains("sc.exe stop"));
    assert!(checklist.contains("sc.exe delete"));
    assert!(
        checklist.to_ascii_lowercase().contains("minute"),
        "checklist must say how long the run takes"
    );
    for policy in POLICIES {
        assert!(
            checklist.contains(policy),
            "checklist cleanup must name {policy}"
        );
    }

    let finding = fs::read_to_string(root.join(FINDING)).expect("finding");
    assert!(
        finding.contains("Primary") || finding.contains("primary"),
        "finding must pick the primary OS-lock primitive"
    );
    assert!(
        finding.contains("Fall-back")
            || finding.contains("fall-back")
            || finding.contains("Fallback")
            || finding.contains("fallback"),
        "finding must pick the fall-back OS-lock primitive"
    );
    assert!(finding.contains("15"), "finding must name ticket 15");
    assert!(finding.contains("17"), "finding must name ticket 17");
    assert!(finding.contains("LockWorkStation"));
    assert!(finding.contains("WTSDisconnectSession"));
}

fn collect_rust(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name() == "target" {
                continue;
            }
            collect_rust(&path, files);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn collect_manifests(dir: &Path, manifests: &mut Vec<std::path::PathBuf>) {
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
