// Compile-time guard: softlock-core must not depend on a Windows-only crate.

use std::fs;
use std::path::{Path, PathBuf};

const PACKAGE: &str = "softlock-core";

pub fn check_from_env() -> Result<(), String> {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .map_err(|_| "CARGO_MANIFEST_DIR is not set".to_string())?,
    );
    check_crate(&manifest_dir)
}

/// Reads a crate directory the way the build script does: its manifest, then
/// a lockfile beside it or two directories up.
pub fn check_crate(dir: &Path) -> Result<(), String> {
    let manifest = fs::read_to_string(dir.join("Cargo.toml"))
        .map_err(|error| format!("read Cargo.toml: {error}"))?;
    check_manifest(&manifest)?;
    let beside = dir.join("Cargo.lock");
    let lock_path = if beside.exists() {
        beside
    } else {
        dir.join("../../Cargo.lock")
    };
    let lock = fs::read_to_string(&lock_path)
        .map_err(|error| format!("read {}: {error}", lock_path.display()))?;
    check_lock(&lock)
}

pub fn check_manifest(manifest: &str) -> Result<(), String> {
    for name in dependency_names(manifest) {
        if let Some(reason) = windows_only(&name) {
            return Err(format!("{PACKAGE} depends on {name}, {reason}"));
        }
    }
    Ok(())
}

pub fn check_lock(lock: &str) -> Result<(), String> {
    for name in direct_lock_deps(lock, PACKAGE) {
        if let Some(reason) = windows_only(&name) {
            return Err(format!("{PACKAGE} lockfile depends on {name}, {reason}"));
        }
    }
    Ok(())
}

fn windows_only(name: &str) -> Option<&'static str> {
    if name == "windows"
        || name == "winapi"
        || name == "winreg"
        || name == "ntapi"
        || name == "softlock-windows"
        || name.starts_with("windows-")
        || name.starts_with("windows_")
        || name.starts_with("winapi-")
    {
        Some("which is Windows-only")
    } else {
        None
    }
}

fn dependency_names(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_deps = false;
    for raw in manifest.lines() {
        let line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let table = line.trim_matches(['[', ']']).trim();
            let segments = table_segments(table);
            in_deps = false;
            if let Some(index) = segments.iter().position(|segment| is_dep_table(segment)) {
                if let Some(name) = segments.get(index + 1) {
                    names.push(name.clone());
                }
                in_deps = true;
            }
            continue;
        }
        if !in_deps {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().trim_matches('"');
        if key.is_empty()
            || matches!(
                key,
                "version"
                    | "path"
                    | "features"
                    | "default-features"
                    | "optional"
                    | "package"
                    | "workspace"
                    | "registry"
            )
        {
            continue;
        }
        names.push(key.to_string());
        if let Some(renamed) = renamed_package(value) {
            names.push(renamed);
        }
    }
    names
}

fn renamed_package(value: &str) -> Option<String> {
    let after = value.split("package").nth(1)?;
    let quoted = after.split('"').nth(1)?;
    if quoted.is_empty() {
        None
    } else {
        Some(quoted.to_string())
    }
}

fn direct_lock_deps(lock: &str, package: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut current = false;
    let mut in_deps = false;
    for raw in lock.lines() {
        let line = raw.trim();
        if line.starts_with("[[package]]") {
            current = false;
            in_deps = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("name = ") {
            current = rest.trim_matches('"') == package;
            continue;
        }
        if !current {
            continue;
        }
        if line.starts_with("dependencies") {
            in_deps = true;
            continue;
        }
        if in_deps && line.starts_with(']') {
            in_deps = false;
            continue;
        }
        if in_deps && let Some(dep) = quoted(line) {
            names.push(dep.split_whitespace().next().unwrap_or(dep).to_string());
        }
    }
    names
}

fn quoted(line: &str) -> Option<&str> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn is_dep_table(segment: &str) -> bool {
    segment == "dependencies" || segment == "dev-dependencies" || segment == "build-dependencies"
}

/// Splits a TOML table path on dots that are outside quotes, so
/// `target.'cfg(windows)'.dependencies.winreg` keeps `cfg(windows)` whole.
fn table_segments(table: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for ch in table.chars() {
        if let Some(mark) = quote {
            if ch == mark {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        if ch == '.' {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(index) => &line[..index],
        None => line,
    }
}
