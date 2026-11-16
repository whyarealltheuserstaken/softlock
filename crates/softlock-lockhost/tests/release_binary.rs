//! The release-binary check, and the contaminated build that proves it fails.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MARKER_STRING: &[u8] = b"softlock-simulated-physical-marker";
const MARKER_SYMBOL: &[u8] = b"softlock_simulated_physical_marker";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Hit {
    path: String,
    string: bool,
    symbol: bool,
}

#[test]
fn scanner_rejects_each_marker_on_its_own() {
    let root = unique_dir("markers");
    let string_only = root.join("string-only.rlib");
    let symbol_only = root.join("symbol-only.exe");
    let clean = root.join("clean.dll");
    fs::write(&string_only, padding(MARKER_STRING)).unwrap();
    fs::write(&symbol_only, padding(MARKER_SYMBOL)).unwrap();
    fs::write(&clean, b"release binary without the test feature").unwrap();

    let string_hits = scan(&root, "string-only.rlib");
    assert!(string_hits.iter().any(|hit| hit.string && !hit.symbol));
    let symbol_hits = scan(&root, "symbol-only.exe");
    assert!(symbol_hits.iter().any(|hit| hit.symbol && !hit.string));
    assert!(
        scan(&root, "clean.dll").is_empty(),
        "a clean file must not trip the check"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn contaminated_build_fails_the_release_binary_check() {
    let root = unique_dir("builds");
    let clean_dir = root.join("clean");
    let dirty_dir = root.join("dirty");
    build(&clean_dir, false).expect("release build without the test feature");
    let clean_hits = scan(&clean_dir.join("release"), "");
    assert!(
        clean_hits.is_empty(),
        "release build without the feature must not contain markers:\n{}",
        format_hits(&clean_hits)
    );

    build(&dirty_dir, true).expect("release build with the test feature");
    let dirty_hits = scan(&dirty_dir.join("release"), "");
    assert!(
        dirty_hits.iter().any(|hit| hit.string),
        "contaminated build must contain the marker string:\n{}",
        format_hits(&dirty_hits)
    );
    assert!(
        dirty_hits.iter().any(|hit| hit.symbol),
        "contaminated build must contain the marker symbol:\n{}",
        format_hits(&dirty_hits)
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn product_release_binaries_omit_simulated_physical_markers() {
    let Ok(dir) = std::env::var("SOFTLOCK_RELEASE_SCAN_DIR") else {
        println!("product release scan skipped");
        return;
    };
    let hits = scan(Path::new(&dir), "");
    let count = count_binaries(Path::new(&dir));
    assert!(
        count > 0,
        "no release binaries under {dir}; the check would not be looking at a build"
    );
    assert!(
        hits.is_empty(),
        "simulated-physical markers in release binaries:\n{}",
        format_hits(&hits)
    );
    println!("scanned release binaries: {count}");
}

fn build(target_dir: &Path, simulated_physical: bool) -> Result<(), String> {
    let manifest = repo_root().join("Cargo.toml");
    let mut command = Command::new(env!("CARGO"));
    command
        .arg("build")
        .arg("--release")
        .arg("--locked")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("-p")
        .arg("softlock-lockhost")
        .arg("--target-dir")
        .arg(target_dir)
        .env("CARGO_TARGET_DIR", target_dir)
        .current_dir(repo_root());
    if simulated_physical {
        command
            .arg("--features")
            .arg("softlock-lockhost/simulated-physical");
    }
    let output = command
        .output()
        .map_err(|error| format!("start cargo: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "cargo build exited {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn scan(root: &Path, name_filter: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for path in binaries(root) {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if !name_filter.is_empty() && name != name_filter {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let string = contains(&bytes, MARKER_STRING);
        let symbol = contains(&bytes, MARKER_SYMBOL);
        if string || symbol {
            hits.push(Hit {
                path: path.display().to_string(),
                string,
                symbol,
            });
        }
    }
    hits
}

fn count_binaries(root: &Path) -> usize {
    binaries(root).len()
}

fn binaries(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(root, &mut found);
    found
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "build" || name == ".fingerprint" || name == "incremental" {
                continue;
            }
            walk(&path, found);
            continue;
        }
        if is_binary_name(&name) {
            found.push(path);
        }
    }
}

fn is_binary_name(name: &str) -> bool {
    name.ends_with(".rlib")
        || name.ends_with(".exe")
        || name.ends_with(".dll")
        || name.ends_with(".so")
        || name.ends_with(".dylib")
        || name.ends_with(".bin")
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn padding(needle: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; 32];
    bytes.extend_from_slice(needle);
    bytes.extend_from_slice(&[0u8; 32]);
    bytes
}

fn format_hits(hits: &[Hit]) -> String {
    if hits.is_empty() {
        return "(no hits)".to_string();
    }
    hits.iter()
        .map(|hit| format!("{} string={} symbol={}", hit.path, hit.string, hit.symbol))
        .collect::<Vec<_>>()
        .join("\n")
}

fn unique_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "softlock-release-check-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
