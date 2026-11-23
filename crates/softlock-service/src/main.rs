//! Service process. `--print-names` reports the name constants. `--foreground`
//! serves the pipe in this session. With no flag the process asks the Service
//! Control Manager to start it.

fn main() {
    #[cfg(feature = "simulated-physical")]
    {
        let _ = softlock_service::marker_bytes();
        let _ = softlock_service::present_physical_marker_bytes();
    }
    #[cfg(feature = "live-fail-closed")]
    {
        let _ = softlock_service::live_fail_closed_marker_bytes();
        let _ = softlock_service::present_physical_marker_bytes();
    }
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--print-names") => {
            println!("name={}", softlock_core::SERVICE_NAME);
            println!("display={}", softlock_core::SERVICE_DISPLAY_NAME);
            println!("pipe={}", softlock_core::pipe_name());
        }
        Some("--foreground") => {
            let code = softlock_service::run_foreground();
            std::process::exit(code);
        }
        Some("--repair-settings") => {
            #[cfg(windows)]
            {
                let code = softlock_service::repair_main();
                std::process::exit(code);
            }
            #[cfg(not(windows))]
            {
                eprintln!("the service runs on Windows");
                std::process::exit(1);
            }
        }
        Some("--session-probe") => {
            let path = args.next().unwrap_or_default();
            let code = softlock_service::run_session_probe(&path);
            std::process::exit(code);
        }
        _ => {
            #[cfg(windows)]
            {
                let code = softlock_service::run_service();
                std::process::exit(code);
            }
            #[cfg(not(windows))]
            {
                // A macOS smoke launch of the product binary has nothing to do.
                eprintln!("the service runs on Windows");
                std::process::exit(0);
            }
        }
    }
}
