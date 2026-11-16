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
pub use shell::{ShellSurface, should_close};
pub use softlock_core::PRODUCT_DISPLAY_NAME;
pub use toast_plan::{DISABLED_TOASTS, POLICY_SUPPRESSED, Restore, restore_of};
pub use tour::{Action, Confirm, Step, SurfaceId, steps, tour_duration};

/// Run the spike command. `print-plan` works on every OS. Showing a window
/// requires Windows.
pub fn run(config: Config) -> Result<(), String> {
    // The Windows backend is the production user of these values. Referencing
    // them here keeps that contract compiling on every OS.
    let _ = portable_contract();
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

fn portable_contract() -> u32 {
    let _ = tour::toast_xml(softlock_core::PRODUCT_DISPLAY_NAME);
    let _ = safety::HIT_TRANSPARENT;
    safety::EX_OVERLAY
        ^ safety::EX_TOPMOST
        ^ safety::EX_TRANSPARENT
        ^ safety::EX_TOOLWINDOW
        ^ safety::EX_LAYERED
        ^ safety::EX_NOACTIVATE
        ^ safety::CAPTURE_EXCLUDED
        ^ toast_plan::TOAST_ENABLED_SUBKEY.len() as u32
        ^ toast_plan::TOAST_ENABLED_VALUE.len() as u32
        ^ toast_plan::POLICY_SUBKEY.len() as u32
        ^ toast_plan::POLICY_VALUE.len() as u32
        ^ toast_plan::SENTINEL_SUBKEY.len() as u32
        ^ toast_plan::DISABLED_TOASTS
        ^ toast_plan::POLICY_SUPPRESSED
}
