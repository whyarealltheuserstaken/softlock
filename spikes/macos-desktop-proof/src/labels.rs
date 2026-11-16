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

pub const REQUIRED_TAPS: [(&str, &str); 3] = [
    ("kCGSessionEventTap", "user"),
    ("kCGHIDEventTap", "user"),
    ("kCGHIDEventTap", "root"),
];
