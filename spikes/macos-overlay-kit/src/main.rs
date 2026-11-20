//! Throwaway m06 overlay kit. Not linked into product binaries.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod args;
mod claim;
mod report;
mod scrub;
mod text;
mod version;
mod wait;

#[cfg(target_os = "macos")]
mod mac;

fn main() {
    let code = dispatch();
    std::process::exit(code);
}

fn dispatch() -> i32 {
    #[cfg(target_os = "macos")]
    {
        mac::entry()
    }
    #[cfg(not(target_os = "macos"))]
    {
        match crate::args::parse_env() {
            Err(error) => {
                eprintln!("{error}");
                2
            }
            Ok(_) => {
                eprintln!("m06 overlay kit runs on macOS");
                2
            }
        }
    }
}
