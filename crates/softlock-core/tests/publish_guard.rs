//! Workspace crates set `publish = false`. The settings app is private.

mod common;

use std::process::Command;

const CONTENT_SECURITY_POLICY: &str =
    "default-src 'self'; script-src 'self'; connect-src ipc: http://ipc.localhost";

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
    let value = common::read_json("crates/softlock-app/src-tauri/tauri.conf.json");
    assert_eq!(softlock_core::APP_BUNDLE_ID, "com.softlock.app");
    assert_eq!(value["productName"], softlock_core::PRODUCT_DISPLAY_NAME);
    assert_eq!(value["identifier"], softlock_core::APP_BUNDLE_ID);
    assert!(value["app"]["windows"][0].get("title").is_none());
    assert_eq!(value["app"]["security"]["csp"], CONTENT_SECURITY_POLICY);
}

#[test]
fn settings_app_package_is_private_and_named_for_the_product() {
    let value = common::read_json("crates/softlock-app/package.json");
    assert_eq!(softlock_core::APP_PACKAGE_NAME, "softlock");
    assert_eq!(value["private"], true);
    assert_eq!(value["name"], softlock_core::APP_PACKAGE_NAME);
}

#[test]
fn dev_server_port_is_shared_with_tauri() {
    let dev = common::read_json("crates/softlock-app/dev-server.json");
    let host = dev["host"].as_str().expect("dev server host");
    let port = dev["port"].as_u64().expect("dev server port");
    let tauri = common::read_json("crates/softlock-app/src-tauri/tauri.conf.json");
    assert_eq!(tauri["build"]["devUrl"], format!("http://{host}:{port}"));
}
