//! Windows OS-lock calls.
//!
//! The decision lives in `softlock-core`. This crate performs the calls and
//! confirms them. It does not clear `DisableLockWorkstation` and it does not
//! reconnect a session.

mod gate;

pub use gate::{SchoolCheck, classify_school, hosted_runner, proof_allowed};

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::{
    CloseResult, run_lock_host, run_lock_host_primary, run_lock_host_service_dead, run_service,
    run_service_disconnect, runner_may_lock,
};

#[cfg(not(windows))]
pub fn runner_may_lock() -> Result<(), &'static str> {
    Err("not-windows")
}

pub fn confirm_main() -> i32 {
    #[cfg(windows)]
    {
        win::confirm_main()
    }
    #[cfg(not(windows))]
    {
        eprintln!("os-lock confirm runs on Windows");
        2
    }
}
