//! Seam 1. The product name stays in the name constants and the string resources.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use softlock_core::PRODUCT_DISPLAY_NAME;

const NAME_CONSTANTS: &str = "crates/softlock-core/src/name.rs";
const SETTINGS_APP_CONFIG: &str = "crates/softlock-app/src-tauri/tauri.conf.json";
const INDEX_HTML: &str = "crates/softlock-app/index.html";
const STRING_RESOURCES: &str = "crates/softlock-app/src/strings/";

/// These files quote required spec sentences and the locked service-id spellings.
const SPEC_LITERAL_FILES: &[&str] = &[
    "crates/softlock-core/tests/doc_lint.rs",
    "crates/softlock-core/tests/name_guard.rs",
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
    assert_eq!(PRODUCT_DISPLAY_NAME, "SoftLock");
}

#[test]
fn display_name_appears_only_in_the_name_constants_and_string_resources() {
    let root = common::repo_root();
    let mut files = Vec::new();
    collect_source_files(&root, &mut files);

    let mut violations = Vec::new();
    for path in files {
        let rel = relative_slash(&root, &path);
        if allowed_file(&rel) {
            continue;
        }
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let visible = visible_text(&rel, &text);
        if !visible.contains(PRODUCT_DISPLAY_NAME) {
            continue;
        }
        for (index, line) in visible.lines().enumerate() {
            if line.contains(PRODUCT_DISPLAY_NAME) {
                violations.push(format!("{rel}:{}", index + 1));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "display name found outside the name constants and string resources:\n{}",
        violations.join("\n")
    );
}

#[test]
fn service_pipe_and_registry_share_one_internal_id() {
    assert_eq!(softlock_core::SERVICE_DISPLAY_NAME, "SoftLock Agent");
    assert_eq!(softlock_core::SERVICE_ID, "softlock-agent");
    assert_eq!(softlock_core::SERVICE_NAME, "SoftLockAgent");
    assert_eq!(softlock_core::pipe_name(), r"\\.\pipe\softlock-agent");
    assert_eq!(softlock_core::registry_key(), r"Software\SoftLockAgent");
}

#[test]
fn string_resource_and_document_title_match_the_product_name() {
    let product = common::read_json("crates/softlock-app/src/strings/product-name.json");
    assert_eq!(product["productName"], PRODUCT_DISPLAY_NAME);

    let html = fs::read_to_string(common::repo_root().join(INDEX_HTML)).expect("index.html");
    assert_eq!(document_title(&html), Some(PRODUCT_DISPLAY_NAME));
}

fn allowed_file(rel: &str) -> bool {
    rel == NAME_CONSTANTS
        || SPEC_LITERAL_FILES.contains(&rel)
        || (rel.starts_with(STRING_RESOURCES) && rel.ends_with(".json"))
}

/// Text that still counts as a handwritten copy of the product name.
fn visible_text(rel: &str, text: &str) -> String {
    if rel == INDEX_HTML {
        return text.replace(&title_element(PRODUCT_DISPLAY_NAME), "");
    }
    if rel == SETTINGS_APP_CONFIG {
        return text.replace(&format!("\"productName\": \"{PRODUCT_DISPLAY_NAME}\""), "");
    }
    text.to_string()
}

fn document_title(html: &str) -> Option<&str> {
    let start = html.find("<title>")? + "<title>".len();
    let rest = html.get(start..)?;
    let end = rest.find("</title>")?;
    Some(rest[..end].trim())
}

fn title_element(name: &str) -> String {
    format!("<title>{name}</title>")
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
