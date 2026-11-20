mod measure;
mod notify;
mod service;
mod session;
mod util;

pub fn dispatch(args: &[String]) -> u8 {
    let command = args.first().map(String::as_str).unwrap_or("");
    if command != "service" && refuse_identity() {
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
        "measure-lock" => measure::lock("lock"),
        "measure-self" => measure::own(args),
        "fail-closed" => measure::fail_closed(),
        "install-service" => service::install(args),
        "uninstall-service" => service::uninstall(),
        "service-disconnect" => service::request(args),
        "service" => {
            if refuse_identity() {
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

fn refuse_identity() -> bool {
    match session::school_check() {
        crate::school::SchoolCheck::No => false,
        crate::school::SchoolCheck::Yes => {
            eprintln!("refusing: school workstation");
            true
        }
        crate::school::SchoolCheck::Unreadable => {
            let missing = session::missing_identity_fields();
            if missing.is_empty() {
                eprintln!("refusing: workstation identity could not be read");
            } else {
                eprintln!(
                    "refusing: workstation identity could not be read: {}",
                    missing.join(" ")
                );
            }
            true
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
            | "fail-closed"
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
