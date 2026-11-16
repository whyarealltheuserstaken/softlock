//! Keep-awake spike for Modern Standby and a closed lid.
//!
//! The crate is a workspace member so CI can test it, and it is not a default
//! member, so a normal release build does not produce it. No shipping crate
//! depends on it.

// The Windows host is the only caller of the log writer and the power-plan
// helpers. Linux and macOS CI still typecheck those helpers.
#![cfg_attr(not(windows), allow(dead_code))]

mod analyze;
mod cli;
mod logfile;
mod logline;
mod plan;
mod scenario;
mod session;
mod signals;
mod timefmt;

#[cfg(windows)]
mod win;

pub use analyze::{AnalyzeError, Summary, summarize_dir};
pub use cli::{
    LOG_DIR, PACKAGE, Parsed, RunSpec, analyze_command, parse_args, practice_command, run_command,
    smoke_command, usage,
};
pub use plan::{CLOSE_LID_LINE, MachineView, OverlayLook, PowerPlan, spike_power_plan};
pub use scenario::{MEASUREMENT_SCENARIOS, Scenario};

use std::path::Path;

/// Run the spike from already-split arguments. Returns a process exit code.
pub fn run_from_args<I, S>(args: I, cwd: &Path) -> u8
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    match parse_args(args, cwd) {
        Ok(Parsed::Help) => {
            println!("{}", usage());
            0
        }
        Ok(Parsed::Analyze { dir }) => match summarize_dir(&dir) {
            Ok(summary) => {
                println!("{}", summary.text);
                0
            }
            Err(error) => {
                eprintln!("{error}");
                1
            }
        },
        Ok(Parsed::Run(spec)) => match session::run(&spec) {
            Ok(report) => {
                session::print_finished(&spec, &report);
                0
            }
            Err(error) => {
                eprintln!("{}", error.message());
                error.code()
            }
        },
        Ok(Parsed::Smoke) => match session::smoke() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("{}", error.message());
                error.code()
            }
        },
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}
