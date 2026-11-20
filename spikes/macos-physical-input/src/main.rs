//! Throwaway m05 physical vs posted input kit. Not linked into product binaries.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod args;
mod classify;
mod grants;
mod guard;
mod plan;
mod report;
mod scrub;
mod target;
mod text;
mod version;

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
                eprintln!("m05 physical-input kit runs on macOS");
                2
            }
        }
    }
}
