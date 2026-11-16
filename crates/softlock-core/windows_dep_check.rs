// Compile-time guard: softlock-core must not depend on a Windows-only crate.

use std::fs;
use std::path::PathBuf;

const PACKAGE: &str = "softlock-core";

pub fn check_from_env() -> Result<(), String> {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .map_err(|_| "CARGO_MANIFEST_DIR is not set".to_string())?,
    );
    let manifest = fs::read_to_string(manifest_dir.join("Cargo.toml"))
        .map_err(|error| format!("read Cargo.toml: {error}"))?;
    check_manifest(&manifest)?;
    let lock_path = manifest_dir.join("../../Cargo.lock");
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
            in_deps = table.split('.').any(|part| part.contains("dependencies"));
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

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(index) => &line[..index],
        None => line,
    }
}
