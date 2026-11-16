//! The compile-time check fails the build when Core depends on a Windows-only crate.

#[path = "../windows_dep_check.rs"]
mod windows_dep_check;

use std::fs;
use std::path::PathBuf;

use windows_dep_check::{check_from_env, check_lock, check_manifest};

#[test]
fn core_windows_only_dependency_fails_the_build() {
    let windows = check_manifest("[dependencies]\nwindows = \"0.61\"\n").unwrap_err();
    assert!(windows.contains("windows"), "{windows}");

    let windows_sys =
        check_manifest("[target.'cfg(windows)'.dependencies]\nwindows-sys = \"0.61\"\n")
            .unwrap_err();
    assert!(windows_sys.contains("windows-sys"), "{windows_sys}");

    let renamed =
        check_manifest("[dependencies]\nserde = { package = \"windows\", version = \"1\" }\n")
            .unwrap_err();
    assert!(renamed.contains("windows"), "{renamed}");

    check_manifest("[dependencies]\nserde = \"1\"\n").expect("serde is portable");
    check_manifest("[dev-dependencies]\n# windows = \"0.61\"\nserde = \"1\"\n")
        .expect("comments are not dependencies");

    check_from_env().expect("the crate that is building stays portable");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(manifest_dir.join("Cargo.toml")).expect("manifest");
    check_manifest(&manifest).expect("the real manifest stays portable");

    let lock = fs::read_to_string(manifest_dir.join("../../Cargo.lock")).expect("lockfile");
    check_lock(&lock).expect("the real lockfile stays portable");

    let bad_lock = r#"
[[package]]
name = "softlock-core"
version = "0.1.0"
dependencies = [
 "windows-sys 0.61.2",
]
"#;
    assert!(check_lock(bad_lock).is_err());

    let build_rs = fs::read_to_string(manifest_dir.join("build.rs")).expect("build.rs");
    assert!(
        build_rs.contains("check_from_env"),
        "build.rs must run the compile-time check"
    );
    assert!(
        build_rs.contains("Windows-only"),
        "the build failure must name the Windows-only dependency"
    );
}
