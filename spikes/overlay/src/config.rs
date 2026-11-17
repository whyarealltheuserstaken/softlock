//! Command line for the overlay spike.

use std::time::Duration;

use crate::tour::SurfaceId;

/// Shown on the overlay and in the checklist.
pub const HOTKEY_LABEL: &str = "Ctrl+Alt+Shift+Q";

/// Henry's default. The window closes itself after this long.
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;
pub const MIN_TIMEOUT_SECS: u64 = 1;
pub const MAX_TIMEOUT_SECS: u64 = 60 * 60;
pub const DEFAULT_PAUSE_SECS: u64 = 20;
pub const DEFAULT_ALPHA: u8 = 200;

/// The green exclusive window never stays up longer than this.
pub const FULLSCREEN_MAX_SECS: u64 = 30;

/// How often the timer mitigation re-asserts topmost, in milliseconds.
pub const REASSERT_INTERVAL_MS: u32 = 250;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Run,
    Tour,
    Smoke,
    Fullscreen,
    RestoreToasts,
    PrintPlan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub command: Command,
    pub timeout: Duration,
    pub pause: Duration,
    pub alpha: u8,
    pub reassert_timer: bool,
    pub reassert_on_pos_change: bool,
    pub close_shell_surfaces: bool,
    pub suppress_toasts: bool,
    pub only: Option<SurfaceId>,
    /// Wait for Enter before the tour shows the overlay.
    pub prompt: bool,
    /// Smoke-only: prove a shell-looking window this process owns is not closed.
    pub shell_fixture: bool,
    /// Run, tour, smoke, and restore-toasts refuse elevation unless this is set.
    pub allow_admin: bool,
    pub fullscreen_secs: u64,
}

impl Config {
    pub fn parse(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Self, String> {
        let args: Vec<String> = args
            .into_iter()
            .map(|arg| arg.as_ref().to_string())
            .collect();
        let mut command = None;
        let mut timeout_secs = None;
        let mut pause_secs = DEFAULT_PAUSE_SECS;
        let mut alpha = DEFAULT_ALPHA;
        let mut reassert_timer = false;
        let mut reassert_on_pos_change = false;
        let mut close_shell_surfaces = false;
        let mut suppress_toasts = false;
        let mut only = None;
        let mut prompt = true;
        let mut shell_fixture = false;
        let mut allow_admin = false;
        let mut fullscreen_secs = None;
        let mut index = 0;
        while index < args.len() {
            let arg = args[index].as_str();
            match arg {
                "--help" | "-h" => return Err(usage()),
                "run" => set_command(&mut command, Command::Run)?,
                "tour" => set_command(&mut command, Command::Tour)?,
                "smoke" => set_command(&mut command, Command::Smoke)?,
                "fullscreen" => set_command(&mut command, Command::Fullscreen)?,
                "restore-toasts" => set_command(&mut command, Command::RestoreToasts)?,
                "print-plan" => set_command(&mut command, Command::PrintPlan)?,
                "--reassert-timer" => reassert_timer = true,
                "--reassert-pos" => reassert_on_pos_change = true,
                "--close-shell" => close_shell_surfaces = true,
                "--suppress-toasts" => suppress_toasts = true,
                "--no-prompt" => prompt = false,
                "--shell-fixture" => shell_fixture = true,
                "--allow-admin" => allow_admin = true,
                "--timeout-secs" => {
                    timeout_secs = Some(parse_secs(take_value(&args, &mut index, arg)?)?);
                }
                "--pause-secs" => {
                    pause_secs = parse_secs(take_value(&args, &mut index, arg)?)?;
                }
                "--alpha" => {
                    alpha = parse_alpha(&take_value(&args, &mut index, arg)?)?;
                }
                "--only" => {
                    only = Some(parse_surface(&take_value(&args, &mut index, arg)?)?);
                }
                "--seconds" => {
                    fullscreen_secs = Some(parse_secs(take_value(&args, &mut index, arg)?)?);
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown flag {other}\n\n{}", usage()));
                }
                other => {
                    return Err(format!("unknown command {other}\n\n{}", usage()));
                }
            }
            index += 1;
        }
        let command = command.unwrap_or(Command::Run);
        let timeout = Duration::from_secs(timeout_secs.unwrap_or(match command {
            Command::Smoke => 8,
            _ => DEFAULT_TIMEOUT_SECS,
        }));
        let fullscreen_secs = fullscreen_secs
            .unwrap_or(DEFAULT_PAUSE_SECS)
            .clamp(1, FULLSCREEN_MAX_SECS);
        if pause_secs == 0 {
            return Err("pause must be at least 1 second".into());
        }
        Ok(Self {
            command,
            timeout,
            pause: Duration::from_secs(pause_secs),
            alpha,
            reassert_timer,
            reassert_on_pos_change,
            close_shell_surfaces,
            suppress_toasts,
            only,
            prompt,
            shell_fixture,
            allow_admin,
            fullscreen_secs,
        })
    }

    pub fn mitigations_off(&self) -> bool {
        !self.reassert_timer
            && !self.reassert_on_pos_change
            && !self.close_shell_surfaces
            && !self.suppress_toasts
    }

    pub fn ensure_timeout_covers_tour(&self) -> Result<(), String> {
        if self.command != Command::Tour {
            return Ok(());
        }
        let needed = crate::tour::tour_duration(self.pause, self.only, self.fullscreen_secs);
        if self.timeout < needed {
            return Err(format!(
                "the timeout is {} seconds, but this tour needs at least {} seconds. Pass --timeout-secs {} (the maximum is {}).",
                self.timeout.as_secs(),
                needed.as_secs(),
                needed.as_secs(),
                MAX_TIMEOUT_SECS
            ));
        }
        Ok(())
    }
}

/// Run, tour, smoke, and restore-toasts refuse elevation unless `--allow-admin` was passed.
/// The fullscreen child stays allowed so a tour can still open the green window.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn elevated_command_allowed(
    command: Command,
    elevated: bool,
    allow_admin: bool,
) -> Result<(), String> {
    let gated = matches!(
        command,
        Command::Run | Command::Tour | Command::Smoke | Command::RestoreToasts
    );
    if gated && elevated && !allow_admin {
        Err("this program is running as admin. Close it and open a normal PowerShell. Do not choose Run as administrator. Pass --allow-admin only if school IT told you to.".into())
    } else {
        Ok(())
    }
}

fn set_command(slot: &mut Option<Command>, command: Command) -> Result<(), String> {
    if slot.is_some() {
        return Err("pass only one command".into());
    }
    *slot = Some(command);
    Ok(())
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_secs(text: String) -> Result<u64, String> {
    let secs: u64 = text
        .parse()
        .map_err(|_| format!("expected a number of seconds, got {text}"))?;
    if !(MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&secs) {
        return Err(format!(
            "seconds must be {MIN_TIMEOUT_SECS} to {MAX_TIMEOUT_SECS}, got {secs}"
        ));
    }
    Ok(secs)
}

fn parse_alpha(text: &str) -> Result<u8, String> {
    let alpha: u8 = text
        .parse()
        .map_err(|_| format!("alpha must be 1 to 255, got {text}"))?;
    if alpha == 0 {
        return Err("alpha 0 would be invisible. Use 1 to 255.".into());
    }
    Ok(alpha)
}

fn parse_surface(text: &str) -> Result<SurfaceId, String> {
    SurfaceId::parse(text).ok_or_else(|| {
        format!("unknown surface {text}. Use Start, TaskView, NotificationCenter, Toast, Uac, Fullscreen, or Monitor.")
    })
}

pub fn usage() -> String {
    format!(
        "\
spike-overlay shows an unsigned pink overlay on every monitor.
It does not block typing or clicking.
Exit: {HOTKEY_LABEL}
It also closes itself after {DEFAULT_TIMEOUT_SECS} seconds (5 minutes) unless you pass --timeout-secs.

Commands:
  spike-overlay [flags]                 keep the overlay up
  spike-overlay tour [flags]            open each surface in turn
  spike-overlay print-plan              print the surfaces and exit
  spike-overlay smoke [--timeout-secs N]
  spike-overlay fullscreen [--seconds N]
  spike-overlay restore-toasts          undo toast suppression if a previous run crashed

Flags:
  --timeout-secs N     1 to {MAX_TIMEOUT_SECS}, default {DEFAULT_TIMEOUT_SECS}
  --pause-secs N       seconds to look at each surface, default {DEFAULT_PAUSE_SECS}
  --reassert-timer     keep the overlay topmost on a timer
  --reassert-pos       keep the overlay topmost when its position changes
  --close-shell        dismiss Start, Task View, and the notification center with Escape. It does not close Explorer
  --suppress-toasts    turn the user toast setting off, and restore it on exit
  --only Surface       tour one surface
  --no-prompt          do not wait for Enter
  --allow-admin        run the tour even when this window is already admin
  --alpha N            1 to 255, default {DEFAULT_ALPHA}
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe_and_mitigations_are_off() {
        let config = Config::parse(None::<&str>).unwrap();
        assert_eq!(config.command, Command::Run);
        assert_eq!(config.timeout, Duration::from_secs(DEFAULT_TIMEOUT_SECS));
        assert_eq!(DEFAULT_TIMEOUT_SECS, 300);
        assert!(config.mitigations_off());
        assert!(config.prompt);
        assert!(!config.shell_fixture);
        assert!(!config.allow_admin);
        assert_eq!(config.alpha, DEFAULT_ALPHA);
    }

    #[test]
    fn each_mitigation_is_its_own_switch() {
        let config = Config::parse([
            "--reassert-timer",
            "--reassert-pos",
            "--close-shell",
            "--suppress-toasts",
        ])
        .unwrap();
        assert!(config.reassert_timer);
        assert!(config.reassert_on_pos_change);
        assert!(config.close_shell_surfaces);
        assert!(config.suppress_toasts);
    }

    #[test]
    fn timeout_rejects_zero_and_anything_over_an_hour() {
        assert!(Config::parse(["--timeout-secs", "0"]).is_err());
        assert!(Config::parse(["--timeout-secs", "3601"]).is_err());
        assert!(Config::parse(["--timeout-secs", "nope"]).is_err());
        let config = Config::parse(["--timeout-secs", "1"]).unwrap();
        assert_eq!(config.timeout.as_secs(), 1);
    }

    #[test]
    fn smoke_default_timeout_is_short_and_explicit_timeout_wins() {
        let smoke = Config::parse(["smoke"]).unwrap();
        assert_eq!(smoke.command, Command::Smoke);
        assert_eq!(smoke.timeout.as_secs(), 8);
        let smoke = Config::parse(["smoke", "--timeout-secs", "3"]).unwrap();
        assert_eq!(smoke.timeout.as_secs(), 3);
    }

    #[test]
    fn tour_refuses_a_timeout_shorter_than_the_look_time() {
        let config = Config::parse(["tour", "--pause-secs", "20", "--timeout-secs", "30"]).unwrap();
        let error = config.ensure_timeout_covers_tour().unwrap_err();
        assert!(error.contains("--timeout-secs"), "{error}");
    }

    #[test]
    fn fullscreen_seconds_are_capped() {
        let config = Config::parse(["fullscreen", "--seconds", "600"]).unwrap();
        assert_eq!(config.fullscreen_secs, FULLSCREEN_MAX_SECS);
    }

    #[test]
    fn unknown_flag_is_an_error() {
        assert!(Config::parse(["--reaassert-timer"]).is_err());
    }

    #[test]
    fn an_elevated_session_needs_allow_admin() {
        for command in [Command::Run, Command::Tour, Command::Smoke, Command::RestoreToasts] {
            assert!(elevated_command_allowed(command, false, false).is_ok());
            assert!(elevated_command_allowed(command, true, true).is_ok());
            let error = elevated_command_allowed(command, true, false).unwrap_err();
            assert!(error.contains("--allow-admin"), "{error}");
        }
        assert!(elevated_command_allowed(Command::Fullscreen, true, false).is_ok());
        let config = Config::parse(["tour", "--allow-admin"]).unwrap();
        assert!(config.allow_admin);
    }

    #[test]
    fn the_default_tour_fits_inside_the_default_timeout() {
        let config = Config::parse(["tour"]).unwrap();
        assert!(config.ensure_timeout_covers_tour().is_ok());
        assert_eq!(config.timeout.as_secs(), DEFAULT_TIMEOUT_SECS);
    }
}
