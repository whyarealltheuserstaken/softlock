//! Native x86_64 smoke. Refuses a Rosetta `uname`.

#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "macos")]
use crate::args::Args;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostFacts {
    pub uname_m: String,
    pub proc_translated: Option<String>,
    pub hw_optional_arm64: Option<String>,
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
        format!("lipo -info {info}"),
        format!("lipo -archs {archs}"),
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
    if ok {
        lines.push("smoke=pass".to_string());
    } else {
        lines.push("smoke=fail".to_string());
    }
    (lines.join("\n"), ok)
}

#[cfg(target_os = "macos")]
pub fn run(args: &Args) -> i32 {
    let _ = args;
    let facts = HostFacts {
        uname_m: command_stdout(&["uname", "-m"]),
        proc_translated: optional_sysctl("sysctl.proc_translated"),
        hw_optional_arm64: optional_sysctl("hw.optional.arm64"),
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
    let (text, ok) = render(
        &facts,
        &info,
        &archs,
        native_intel(&facts),
        fat_universal(&info, &archs),
    );
    println!("{text}");
    println!("runner-image={}", args.runner_image);
    if ok { 0 } else { 1 }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn intel() -> HostFacts {
        HostFacts {
            uname_m: "x86_64".to_string(),
            proc_translated: Some("0".to_string()),
            hw_optional_arm64: Some("0".to_string()),
        }
    }

    #[test]
    fn native_intel_accepts_x86_64_without_translation() {
        native_intel(&intel()).expect("native");
        let missing_keys = HostFacts {
            uname_m: "x86_64".to_string(),
            proc_translated: None,
            hw_optional_arm64: None,
        };
        native_intel(&missing_keys).expect("Intel hosts may omit the Rosetta keys");
    }

    #[test]
    fn a_rosetta_uname_on_apple_silicon_is_not_native_intel() {
        let rosetta = HostFacts {
            uname_m: "x86_64".to_string(),
            proc_translated: Some("1".to_string()),
            hw_optional_arm64: Some("1".to_string()),
        };
        let error = native_intel(&rosetta).expect_err("Rosetta");
        assert!(error.contains("Rosetta"));

        let arm = HostFacts {
            uname_m: "arm64".to_string(),
            proc_translated: Some("0".to_string()),
            hw_optional_arm64: Some("1".to_string()),
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
        );
        assert!(ok);
        assert!(text.contains("uname -m\nx86_64"));
        assert!(text.contains("native-intel=true"));
        assert!(text.contains("rosetta=false"));
        assert!(text.contains("smoke=pass"));
        assert!(text.contains("x86_64"));
        assert!(text.contains("arm64"));
    }
}
