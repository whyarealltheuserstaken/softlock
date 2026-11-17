//! The lock kit stays out of the product workspace. CI may resolve the symbol.
//! It must not call it, and the workflow must not exercise a lock.

mod common;

use std::fs;
use std::process::Command;

use softlock_core::PRODUCT_DISPLAY_NAME;

const SPIKE_MANIFEST: &str = "spikes/macos-lock-kit/Cargo.toml";
const SPIKE_PACKAGE: &str = "macos-lock-kit";
const WORKFLOW: &str = ".github/workflows/macos-lock-kit.yml";

const NEEDLES: &[&str] = &[
    "IOPlatformSerialNumber",
    "IOPlatformUUID",
    "Serial Number",
    "hw.serialnumber",
];

const CHOOSER_ROWS: &[&str] = &[
    "fn symbol_missing_requests_the_documented_path",
    "fn a_failed_call_requests_the_documented_path",
    "fn a_successful_private_call_does_not_request_the_documented_path",
    "fn documented_path_unavailable_or_password_not_required_refuses_to_arm",
];

#[test]
fn macos_lock_kit_is_excluded_from_the_product_workspace() {
    let root = common::repo_root();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
    assert!(
        manifest.contains("spikes/macos-lock-kit"),
        "workspace exclude list must name the lock kit"
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
            "lock kit must not be a workspace member"
        );
        let manifest_path = package["manifest_path"].as_str().unwrap_or("");
        assert!(
            !manifest_path
                .replace('\\', "/")
                .contains("spikes/macos-lock-kit"),
            "{name} resolves inside the lock kit"
        );
    }
}

#[test]
fn product_crates_do_not_depend_on_the_lock_kit() {
    let root = common::repo_root();
    let spike = fs::read_to_string(root.join(SPIKE_MANIFEST)).expect("spike manifest");
    assert!(spike.contains("publish = false"));
    assert!(!spike.contains(PRODUCT_DISPLAY_NAME));
    assert!(spike.contains("softlock-macos"));

    let mut manifests = Vec::new();
    collect_manifests(&root.join("crates"), &mut manifests);
    assert!(!manifests.is_empty());
    for path in manifests {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let rel = relative(&root, &path);
        assert!(
            !text.contains(SPIKE_PACKAGE) && !text.contains("spikes/macos-lock-kit"),
            "{rel} must not depend on the lock kit"
        );
    }
}

#[test]
fn the_chooser_rows_do_not_call_the_live_symbol() {
    let root = common::repo_root();
    let chooser =
        fs::read_to_string(root.join("crates/softlock-macos/src/lock_choice.rs")).expect("chooser");
    for row in CHOOSER_ROWS {
        assert!(chooser.contains(row), "missing {row}");
    }
    assert!(chooser.contains("SACLockScreenImmediate"));
    assert!(chooser.contains("login.framework"));
    assert!(!chooser.contains("dlopen("));
    assert!(!chooser.contains("dlsym("));
    assert!(!chooser.contains("Command::new"));
    assert!(!chooser.contains(PRODUCT_DISPLAY_NAME));
}

#[test]
fn the_read_only_workflow_does_not_lock() {
    let root = common::repo_root();
    let text = fs::read_to_string(root.join(WORKFLOW)).expect("workflow");
    assert!(text.contains("macos-latest"));
    assert!(text.contains("sw_vers"));
    assert!(text.contains("uname -m"));
    assert!(text.contains("--read-only"));
    assert!(text.contains("private-call=not-called"));
    assert!(text.contains("state=never-unlocked"));
    assert!(text.contains("serial-collected=false"));
    assert!(text.contains("runner-lock-allowed=false"));
    assert!(text.contains("filevault-pre-boot-observed=false"));
    assert!(text.contains("contents: read"));
    assert!(text.contains("workflow_dispatch:"));
    assert!(text.contains("pull_request:"));
    assert!(!text.contains("--exercise"));
    assert!(!text.contains("--i-will-unlock"));
    assert!(!text.contains(PRODUCT_DISPLAY_NAME));
    let trigger = text.split("permissions:").next().expect("permissions");
    assert!(
        !trigger
            .lines()
            .any(|line| line.trim() == "push:" || line.trim().starts_with("push:")),
        "lock kit workflow must not run on every push"
    );
    let mut uses = 0;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("- uses:") else {
            continue;
        };
        uses += 1;
        let reference = rest.split('#').next().unwrap_or(rest).trim();
        let sha = reference.rsplit('@').next().expect("action pin");
        assert_eq!(sha.len(), 40, "action pin must be a full SHA: {line}");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }
    assert!(uses >= 3);
}

#[test]
fn serial_markers_are_only_needles_and_the_kit_does_not_print_the_product_name() {
    let root = common::repo_root();
    let mut files = Vec::new();
    collect_text(&root.join("spikes/macos-lock-kit"), &mut files);
    assert!(files.len() > 4);
    for path in &files {
        let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let rel = relative(&root, path);
        assert!(!text.contains(PRODUCT_DISPLAY_NAME), "{rel}");
        if rel.ends_with("src/redact.rs") {
            for needle in NEEDLES {
                assert!(text.contains(needle), "{rel} must redact {needle}");
            }
            continue;
        }
        for needle in NEEDLES {
            assert!(!text.contains(needle), "{rel} contains {needle}");
        }
    }
    let workflow = fs::read_to_string(root.join(WORKFLOW)).expect("workflow");
    for needle in NEEDLES {
        assert!(
            workflow.contains(needle),
            "workflow must reject printed {needle}"
        );
    }
}

fn relative(root: &std::path::Path, path: &std::path::Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
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
