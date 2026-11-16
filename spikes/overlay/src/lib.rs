//! Proof overlay for spike P3.
//!
//! Unsigned, topmost, capture-excluded, and click-through. It covers every
//! monitor and does not block typing or clicking. It closes itself after a
//! timeout (5 minutes unless you pass a shorter one) and on Ctrl+Alt+Shift+Q.
//!
//! This crate is a spike. It is not installed, and product crates must not
//! depend on it. [`SHIPS_IN_RELEASE`] stays false.

mod config;
mod coverage;
mod safety;
mod shell;
mod toast_plan;
mod tour;

#[cfg(windows)]
mod win;

pub use config::{
    Command, Config, DEFAULT_ALPHA, DEFAULT_PAUSE_SECS, DEFAULT_TIMEOUT_SECS, FULLSCREEN_MAX_SECS,
    HOTKEY_LABEL, MAX_TIMEOUT_SECS, MIN_TIMEOUT_SECS,
};
pub use coverage::{MonitorRect, reconcile, windows_for};
pub use safety::{REASSERT_INTERVAL_MS, SHIPS_IN_RELEASE, overlay_lines};
pub use shell::{ShellSurface, identify};
pub use softlock_core::PRODUCT_DISPLAY_NAME;
pub use toast_plan::{DISABLED_TOASTS, Restore, restore_of, run_once_command, suppression_blocked};
pub use tour::{Action, Confirm, Step, SurfaceId, steps, tour_duration};

/// Run the spike command. `print-plan` works on every OS. Showing a window
/// requires Windows.
pub fn run(config: Config) -> Result<(), String> {
    match config.command {
        Command::PrintPlan => {
            print!("{}", tour::plan_text());
            Ok(())
        }
        _ => dispatch(config),
    }
}

#[cfg(windows)]
fn dispatch(config: Config) -> Result<(), String> {
    win::run(config)
}

#[cfg(not(windows))]
fn dispatch(config: Config) -> Result<(), String> {
    let _ = config;
    Err("this spike shows a window on Windows only".into())
}
