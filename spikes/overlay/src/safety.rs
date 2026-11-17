//! Exit copy and the style bits the overlay window has to carry.
//!
//! The numbers are the Win32 constants. A Windows test checks they still match
//! the SDK so a header change cannot silently drop click-through or topmost.

use std::time::Duration;

use crate::config::{Config, HOTKEY_LABEL};
use softlock_core::PRODUCT_DISPLAY_NAME;

/// Product crates must not link this spike, and no installer may copy it.
pub const SHIPS_IN_RELEASE: bool = false;

pub use crate::config::REASSERT_INTERVAL_MS;

#[cfg_attr(not(windows), allow(dead_code))]
pub const EX_TOPMOST: u32 = 0x0000_0008;
#[cfg_attr(not(windows), allow(dead_code))]
pub const EX_TRANSPARENT: u32 = 0x0000_0020;
#[cfg_attr(not(windows), allow(dead_code))]
pub const EX_TOOLWINDOW: u32 = 0x0000_0080;
#[cfg_attr(not(windows), allow(dead_code))]
pub const EX_LAYERED: u32 = 0x0008_0000;
#[cfg_attr(not(windows), allow(dead_code))]
pub const EX_NOACTIVATE: u32 = 0x0800_0000;
#[cfg_attr(not(windows), allow(dead_code))]
pub const EX_OVERLAY: u32 =
    EX_TOPMOST | EX_TRANSPARENT | EX_TOOLWINDOW | EX_LAYERED | EX_NOACTIVATE;

/// `WDA_EXCLUDEFROMCAPTURE`.
#[cfg_attr(not(windows), allow(dead_code))]
pub const CAPTURE_EXCLUDED: u32 = 0x11;

/// `HTTRANSPARENT`.
#[cfg_attr(not(windows), allow(dead_code))]
pub const HIT_TRANSPARENT: isize = -1;

pub fn overlay_lines(config: &Config) -> Vec<String> {
    vec![
        format!("{PRODUCT_DISPLAY_NAME} overlay spike"),
        "Click-through. Typing and clicking still work.".into(),
        "Phone photos only. Screenshots hide this pink.".into(),
        format!("Exit: {HOTKEY_LABEL}"),
        format!("Closes by itself in {}.", human_duration(config.timeout)),
        format!("Mitigations: {}", mitigation_label(config)),
    ]
}

pub fn mitigation_label(config: &Config) -> String {
    let mut on = Vec::new();
    if config.reassert_timer {
        on.push("timer");
    }
    if config.reassert_on_pos_change {
        on.push("window-position");
    }
    if config.close_shell_surfaces {
        on.push("close-shell");
    }
    if config.suppress_toasts {
        on.push("suppress-toasts");
    }
    if on.is_empty() {
        "none".into()
    } else {
        on.join(", ")
    }
}

pub fn human_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs.is_multiple_of(60) && secs >= 60 {
        let minutes = secs / 60;
        if minutes == 1 {
            "1 minute".into()
        } else {
            format!("{minutes} minutes")
        }
    } else {
        format!("{secs} seconds")
    }
}

/// True when `source` contains a call to an API that would swallow input.
#[cfg(test)]
fn forbids_input_block(source: &str) -> bool {
    for needle in forbidden_needles() {
        let call = format!("{needle}(");
        if source.contains(&call) {
            return false;
        }
    }
    true
}

#[cfg(test)]
fn forbidden_needles() -> Vec<String> {
    [
        ("SetWindows", "HookEx"),
        ("Block", "Input"),
        ("WH_KEYBOARD", "_LL"),
        ("WH_MOUSE", "_LL"),
    ]
    .into_iter()
    .map(|(left, right)| format!("{left}{right}"))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Command, Config, HOTKEY_LABEL};

    fn sample() -> Config {
        Config::parse(None::<&str>).unwrap()
    }

    #[test]
    fn the_overlay_tells_you_both_ways_out() {
        let lines = overlay_lines(&sample()).join("\n");
        assert!(lines.contains(PRODUCT_DISPLAY_NAME));
        assert!(lines.contains(HOTKEY_LABEL));
        assert!(lines.contains("5 minutes"));
        assert!(lines.contains("Click-through"));
        assert!(lines.contains("Phone photos only"));
        const { assert!(!SHIPS_IN_RELEASE) };
    }

    #[test]
    fn mitigations_are_named_only_when_the_switch_is_on() {
        let mut config = sample();
        assert_eq!(mitigation_label(&config), "none");
        config.reassert_timer = true;
        config.suppress_toasts = true;
        assert_eq!(mitigation_label(&config), "timer, suppress-toasts");
        assert_eq!(REASSERT_INTERVAL_MS, 250);
        let _ = (config.command, Command::Run);
    }

    #[test]
    fn style_bits_include_topmost_click_through_and_no_activate() {
        assert_ne!(EX_OVERLAY & EX_TOPMOST, 0);
        assert_ne!(EX_OVERLAY & EX_TRANSPARENT, 0);
        assert_ne!(EX_OVERLAY & EX_LAYERED, 0);
        assert_ne!(EX_OVERLAY & EX_NOACTIVATE, 0);
        assert_ne!(EX_OVERLAY & EX_TOOLWINDOW, 0);
        assert_eq!(CAPTURE_EXCLUDED, 0x11);
        assert_eq!(HIT_TRANSPARENT, -1);
    }

    #[test]
    fn input_blocking_calls_are_rejected_and_injected_keys_are_not() {
        let blocked = format!("{}{}", "Block", "Input(");
        assert!(!forbids_input_block(&blocked));
        assert!(forbids_input_block(
            "SendInput is how the tour opens a menu"
        ));
    }

    #[test]
    fn the_spike_source_never_blocks_input_and_does_not_ship() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut source = String::new();
        collect_text(&root.join("src"), &mut source);
        assert!(
            forbids_input_block(&source),
            "the overlay spike called an API that swallows typing or clicking"
        );
        const { assert!(!SHIPS_IN_RELEASE) };

        let repo = root.join("../..");
        let spike_manifest = std::fs::canonicalize(root.join("Cargo.toml")).unwrap();
        let mut cargo_files = Vec::new();
        collect_named(&repo, "Cargo.toml", &mut cargo_files);
        for path in cargo_files {
            if std::fs::canonicalize(&path).ok().as_ref() == Some(&spike_manifest) {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                !text.contains("softlock-spike-overlay"),
                "{} must not depend on the spike",
                path.display()
            );
        }

        for relative in [
            "docs/findings/07-overlay/CHECKLIST.md",
            "docs/findings/07-overlay/FINDING.md",
        ] {
            let text = std::fs::read_to_string(repo.join(relative))
                .unwrap_or_else(|error| panic!("missing {relative}: {error}"));
            assert!(
                !text.contains("- [x]") && !text.contains("- [X]"),
                "{relative} marks a manual box done"
            );
        }
        let finding =
            std::fs::read_to_string(repo.join("docs/findings/07-overlay/FINDING.md")).unwrap();
        assert!(finding.contains("#18"));
        assert!(finding.contains("#30"));
        let checklist =
            std::fs::read_to_string(repo.join("docs/findings/07-overlay/CHECKLIST.md")).unwrap();
        assert!(checklist.contains(HOTKEY_LABEL));
        assert!(checklist.contains("ThinkStation"));
        assert!(checklist.contains("laptop"));
        assert!(checklist.contains("covered"));
        assert!(checklist.contains("drawn above"));
        assert!(!checklist.contains("visible-but-not-interactive"));
        assert!(checklist.contains("mitigated by"));
        assert!(checklist.contains("school IT"));
        assert!(checklist.contains("If you get stuck"));
        assert!(checklist.contains("restore-toasts"));
        assert!(checklist.contains("1.100.0"));
        assert!(checklist.to_ascii_lowercase().contains("phone"));
        assert!(checklist.contains("Never click Yes"));
        assert!(finding.contains("input guard"));
        assert!(finding.contains("drawn above"));
        assert!(!finding.contains("visible-but-not-interactive"));
        assert!(finding.contains("in-process"));
        let shell_src = std::fs::read_to_string(root.join("src/win/shell.rs")).unwrap();
        assert!(
            !shell_src.contains(&format!("{}{}", "WM_", "CLOSE")),
            "close-shell posts a close message to a window"
        );
    }

    fn collect_text(dir: &std::path::Path, out: &mut String) {
        let entries = std::fs::read_dir(dir).unwrap();
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect_text(&path, out);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                out.push_str(&std::fs::read_to_string(&path).unwrap());
                out.push('\n');
            }
        }
    }

    fn collect_named(dir: &std::path::Path, name: &str, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if path.is_dir() {
                if file_name == "target" || file_name == ".git" || file_name == "node_modules" {
                    continue;
                }
                collect_named(&path, name, out);
            } else if file_name == name {
                out.push(path);
            }
        }
    }
}
