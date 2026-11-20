fn main() {
    println!("cargo:rerun-if-changed=src/mac/probe.m");
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target != "macos" {
        return;
    }
    cc::Build::new()
        .file("src/mac/probe.m")
        .flag("-x")
        .flag("objective-c")
        .flag("-fobjc-arc")
        .flag("-Wno-deprecated-declarations")
        .compile("macosphysicalinput");
    for framework in [
        "AppKit",
        "ApplicationServices",
        "CoreFoundation",
        "CoreGraphics",
        "Foundation",
        "IOKit",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
