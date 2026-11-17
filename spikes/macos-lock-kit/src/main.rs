//! Throwaway macOS lock-path kit. Not linked into product binaries.
//!
//! The default run is read-only. It resolves `SACLockScreenImmediate` and does
//! not call it. An exercise run is a separate process Henry starts himself.

mod args;
mod commands;
mod helper;
mod kit_gate;
mod lock_attempt;
mod redact;
mod report;
mod text;

#[cfg(unix)]
mod signals;

#[cfg(target_os = "macos")]
mod mac;

fn main() {
    #[cfg(unix)]
    if let Some(code) = signals::simulate_from_env() {
        std::process::exit(code);
    }
    std::process::exit(dispatch());
}

fn dispatch() -> i32 {
    let args = match args::parse_env() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    #[cfg(target_os = "macos")]
    {
        mac::run(args)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = args;
        eprintln!("macos lock kit runs on macOS");
        2
    }
}
