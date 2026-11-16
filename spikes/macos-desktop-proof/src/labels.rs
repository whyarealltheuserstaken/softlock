//! Throwaway launchd labels. These are not the product's launchd ids.

pub const AGENT_LABEL: &str = "com.runner.macosproof.agent";
pub const DAEMON_LABEL: &str = "com.runner.macosproof.daemon";
pub const DAEMON_PLIST: &str = "/Library/LaunchDaemons/com.runner.macosproof.daemon.plist";
pub const AGENT_PLIST_NAME: &str = "com.runner.macosproof.agent.plist";

pub const CAPTURE_APIS: [&str; 4] = [
    "CGWindowListCreateImage",
    "CGDisplayCreateImage",
    "screencapture",
    "ScreenCaptureKit",
];

/// Sharing types m04 measures. `default` is the value AppKit leaves unset.
pub const SHARING_TYPES: [&str; 2] = ["default", "none"];

/// What `macos_prove_screen_capture_kit` calls: one whole-display image and an empty exclude list.
pub const SCREEN_CAPTURE_ENTRY: &str = "SCScreenshotManager excludingWindows empty";

pub const REQUIRED_TAPS: [(&str, &str); 3] = [
    ("kCGSessionEventTap", "user"),
    ("kCGHIDEventTap", "user"),
    ("kCGHIDEventTap", "root"),
];
