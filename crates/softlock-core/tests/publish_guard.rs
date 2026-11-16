//! Workspace crates set `publish = false`. The npm package is private.

mod common;

use std::fs;
use std::process::Command;

#[test]
fn workspace_crates_are_not_published() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(common::repo_root())
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let meta: serde_json::Value = serde_json::from_slice(&output.stdout).expect("metadata json");
    let packages = meta["packages"].as_array().expect("workspace packages");
    assert!(!packages.is_empty());
    for package in packages {
        let name = package["name"].as_str().expect("package name");
        assert_eq!(
            package["publish"],
            serde_json::json!([]),
            "{name} must set publish = false"
        );
        assert!(
            !name.eq_ignore_ascii_case("softlock"),
            "{name} must not be published under softlock"
        );
    }
}

#[test]
fn settings_app_bundle_id_is_the_product_id() {
    let path = common::repo_root().join("crates/softlock-app/src-tauri/tauri.conf.json");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("missing {}: {error}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&text).expect("tauri config");
    assert_eq!(softlock_core::APP_BUNDLE_ID, "com.softlock.app");
    assert_eq!(value["productName"], "SoftLock");
    assert_eq!(value["identifier"], "com.softlock.app");
}

#[test]
fn settings_app_package_is_private_and_named_for_the_product() {
    let path = common::repo_root().join("crates/softlock-app/package.json");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("missing {}: {error}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&text).expect("package.json");
    assert_eq!(softlock_core::APP_PACKAGE_NAME, "softlock");
    assert_eq!(value["private"], true);
    assert_eq!(value["name"], "softlock");
}
