//! Dev launch. Compiled only with the `dev-launch` feature, never into a
//! release build. The PIN is typed on the lock screen; it is not an argument.

fn main() {
    let _ = softlock_lockhost::dev_launch_marker_bytes();
    let hash = std::env::args().nth(1).unwrap_or_default();
    if hash.is_empty() {
        eprintln!("usage: softlock-lockhost <argon2id-phc>");
        std::process::exit(2);
    }
    if std::env::args().len() > 2 {
        eprintln!("usage: softlock-lockhost <argon2id-phc>");
        std::process::exit(2);
    }
    #[cfg(windows)]
    {
        let config = softlock_lockhost::DevConfig {
            pin_hash: hash,
            while_arming: None,
        };
        match softlock_lockhost::DevLock::arm(config) {
            Ok(lock) => {
                while lock.is_locked() {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
            Err(error) => {
                eprintln!("dev launch failed: {error}");
                std::process::exit(1);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hash;
        eprintln!("dev launch runs on Windows");
        std::process::exit(1);
    }
}
