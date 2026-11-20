#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|arg| arg == "--owner-session") {
        let code = softlock_app_lib::run_owner_session();
        std::process::exit(code);
    }
    softlock_app_lib::run();
}
