//! The only commands the kit may spawn.
//!
//! Read-only commands report versions, the screen-saver password keys, and
//! FileVault status. The screen-saver engine is not in that list. Nothing here
//! writes preferences or changes FileVault.

use softlock_macos::SCREEN_SAVER_ENGINE_CANDIDATES;

pub fn read_only_argv_allowed(program: &str, args: &[&str]) -> bool {
    matches!(
        (program, args),
        ("/usr/bin/sw_vers", [])
            | ("/usr/bin/uname", ["-m"])
            | ("/usr/bin/id", ["-u"])
            | ("/usr/sbin/sysctl", ["-n", "hw.model"])
            | (
                "/usr/bin/defaults",
                ["read", "com.apple.screensaver", "askForPassword"]
            )
            | (
                "/usr/bin/defaults",
                [
                    "-currentHost",
                    "read",
                    "com.apple.screensaver",
                    "askForPassword"
                ]
            )
            | (
                "/usr/bin/defaults",
                ["read", "com.apple.screensaver", "askForPasswordDelay"]
            )
            | (
                "/usr/bin/defaults",
                [
                    "-currentHost",
                    "read",
                    "com.apple.screensaver",
                    "askForPasswordDelay"
                ]
            )
            | ("/usr/bin/fdesetup", ["status"])
            | ("/usr/bin/fdesetup", ["isactive"])
            | ("/usr/sbin/sysadminctl", ["-screenLock", "status"])
    ) || otool_dash_l(program, args)
}

pub fn engine_path_allowed(program: &str) -> bool {
    SCREEN_SAVER_ENGINE_CANDIDATES.contains(&program)
}

pub fn observation_argv_allowed(program: &str, args: &[&str]) -> bool {
    program == "/usr/bin/pgrep" && args == ["-x", "ScreenSaverEngine"]
}

fn otool_dash_l(program: &str, args: &[&str]) -> bool {
    program == "/usr/bin/otool"
        && args.len() == 2
        && args[0] == "-L"
        && !args[1].is_empty()
        && !args[1].starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_commands_are_status_reads() {
        assert!(read_only_argv_allowed("/usr/bin/fdesetup", &["status"]));
        assert!(read_only_argv_allowed("/usr/bin/fdesetup", &["isactive"]));
        assert!(read_only_argv_allowed(
            "/usr/bin/defaults",
            &["read", "com.apple.screensaver", "askForPasswordDelay"]
        ));
        assert!(read_only_argv_allowed(
            "/usr/sbin/sysctl",
            &["-n", "hw.model"]
        ));
        assert!(read_only_argv_allowed(
            "/usr/bin/otool",
            &["-L", "/tmp/macos-lock-kit"]
        ));
        assert!(read_only_argv_allowed(
            "/usr/sbin/sysadminctl",
            &["-screenLock", "status"]
        ));
    }

    #[test]
    fn writes_and_extra_verbs_are_rejected() {
        assert!(!read_only_argv_allowed("/usr/bin/fdesetup", &["enable"]));
        assert!(!read_only_argv_allowed("/usr/bin/fdesetup", &["disable"]));
        assert!(!read_only_argv_allowed(
            "/usr/bin/fdesetup",
            &["status", "-extended"]
        ));
        assert!(!read_only_argv_allowed(
            "/usr/bin/defaults",
            &[
                "write",
                "com.apple.screensaver",
                "askForPassword",
                "-int",
                "1"
            ]
        ));
        assert!(!read_only_argv_allowed("/usr/sbin/sysctl", &["-a"]));
        assert!(!read_only_argv_allowed("/bin/sh", &["-c", "true"]));
        assert!(!read_only_argv_allowed(
            "/usr/sbin/sysadminctl",
            &["-screenLock", "immediate"]
        ));
        assert!(!read_only_argv_allowed(
            "/usr/sbin/sysadminctl",
            &["-screenLock", "off"]
        ));
        assert!(!read_only_argv_allowed("/usr/bin/sudo", &["-n", "true"]));
        assert!(!engine_path_allowed("/usr/bin/fdesetup"));
        assert!(engine_path_allowed(SCREEN_SAVER_ENGINE_CANDIDATES[0]));
        assert!(!observation_argv_allowed(
            "/usr/bin/pgrep",
            &["-lf", "login"]
        ));
        assert!(observation_argv_allowed(
            "/usr/bin/pgrep",
            &["-x", "ScreenSaverEngine"]
        ));
        assert!(!read_only_argv_allowed(
            SCREEN_SAVER_ENGINE_CANDIDATES[0],
            &[]
        ));
    }
}
