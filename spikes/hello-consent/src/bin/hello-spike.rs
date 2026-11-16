//! Proof tool for the Windows Hello consent spike.
//!
//! On Windows it can block the physical keyboard and mouse. It always
//! removes that block by itself. It does not run on other systems.

fn main() {
    // Keep the release marker in this binary.
    std::hint::black_box(softlock_hello_spike::NOT_FOR_RELEASE_MARKER);

    #[cfg(windows)]
    {
        let code = win::run();
        std::process::exit(code);
    }
    #[cfg(not(windows))]
    {
        let _ = std::env::args().len();
        eprintln!("hello-spike runs on Windows only");
        std::process::exit(2);
    }
}

#[cfg(windows)]
#[path = "../win/mod.rs"]
mod win;
