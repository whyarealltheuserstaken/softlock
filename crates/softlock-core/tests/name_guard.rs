//! Seam 1. The display name stays in one constant and the string resources.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

const DISPLAY_NAME: &str = "SoftLock";
const CONSTANT_REL: &str = "crates/softlock-core/src/name.rs";
const CONSTANT_LINE: &str = "pub const PRODUCT_DISPLAY_NAME: &str = \"SoftLock\";";
const STRING_RESOURCES: &str = "crates/softlock-app/src/strings/";

/// These files quote the spec's required literals. They are not a general exemption.
const SPEC_LITERAL_FILES: &[&str] = &[
    "crates/softlock-core/tests/name_guard.rs",
    "crates/softlock-core/tests/doc_lint.rs",
    "crates/softlock-app/e2e/smoke.spec.ts",
];

const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "svelte", "ts", "tsx", "js", "mjs", "cjs", "json", "toml", "html", "css", "yml", "yaml",
];

const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "dist-ssr",
    "test-results",
    "playwright-report",
    "blob-report",
    "docs",
    "gen",
];

const SKIP_FILES: &[&str] = &["package-lock.json", "Cargo.lock"];

#[test]
fn display_name_constant_matches_the_working_name() {
    assert_eq!(softlock_core::PRODUCT_DISPLAY_NAME, "SoftLock");
}

#[test]
fn display_name_appears_only_in_the_constant_and_string_resources() {
    let root = common::repo_root();
    let mut files = Vec::new();
    collect_source_files(&root, &mut files);

    let mut constant_hits = 0;
    let mut violations = Vec::new();

    for path in files {
        let rel = relative_slash(&root, &path);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        if !text.contains(DISPLAY_NAME) {
            continue;
        }
        for (index, line) in text.lines().enumerate() {
            if !line.contains(DISPLAY_NAME) {
                continue;
            }
            let trimmed = line.trim();
            if rel == CONSTANT_REL && trimmed == CONSTANT_LINE {
                constant_hits += 1;
                continue;
            }
            if allowed_occurrence(&rel) {
                continue;
            }
            violations.push(format!("{rel}:{}", index + 1));
        }
    }

    assert_eq!(
        constant_hits, 1,
        "expected exactly one display-name constant"
    );
    assert!(
        violations.is_empty(),
        "display name found outside the constant and string resources:\n{}",
        violations.join("\n")
    );
}

#[test]
fn service_pipe_and_registry_share_one_internal_id() {
    assert_eq!(softlock_core::service_name(), "deskguard");
    assert_eq!(softlock_core::pipe_name(), r"\\.\pipe\deskguard");
    assert_eq!(softlock_core::registry_key(), r"Software\deskguard");
}

#[test]
fn string_resource_matches_the_working_name() {
    let path = common::repo_root().join("crates/softlock-app/src/strings/product-name.json");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("missing string resource {}: {error}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&text).expect("string resource is json");
    assert_eq!(value["productName"], "SoftLock");
}

fn allowed_occurrence(rel: &str) -> bool {
    if rel.starts_with(STRING_RESOURCES) && rel.ends_with(".json") {
        return true;
    }
    SPEC_LITERAL_FILES.contains(&rel)
}

fn collect_source_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            collect_source_files(&path, files);
            continue;
        }
        if SKIP_FILES.contains(&name.as_ref()) {
            continue;
        }
        let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if SOURCE_EXTENSIONS.contains(&extension) {
            files.push(path);
        }
    }
}

fn relative_slash(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("source file is inside the repository")
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
