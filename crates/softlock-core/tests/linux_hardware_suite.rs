//! The Linux hardware suite stays out of the product workspace.

mod common;

use std::fs;
use std::process::Command;

use softlock_core::PRODUCT_DISPLAY_NAME;

const MANIFEST: &str = "tests/hardware/linux/Cargo.toml";
const PACKAGE: &str = "linux-hardware-suite";
const WORKFLOW: &str = ".github/workflows/linux-hardware-suite.yml";
const RUNNER: &str = "tests/hardware/linux/run.sh";

const BANNED: &[&str] = &[
    "/sys/class/dmi/id/product_serial",
    "/sys/class/dmi/id/board_serial",
    "/sys/class/dmi/id/chassis_serial",
    "EVIOCGUNIQ",
    "unlock-session",
];

#[test]
fn linux_hardware_suite_is_excluded_from_the_product_workspace() {
    let root = common::repo_root();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
    assert!(
        manifest.contains("tests/hardware/linux"),
        "workspace exclude list must name the Linux hardware suite"
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
            name, PACKAGE,
            "linux hardware suite must not be a workspace member"
        );
        let manifest_path = package["manifest_path"].as_str().unwrap_or("");
        assert!(
            !manifest_path
                .replace('\\', "/")
                .contains("tests/hardware/linux"),
            "{name} resolves inside the linux hardware suite"
        );
    }
}

#[test]
fn linux_hardware_suite_is_unpublished_and_product_crates_do_not_depend_on_it() {
    let root = common::repo_root();
    let spike = fs::read_to_string(root.join(MANIFEST)).expect("suite manifest");
    assert!(spike.contains("publish = false"));
    assert!(!spike.contains(PRODUCT_DISPLAY_NAME));

    let mut manifests = Vec::new();
    collect_manifests(&root.join("crates"), &mut manifests);
    assert!(!manifests.is_empty());
    for path in manifests {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let rel = relative(&root, &path);
        assert!(
            !text.contains(PACKAGE) && !text.contains("tests/hardware/linux"),
            "{rel} must not depend on the linux hardware suite"
        );
    }
}

#[test]
fn the_ci_workflow_runs_ci_mode_on_ubuntu_and_pins_actions() {
    let root = common::repo_root();
    let text = fs::read_to_string(root.join(WORKFLOW)).expect("workflow");
    assert!(text.contains("ubuntu-latest"));
    assert!(text.contains("--ci"));
    assert!(text.contains(RUNNER) || text.contains("tests/hardware/linux/run.sh"));
    assert!(text.contains("softlock-hardware-suite/1"));
    assert!(text.contains("never-unlocked"));
    assert!(text.contains("serial-collected"));
    assert!(text.contains("contents: read"));
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
fn suite_sources_do_not_record_serials_or_unlock() {
    let root = common::repo_root();
    let mut files = Vec::new();
    collect_text(&root.join("tests/hardware/linux"), &mut files);
    assert!(files.len() > 4);
    for path in &files {
        let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let rel = relative(&root, path);
        assert!(!text.contains(PRODUCT_DISPLAY_NAME), "{rel}");
        let allow_needles = rel.ends_with("src/redact.rs")
            || rel.ends_with("src/fail_closed.rs")
            || rel.ends_with("src/restore.rs")
            || rel.ends_with("linux_hardware_suite.rs")
            || rel.contains("tests/parse.rs");
        if allow_needles {
            continue;
        }
        for needle in BANNED {
            assert!(!text.contains(needle), "{rel} contains {needle}");
        }
    }
}

#[test]
fn run_sh_builds_locked_release() {
    let root = common::repo_root();
    let text = fs::read_to_string(root.join(RUNNER)).expect("run.sh");
    assert!(text.contains("--release"));
    assert!(text.contains("--locked"));
    assert!(text.contains("cargo test"));
    assert!(text.contains("cargo run"));
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
            if entry.file_name() == "target" || entry.file_name() == "suite-output" {
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
            || name.ends_with(".sh")
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
