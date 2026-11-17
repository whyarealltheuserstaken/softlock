//! The release-binary check, and the contaminated build that proves it fails.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MARKER_STRING: &[u8] = b"softlock-simulated-physical-marker";
const MARKER_SYMBOL: &[u8] = b"softlock_simulated_physical_marker";
const DEV_STRING: &[u8] = b"softlock-dev-launch-marker";
const DEV_SYMBOL: &[u8] = b"softlock_dev_launch_marker";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Hit {
    path: String,
    string: bool,
    symbol: bool,
    dev_string: bool,
    dev_symbol: bool,
}

#[test]
fn scanner_rejects_each_marker_on_its_own() {
    let root = unique_dir("markers");
    let string_only = root.join("string-only.rlib");
    let symbol_only = root.join("symbol-only.exe");
    let clean = root.join("clean.dll");
    let dev_string_only = root.join("dev-string.dll");
    let dev_symbol_only = root.join("dev-symbol.exe");
    fs::write(&string_only, padding(MARKER_STRING)).unwrap();
    fs::write(&symbol_only, padding(MARKER_SYMBOL)).unwrap();
    fs::write(&dev_string_only, padding(DEV_STRING)).unwrap();
    fs::write(&dev_symbol_only, padding(DEV_SYMBOL)).unwrap();
    fs::write(&clean, b"release binary without the test feature").unwrap();

    let string_hits = scan(&root, "string-only.rlib");
    assert!(
        string_hits
            .iter()
            .any(|hit| hit.string && !hit.symbol && !hit.dev_string)
    );
    let symbol_hits = scan(&root, "symbol-only.exe");
    assert!(
        symbol_hits
            .iter()
            .any(|hit| hit.symbol && !hit.string && !hit.dev_symbol)
    );
    let dev_string_hits = scan(&root, "dev-string.dll");
    assert!(
        dev_string_hits
            .iter()
            .any(|hit| hit.dev_string && !hit.dev_symbol && !hit.string)
    );
    let dev_symbol_hits = scan(&root, "dev-symbol.exe");
    assert!(
        dev_symbol_hits
            .iter()
            .any(|hit| hit.dev_symbol && !hit.dev_string && !hit.symbol)
    );
    assert!(
        scan(&root, "clean.dll").is_empty(),
        "a clean file must not trip the check"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn extensionless_executable_fails_the_release_binary_check() {
    let root = unique_dir("extensionless");
    let app = root.join("softlock-app");
    let pdb = root.join("softlock-app.pdb");
    let import_lib = root.join("softlock_app_lib.lib");
    let archive = root.join("libsoftlock_app_lib.a");
    let odd_executable = root.join("wrapped.dat");
    let notes = root.join("notes.txt");
    fs::write(&app, padding(MARKER_STRING)).unwrap();
    fs::write(&pdb, padding(MARKER_SYMBOL)).unwrap();
    fs::write(&import_lib, padding(MARKER_STRING)).unwrap();
    fs::write(&archive, padding(MARKER_SYMBOL)).unwrap();
    fs::write(&odd_executable, padding(MARKER_STRING)).unwrap();
    fs::write(&notes, padding(MARKER_STRING)).unwrap();
    make_executable(&app);
    make_executable(&odd_executable);

    let hits = scan(&root, "");
    assert!(
        hits.iter()
            .any(|hit| hit.path.ends_with("softlock-app") && hit.string),
        "an extensionless executable must fail the scan:\n{}",
        format_hits(&hits)
    );
    assert!(
        hits.iter()
            .any(|hit| hit.path.ends_with(".pdb") && hit.symbol)
    );
    assert!(
        hits.iter()
            .any(|hit| hit.path.ends_with(".lib") && hit.string)
    );
    assert!(
        hits.iter()
            .any(|hit| hit.path.ends_with(".a") && hit.symbol)
    );
    #[cfg(unix)]
    assert!(
        hits.iter()
            .any(|hit| hit.path.ends_with("wrapped.dat") && hit.string),
        "a file with the executable bit must fail the scan:\n{}",
        format_hits(&hits)
    );
    assert!(
        hits.iter().all(|hit| !hit.path.ends_with("notes.txt")),
        "a non-executable text file must not be scanned"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn contaminated_build_fails_the_release_binary_check() {
    let root = unique_dir("builds");
    let clean_dir = root.join("clean");
    let dirty_dir = root.join("dirty");
    build(&clean_dir, "softlock-lockhost", None).expect("release build without the test feature");
    let clean_hits = scan(&clean_dir.join("release"), "");
    assert!(
        clean_hits.is_empty(),
        "release build without the feature must not contain markers:\n{}",
        format_hits(&clean_hits)
    );

    build(
        &dirty_dir,
        "softlock-lockhost",
        Some("softlock-lockhost/simulated-physical,softlock-lockhost/dev-launch"),
    )
    .expect("release build with the test features");
    let dirty_hits = scan(&dirty_dir.join("release"), "");
    assert!(
        dirty_hits.iter().any(|hit| hit.string),
        "contaminated build must contain the simulated-physical marker string:\n{}",
        format_hits(&dirty_hits)
    );
    assert!(
        dirty_hits.iter().any(|hit| hit.symbol),
        "contaminated build must contain the simulated-physical marker symbol:\n{}",
        format_hits(&dirty_hits)
    );
    assert!(
        dirty_hits.iter().any(|hit| hit.dev_string),
        "contaminated build must contain the dev-launch marker string:\n{}",
        format_hits(&dirty_hits)
    );
    assert!(
        dirty_hits.iter().any(|hit| hit.dev_symbol),
        "contaminated build must contain the dev-launch marker symbol:\n{}",
        format_hits(&dirty_hits)
    );
    let service_dir = root.join("service");
    build(&service_dir, "softlock-service", Some("simulated-physical"))
        .expect("service release build with the test feature");
    let service_hits = scan(&service_dir.join("release"), "");
    assert!(
        service_hits.iter().any(|hit| hit.string && hit.symbol),
        "a service binary with the test feature must fail the scan:\n{}",
        format_hits(&service_hits)
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
        "test-only markers in release binaries:\n{}",
        format_hits(&hits)
    );
    println!("scanned release binaries: {count}");
}

fn build(target_dir: &Path, package: &str, features: Option<&str>) -> Result<(), String> {
    let manifest = repo_root().join("Cargo.toml");
    let mut command = Command::new(env!("CARGO"));
    command
        .arg("build")
        .arg("--release")
        .arg("--locked")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("-p")
        .arg(package)
        .arg("--target-dir")
        .arg(target_dir)
        .env("CARGO_TARGET_DIR", target_dir)
        .current_dir(repo_root());
    if let Some(features) = features {
        command.arg("--features").arg(features);
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
        let dev_string = contains(&bytes, DEV_STRING);
        let dev_symbol = contains(&bytes, DEV_SYMBOL);
        if string || symbol || dev_string || dev_symbol {
            hits.push(Hit {
                path: path.display().to_string(),
                string,
                symbol,
                dev_string,
                dev_symbol,
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
            // `build/` is build-script scratch. Every product crate is named
            // with `-p` on the release build, so the artifacts under test are
            // the package outputs in the release directory, not that scratch.
            if name == "build" || name == ".fingerprint" || name == "incremental" {
                continue;
            }
            walk(&path, found);
            continue;
        }
        if is_release_artifact(&path, &name) {
            found.push(path);
        }
    }
}

fn is_release_artifact(path: &Path, name: &str) -> bool {
    is_binary_name(name) || is_extensionless(name) || is_executable(path)
}

fn is_extensionless(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains('.')
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

fn is_binary_name(name: &str) -> bool {
    name.ends_with(".rlib")
        || name.ends_with(".exe")
        || name.ends_with(".dll")
        || name.ends_with(".so")
        || name.ends_with(".dylib")
        || name.ends_with(".bin")
        || name.ends_with(".pdb")
        || name.ends_with(".lib")
        || name.ends_with(".a")
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
        .map(|hit| {
            format!(
                "{} string={} symbol={} dev_string={} dev_symbol={}",
                hit.path, hit.string, hit.symbol, hit.dev_string, hit.dev_symbol
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
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
