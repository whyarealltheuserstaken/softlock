//! Fails the build when Core depends on a Windows-only crate.

include!("windows_dep_check.rs");

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    println!("cargo:rerun-if-changed=windows_dep_check.rs");
    if let Err(error) = check_from_env() {
        panic!(
            "compile-time check failed: Core took a dependency on a Windows-only crate: {error}"
        );
    }
}
