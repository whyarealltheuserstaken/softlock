mod measure;
mod notify;
mod service;
mod session;
mod util;

pub fn dispatch(args: &[String]) -> u8 {
    let command = args.first().map(String::as_str).unwrap_or("");
    if command != "service" && session::school_workstation() {
        eprintln!("refusing: school workstation");
        return 2;
    }
    if needs_authorization(command) && !authorized(args) {
        eprintln!("refusing without GITHUB_ACTIONS or --allow-local");
        return 2;
    }
    match command {
        "preflight" => session::preflight(),
        "status" => session::status(),
        "heartbeat" => heartbeat(args),
        "measure-blocked" => measure::lock("blocked"),
        "measure-lock" => measure::lock("confirm"),
        "measure-self" => measure::own(args),
        "stay-disconnected" => measure::stay(),
        "install-service" => service::install(args),
        "uninstall-service" => service::uninstall(),
        "service-disconnect" => service::request(args),
        "service" => {
            if session::school_workstation() {
                eprintln!("refusing: school workstation");
                return 2;
            }
            service::run()
        }
        _ => {
            eprintln!("unknown command");
            2
        }
    }
}

fn authorized(args: &[String]) -> bool {
    util::has_flag(args, "--allow-local")
        || std::env::var("GITHUB_ACTIONS").ok().as_deref() == Some("true")
}

fn needs_authorization(command: &str) -> bool {
    matches!(
        command,
        "heartbeat"
            | "measure-blocked"
            | "measure-lock"
            | "measure-self"
            | "stay-disconnected"
            | "install-service"
            | "service-disconnect"
    )
}

fn heartbeat(args: &[String]) -> u8 {
    let path = match util::flag_path(args, "--file") {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    loop {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{millis}");
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}
