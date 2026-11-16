//! Throwaway desktop probe. Not linked into product binaries.

mod bmp;
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
        eprintln!("desktop proof runs on Windows");
        2
    }
}
