//! Native x86_64 smoke. Refuses a Rosetta `uname`. Calls public CoreGraphics
//! in-process. Resolves the D5 lock symbol with dlopen/dlsym and never calls it.

#[cfg(target_os = "macos")]
use std::ffi::CString;
#[cfg(target_os = "macos")]
use std::os::raw::c_void;
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "macos")]
use crate::args::Args;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostFacts {
    pub uname_m: String,
    pub proc_translated: Option<String>,
    pub hw_optional_arm64: Option<String>,
    pub hw_cputype: Option<String>,
    pub cpu_brand: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameworkReport {
    pub display_id: String,
    pub bounds: String,
    pub session_dictionary: String,
    pub sac_lock_symbol: String,
}

pub fn native_intel(facts: &HostFacts) -> Result<(), String> {
    if facts.uname_m != "x86_64" {
        return Err(format!(
            "uname -m is {}, expected x86_64",
            display(&facts.uname_m)
        ));
    }
    if facts.proc_translated.as_deref() == Some("1") {
        return Err("sysctl.proc_translated is 1; this is Rosetta, not native Intel".to_string());
    }
    if facts.hw_optional_arm64.as_deref() == Some("1") {
        return Err(
            "hw.optional.arm64 is 1; this host is Apple silicon, not native Intel".to_string(),
        );
    }
    if facts.hw_cputype.as_deref() != Some("7") {
        return Err(format!(
            "hw.cputype is {}, expected 7",
            display(facts.hw_cputype.as_deref().unwrap_or(""))
        ));
    }
    let brand = facts.cpu_brand.as_deref().unwrap_or("");
    if !brand.contains("Intel") {
        return Err(format!(
            "machdep.cpu.brand_string is {}, expected Intel",
            display(brand)
        ));
    }
    Ok(())
}

pub fn fat_universal(info: &str, archs: &str) -> Result<(), String> {
    for (label, text) in [("lipo -info", info), ("lipo -archs", archs)] {
        if !text.contains("x86_64") {
            return Err(format!("{label} missing x86_64: {text}"));
        }
        if !text.contains("arm64") {
            return Err(format!("{label} missing arm64: {text}"));
        }
    }
    Ok(())
}

pub fn render(
    facts: &HostFacts,
    info: &str,
    archs: &str,
    native: Result<(), String>,
    fat: Result<(), String>,
    framework: &FrameworkReport,
) -> (String, bool) {
    let mut lines = vec![
        "uname -m".to_string(),
        facts.uname_m.clone(),
        format!(
            "sysctl.proc_translated={}",
            facts.proc_translated.as_deref().unwrap_or("absent")
        ),
        format!(
            "hw.optional.arm64={}",
            facts.hw_optional_arm64.as_deref().unwrap_or("absent")
        ),
        format!(
            "hw.cputype={}",
            facts.hw_cputype.as_deref().unwrap_or("absent")
        ),
        format!(
            "machdep.cpu.brand_string={}",
            facts.cpu_brand.as_deref().unwrap_or("absent")
        ),
        format!("lipo -info {info}"),
        format!("lipo -archs {archs}"),
        format!("cg-main-display-id={}", framework.display_id),
        format!("cg-main-display-bounds={}", framework.bounds),
        format!("cg-session-dictionary={}", framework.session_dictionary),
        format!("sac-lock-symbol={}", framework.sac_lock_symbol),
        "sac-lock-called=false".to_string(),
    ];
    let mut ok = true;
    match &native {
        Ok(()) => {
            lines.push("native-intel=true".to_string());
            lines.push("rosetta=false".to_string());
        }
        Err(error) => {
            ok = false;
            lines.push("native-intel=false".to_string());
            lines.push(format!("native-error={error}"));
        }
    }
    match &fat {
        Ok(()) => lines.push("fat=true".to_string()),
        Err(error) => {
            ok = false;
            lines.push("fat=false".to_string());
            lines.push(format!("fat-error={error}"));
        }
    }
    for error in framework_failures(framework) {
        ok = false;
        lines.push(format!("framework-error={error}"));
    }
    if ok {
        lines.push("smoke=pass".to_string());
    } else {
        lines.push("smoke=fail".to_string());
    }
    (lines.join("\n"), ok)
}

#[cfg(target_os = "macos")]
pub fn run(args: &Args) -> i32 {
    let facts = HostFacts {
        uname_m: command_stdout(&["uname", "-m"]),
        proc_translated: optional_sysctl("sysctl.proc_translated"),
        hw_optional_arm64: optional_sysctl("hw.optional.arm64"),
        hw_cputype: optional_sysctl("hw.cputype"),
        cpu_brand: optional_sysctl("machdep.cpu.brand_string"),
    };
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("current exe: {error}");
            return 1;
        }
    };
    let info = command_stdout(&["lipo", "-info", &exe.to_string_lossy()]);
    let archs = command_stdout(&["lipo", "-archs", &exe.to_string_lossy()]);
    let framework = measure_frameworks();
    let (text, ok) = render(
        &facts,
        &info,
        &archs,
        native_intel(&facts),
        fat_universal(&info, &archs),
        &framework,
    );
    println!("{text}");
    println!("runner-image={}", args.runner_image);
    if ok { 0 } else { 1 }
}

fn framework_failures(framework: &FrameworkReport) -> Vec<String> {
    let mut errors = Vec::new();
    if framework.display_id == "0" {
        errors.push("cg-main-display-id is 0".to_string());
    }
    match parse_bounds(&framework.bounds) {
        Ok((width, height)) => {
            if width == 0.0 {
                errors.push("cg-main-display-bounds width is 0".to_string());
            }
            if height == 0.0 {
                errors.push("cg-main-display-bounds height is 0".to_string());
            }
        }
        Err(error) => errors.push(error),
    }
    if framework.session_dictionary != "non-null" {
        errors.push(format!(
            "cg-session-dictionary is {}, expected non-null",
            display(&framework.session_dictionary)
        ));
    }
    if framework.sac_lock_symbol != "resolved" {
        errors.push(format!(
            "sac-lock-symbol is {}, expected resolved",
            display(&framework.sac_lock_symbol)
        ));
    }
    errors
}

fn parse_bounds(bounds: &str) -> Result<(f64, f64), String> {
    let parts: Vec<&str> = bounds.split(',').collect();
    if parts.len() != 4 {
        return Err(format!("cg-main-display-bounds is {bounds}"));
    }
    let width = parts[2]
        .parse::<f64>()
        .map_err(|_| format!("cg-main-display-bounds width is {}", parts[2]))?;
    let height = parts[3]
        .parse::<f64>()
        .map_err(|_| format!("cg-main-display-bounds height is {}", parts[3]))?;
    Ok((width, height))
}

fn display(value: &str) -> String {
    if value.is_empty() {
        "(empty)".to_string()
    } else {
        value.to_string()
    }
}

#[cfg(target_os = "macos")]
fn command_stdout(argv: &[&str]) -> String {
    let Some((program, rest)) = argv.split_first() else {
        return String::new();
    };
    match Command::new(program).args(rest).output() {
        Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_string(),
        Err(_) => String::new(),
    }
}

#[cfg(target_os = "macos")]
fn optional_sysctl(name: &str) -> Option<String> {
    let output = Command::new("sysctl").args(["-n", name]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct CGRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(id: u32) -> CGRect;
    fn CGSessionCopyCurrentDictionary() -> *mut c_void;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// Open the private login image and look up the D5 symbol. The pointer is
/// discarded. This function must not call the symbol.
#[cfg(target_os = "macos")]
fn sac_lock_symbol_resolved() -> bool {
    let path = CString::new("/System/Library/PrivateFrameworks/login.framework/login")
        .expect("login path");
    let name = CString::new("SACLockScreenImmediate").expect("symbol name");
    unsafe {
        let handle = libc::dlopen(path.as_ptr(), libc::RTLD_LAZY);
        if handle.is_null() {
            return false;
        }
        let symbol = libc::dlsym(handle, name.as_ptr());
        !symbol.is_null()
    }
}

#[cfg(target_os = "macos")]
fn measure_frameworks() -> FrameworkReport {
    unsafe {
        let id = CGMainDisplayID();
        let bounds = CGDisplayBounds(id);
        let session = CGSessionCopyCurrentDictionary();
        let session_dictionary = if session.is_null() {
            "null".to_string()
        } else {
            CFRelease(session);
            "non-null".to_string()
        };
        FrameworkReport {
            display_id: id.to_string(),
            bounds: format!(
                "{},{},{},{}",
                bounds.x, bounds.y, bounds.width, bounds.height
            ),
            session_dictionary,
            sac_lock_symbol: if sac_lock_symbol_resolved() {
                "resolved".to_string()
            } else {
                "missing".to_string()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intel() -> HostFacts {
        HostFacts {
            uname_m: "x86_64".to_string(),
            proc_translated: Some("0".to_string()),
            hw_optional_arm64: Some("0".to_string()),
            hw_cputype: Some("7".to_string()),
            cpu_brand: Some("Intel(R) Core(TM) i5".to_string()),
        }
    }

    fn framework() -> FrameworkReport {
        FrameworkReport {
            display_id: "1".to_string(),
            bounds: "0,0,1024,768".to_string(),
            session_dictionary: "non-null".to_string(),
            sac_lock_symbol: "resolved".to_string(),
        }
    }

    #[test]
    fn native_intel_accepts_x86_64_intel_cputype() {
        native_intel(&intel()).expect("native");
    }

    #[test]
    fn native_intel_requires_cputype_7_and_an_intel_brand() {
        let mut wrong_type = intel();
        wrong_type.hw_cputype = Some("16777228".to_string());
        assert!(
            native_intel(&wrong_type)
                .unwrap_err()
                .contains("hw.cputype")
        );

        let mut apple = intel();
        apple.cpu_brand = Some("Apple M1".to_string());
        assert!(native_intel(&apple).unwrap_err().contains("Intel"));

        let mut missing = intel();
        missing.hw_cputype = None;
        missing.cpu_brand = None;
        assert!(native_intel(&missing).is_err());
    }

    #[test]
    fn a_rosetta_uname_on_apple_silicon_is_not_native_intel() {
        let rosetta = HostFacts {
            uname_m: "x86_64".to_string(),
            proc_translated: Some("1".to_string()),
            hw_optional_arm64: Some("1".to_string()),
            hw_cputype: Some("7".to_string()),
            cpu_brand: Some("Intel".to_string()),
        };
        let error = native_intel(&rosetta).expect_err("Rosetta");
        assert!(error.contains("Rosetta"));

        let arm = HostFacts {
            uname_m: "arm64".to_string(),
            proc_translated: Some("0".to_string()),
            hw_optional_arm64: Some("1".to_string()),
            hw_cputype: Some("16777228".to_string()),
            cpu_brand: Some("Apple M1".to_string()),
        };
        let error = native_intel(&arm).expect_err("arm64");
        assert!(error.contains("arm64"));
    }

    #[test]
    fn lipo_must_name_both_slices() {
        fat_universal(
            "Architectures in the fat file: x86_64 arm64",
            "x86_64 arm64",
        )
        .expect("fat");
        assert!(
            fat_universal("x86_64", "x86_64")
                .unwrap_err()
                .contains("arm64")
        );
        assert!(
            fat_universal("arm64", "arm64")
                .unwrap_err()
                .contains("x86_64")
        );
    }

    #[test]
    fn smoke_pass_text_names_native_intel_not_rosetta() {
        let info = "Architectures in the fat file: macos-desktop-proof are: x86_64 arm64";
        let archs = "x86_64 arm64";
        let (text, ok) = render(
            &intel(),
            info,
            archs,
            native_intel(&intel()),
            fat_universal(info, archs),
            &framework(),
        );
        assert!(ok);
        assert!(text.contains("uname -m\nx86_64"));
        assert!(text.contains("hw.cputype=7"));
        assert!(text.contains("Intel"));
        assert!(text.contains("native-intel=true"));
        assert!(text.contains("rosetta=false"));
        assert!(text.contains("cg-main-display-id=1"));
        assert!(text.contains("cg-session-dictionary=non-null"));
        assert!(text.contains("sac-lock-symbol=resolved"));
        assert!(text.contains("sac-lock-called=false"));
        assert!(text.contains("smoke=pass"));
        assert!(!text.contains("framework-error="));
    }

    fn render_framework(report: FrameworkReport) -> (String, bool) {
        let info = "Architectures in the fat file: macos-desktop-proof are: x86_64 arm64";
        let archs = "x86_64 arm64";
        render(
            &intel(),
            info,
            archs,
            native_intel(&intel()),
            fat_universal(info, archs),
            &report,
        )
    }

    #[test]
    fn smoke_fails_when_the_display_id_is_zero() {
        let mut report = framework();
        report.display_id = "0".to_string();
        let (text, ok) = render_framework(report);
        assert!(!ok);
        assert!(text.contains("framework-error=cg-main-display-id is 0"));
        assert!(text.contains("smoke=fail"));
    }

    #[test]
    fn smoke_fails_when_the_display_width_is_zero() {
        let mut report = framework();
        report.bounds = "0,0,0,768".to_string();
        let (text, ok) = render_framework(report);
        assert!(!ok);
        assert!(text.contains("framework-error=cg-main-display-bounds width is 0"));
        assert!(text.contains("smoke=fail"));
    }

    #[test]
    fn smoke_fails_when_the_display_height_is_zero() {
        let mut report = framework();
        report.bounds = "0,0,1024,0".to_string();
        let (text, ok) = render_framework(report);
        assert!(!ok);
        assert!(text.contains("framework-error=cg-main-display-bounds height is 0"));
        assert!(text.contains("smoke=fail"));
    }

    #[test]
    fn smoke_fails_when_the_session_dictionary_is_null() {
        let mut report = framework();
        report.session_dictionary = "null".to_string();
        let (text, ok) = render_framework(report);
        assert!(!ok);
        assert!(text.contains("framework-error=cg-session-dictionary is null, expected non-null"));
        assert!(text.contains("smoke=fail"));
    }

    #[test]
    fn smoke_fails_when_the_lock_symbol_is_missing() {
        let mut report = framework();
        report.sac_lock_symbol = "missing".to_string();
        let (text, ok) = render_framework(report);
        assert!(!ok);
        assert!(text.contains("framework-error=sac-lock-symbol is missing, expected resolved"));
        assert!(text.contains("smoke=fail"));
    }

    #[test]
    fn smoke_source_never_calls_the_private_lock_symbol() {
        let src = include_str!("smoke.rs");
        let production = src.split("#[cfg(test)]").next().expect("production");
        assert!(production.contains("dlopen"));
        assert!(production.contains("dlsym"));
        assert!(production.contains("SACLockScreenImmediate"));
        assert!(production.contains("login.framework"));
        assert!(
            !production.contains("transmute"),
            "resolving the symbol must not transmute it into a callable"
        );
        assert!(
            !production.contains("SACLockScreenImmediate("),
            "the private lock function must not be called"
        );
        assert!(production.contains("The pointer is"));
        assert!(production.contains("discarded"));
    }
}
