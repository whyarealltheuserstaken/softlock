//! Throwaway macOS desktop probe. Not linked into product binaries.

mod args;
mod classify;
mod guard;
mod labels;
mod launchctl;
mod plist;
mod png;
mod report;
mod smoke;
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
        eprintln!("macos desktop proof runs on macOS");
        2
    }
}
