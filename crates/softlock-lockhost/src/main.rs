//! Lock host process. The service spawns it with `--service-host` and two
//! inherited pipe handles. The PIN and the nonce are not command-line arguments.

fn main() {
    #[cfg(feature = "dev-launch")]
    let _ = softlock_lockhost::dev_launch_marker_bytes();
    #[cfg(feature = "live-fail-closed")]
    let _ = softlock_lockhost::live_fail_closed_marker_bytes();
    #[cfg(windows)]
    {
        let code = softlock_lockhost::run_service_host();
        std::process::exit(code);
    }
    #[cfg(not(windows))]
    {
        // A macOS smoke launch of the product binary has nothing to do.
        eprintln!("the lock host runs on Windows");
        std::process::exit(0);
    }
}
