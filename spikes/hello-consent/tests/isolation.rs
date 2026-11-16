//! The spike stays out of product crates, and the manual pages stay blank.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn spike_is_outside_the_product_dependency_graph() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(repo_root())
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let meta: serde_json::Value = serde_json::from_slice(&output.stdout).expect("metadata json");
    let packages = meta["packages"].as_array().expect("packages");
    let spike = packages
        .iter()
        .find(|package| package["name"] == "softlock-hello-spike")
        .expect("spike package");
    let manifest = spike["manifest_path"].as_str().expect("manifest path");
    assert!(
        manifest
            .replace('\\', "/")
            .contains("/spikes/hello-consent/"),
        "spike manifest should live under spikes/: {manifest}"
    );

    let spike_id = spike["id"].as_str().expect("spike id");
    let defaults = meta["workspace_default_members"]
        .as_array()
        .expect("default members");
    assert!(
        !defaults.iter().any(|id| id == spike_id),
        "a plain release build must not include the spike"
    );

    for package in packages {
        let name = package["name"].as_str().expect("name");
        if name == "softlock-hello-spike" {
            continue;
        }
        let deps = package["dependencies"].as_array().expect("dependencies");
        assert!(
            !deps.iter().any(|dep| dep["name"] == "softlock-hello-spike"),
            "{name} must not depend on the spike"
        );
    }
}

#[test]
fn checklist_leads_with_the_block_warning() {
    let text = read("docs/findings/09-hello/CHECKLIST.md");
    let head: String = text.lines().take(16).collect::<Vec<_>>().join("\n");
    assert!(
        head.contains("blocks your keyboard and mouse"),
        "the block warning has to be the first thing he reads"
    );
    assert!(head.contains("3 minutes"));
    assert!(head.contains("Ctrl") && head.contains("Alt") && head.contains("Shift"));
    assert!(head.contains("Q"));
    assert!(
        head.contains("crashes"),
        "the checklist has to say the hook comes off if the tool crashes"
    );
    assert!(
        !text.to_ascii_lowercase().contains("- [x]"),
        "manual boxes stay empty"
    );
}

#[test]
fn finding_leaves_the_decision_blank() {
    let text = read("docs/findings/09-hello/FINDING.md");
    assert!(text.contains("(blank)"));
    assert!(text.contains("21"));
    assert!(text.contains("22"));
    assert!(text.contains("- [ ] Go."));
    assert!(text.contains("- [ ] Fallback."));
    assert!(!text.to_ascii_lowercase().contains("- [x]"));
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}
