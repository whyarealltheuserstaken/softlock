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
        .flag("-Wno-unguarded-availability")
        .compile("macosprobe");
    for framework in [
        "AppKit",
        "ApplicationServices",
        "CoreFoundation",
        "CoreGraphics",
        "Foundation",
        "ScreenCaptureKit",
        "Security",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
