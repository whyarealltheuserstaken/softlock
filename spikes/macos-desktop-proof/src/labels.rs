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

/// Sharing phases. `default` is the value AppKit leaves unset. `baseline` is the same capture with the windows ordered out.
pub const SHARING_PHASES: [&str; 3] = ["default", "none", "baseline"];

/// The four m02 paths plus one `SCStream` frame of the whole display.
pub const SHARING_APIS: [&str; 5] = [
    "CGWindowListCreateImage",
    "CGDisplayCreateImage",
    "screencapture",
    "ScreenCaptureKit",
    "SCStream",
];

/// What `macos_prove_screen_capture_kit` calls: one whole-display image and an empty exclude list.
pub const SCREEN_CAPTURE_ENTRY: &str = "SCScreenshotManager excludingWindows empty";

/// One `SCStream` frame of the display, with no windows excluded.
pub const SCSTREAM_ENTRY: &str = "SCStream excludingWindows empty";

pub const REQUIRED_TAPS: [(&str, &str); 3] = [
    ("kCGSessionEventTap", "user"),
    ("kCGHIDEventTap", "user"),
    ("kCGHIDEventTap", "root"),
];
