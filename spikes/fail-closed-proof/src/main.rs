//! Throwaway fail-closed probe. Not linked into product binaries.

mod guard;
mod journal;
mod protocol;
mod report;
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
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.iter().any(|arg| arg == "--revert") {
            eprintln!("fail-closed proof restores policies on Windows");
            return 0;
        }
        eprintln!("fail-closed proof runs on Windows");
        2
    }
}
