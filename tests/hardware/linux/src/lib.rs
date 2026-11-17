//! Linux hardware suite. Not a product crate.
//!
//! Shared result schema: `softlock-hardware-suite/1` in `report`.

#[cfg(not(target_os = "linux"))]
compile_error!("linux-hardware-suite builds only on Linux");

pub mod args;
pub mod detect;
pub mod display;
pub mod exec;
pub mod fail_closed;
pub mod input;
pub mod overlay;
pub mod physical;
pub mod power;
pub mod prompt;
pub mod redact;
pub mod report;
pub mod restore;
pub mod runner;
pub mod table;

pub use report::{SCHEMA, SuiteResult};
