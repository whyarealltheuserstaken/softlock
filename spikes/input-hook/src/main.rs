//! Throwaway input-hook probe. Not linked into product binaries.

mod guard;
mod protocol;
mod verdict;

#[cfg(windows)]
mod win;

fn main() {
    let code = dispatch();
    std::process::exit(code);
}

fn dispatch() -> i32 {
    #[cfg(windows)]
    {
        win::entry()
    }
    #[cfg(not(windows))]
    {
        eprintln!("input hook proof runs on Windows");
        2
    }
}
