//! The spike is built in CI and is not part of a release build.
//! The checklist and the finding stay blank until the laptop runs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use softlock_spike_keep_awake::{
    CLOSE_LID_LINE, MEASUREMENT_SCENARIOS, analyze_command, run_command, smoke_command,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

#[test]
fn spike_is_a_member_and_not_a_default_member() {
    let root = repo_root();
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
    let members = meta["workspace_members"]
        .as_array()
        .expect("workspace members");
    let defaults = meta["workspace_default_members"]
        .as_array()
        .expect("default members");
    let packages = meta["packages"].as_array().expect("packages");
    let spike = packages
        .iter()
        .find(|package| package["name"] == "softlock-spike-keep-awake")
        .expect("spike package");
    let id = spike["id"].as_str().expect("package id");
    assert!(
        members.iter().any(|member| member == id),
        "the spike must be a workspace member so CI tests it"
    );
    assert!(
        !defaults.iter().any(|member| member == id),
        "a release build must not produce the spike"
    );
    assert_eq!(spike["publish"], serde_json::json!([]));
    for package in packages {
        if package["name"] == "softlock-spike-keep-awake" {
            continue;
        }
        let deps = package["dependencies"].as_array().expect("dependencies");
        assert!(
            deps.iter()
                .all(|dep| dep["name"] != "softlock-spike-keep-awake"),
            "{} must not depend on the spike",
            package["name"]
        );
    }
}

#[test]
fn product_crates_do_not_name_the_spike() {
    let crates = repo_root().join("crates");
    let mut hits = Vec::new();
    scan(&crates, &mut hits);
    assert!(
        hits.is_empty(),
        "product crates must not name the spike: {hits:?}"
    );
}

fn scan(dir: &Path, hits: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if name == "target" || name == "node_modules" {
                continue;
            }
            scan(&path, hits);
            continue;
        }
        let text = fs::read_to_string(&path).unwrap_or_default();
        if text.contains("softlock-spike-keep-awake") {
            hits.push(path);
        }
    }
}

#[test]
fn checklist_has_one_command_per_scenario_and_the_new_columns() {
    let checklist = read(&repo_root().join("docs/findings/08-keep-awake/CHECKLIST.md"));
    assert!(
        checklist.contains(
            "cargo run -p softlock-spike-keep-awake -- run --scenario practice --minutes 3"
        )
    );
    assert!(checklist.contains("heat-guard"));
    assert!(checklist.contains("passive_limit"));
    assert!(checklist.contains("throttle_reasons"));
    assert!(checklist.contains(&analyze_command()));
    assert!(checklist.contains("spikes/keep-awake/logs/"));
    assert!(checklist.contains(CLOSE_LID_LINE));
    assert!(checklist.contains("60 minutes"));
    assert!(checklist.contains("Modern Standby (AoAc): no"));
    assert!(checklist.contains("Lowest battery %"));
    assert!(checklist.contains("Heat"));
    assert!(checklist.contains("Policy or cap"));
    assert!(checklist.contains("hard surface"));
    assert!(checklist.contains("bag"));
    assert!(checklist.contains("80%"));
    assert!(checklist.contains("20%"));
    assert!(checklist.contains("critical-battery"));
    assert!(checklist.contains("Drag"));
    assert!(checklist.contains("issue #8"));
    assert!(checklist.contains("stray key"));
    assert!(checklist.contains("Ctrl+C"));
    assert!(checklist.contains("4 hours 45 minutes"));
    assert!(checklist.contains("40 minutes"));
    for id in MEASUREMENT_SCENARIOS {
        assert!(checklist.contains(&run_command(id)), "{id}");
        assert!(
            checklist.contains(&format!("spikes/keep-awake/logs/{id}.log")),
            "{id}"
        );
    }
    assert!(
        !checklist.contains("- [x]") && !checklist.contains("- [X]"),
        "manual boxes stay unticked"
    );
}

#[test]
fn finding_leaves_results_blank_and_names_dependents() {
    let finding = read(&repo_root().join("docs/findings/08-keep-awake/FINDING.md"));
    assert!(finding.contains("(blank)"));
    assert!(finding.contains("Lowest battery %"));
    assert!(finding.contains("Heat"));
    assert!(finding.contains("Policy or cap"));
    assert!(finding.contains("#20"));
    assert!(finding.contains("#27"));
    assert!(finding.contains("Part of #8"));
    assert!(
        finding
            .contains("Default battery floor: 15% proposed (spec Decision 10); confirm from logs")
    );
    assert!(!finding.contains("- [x]") && !finding.contains("- [X]"));
    assert!(finding.contains("- [ ]"));
}

#[test]
fn findings_do_not_mark_manual_boxes_done() {
    let root = repo_root().join("docs/findings");
    let mut files = Vec::new();
    collect(&root, &mut files);
    for path in files {
        let text = read(&path);
        assert!(
            !text.contains("- [x]") && !text.contains("- [X]"),
            "{} marks a manual box done",
            path.display()
        );
    }
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("findings") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            files.push(path);
        }
    }
}

#[test]
fn windows_ci_smokes_the_spike() {
    let ci = read(&repo_root().join(".github/workflows/ci.yml"));
    assert!(ci.contains("runner.os == 'Windows'"));
    assert!(ci.contains(&smoke_command()));
}
