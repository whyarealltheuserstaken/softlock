//! Throwaway probe. Not linked into product binaries.

mod verdict;

#[cfg(windows)]
mod win;

use std::env;
use std::process::ExitCode;

use serde::Serialize;

use verdict::Pick;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("");
    if command == "decide" {
        return ExitCode::from(decide_cli(&args[1..]));
    }
    dispatch_os(&args)
}

#[cfg(windows)]
fn dispatch_os(args: &[String]) -> ExitCode {
    ExitCode::from(win::dispatch(args))
}

#[cfg(not(windows))]
fn dispatch_os(args: &[String]) -> ExitCode {
    let _ = args;
    eprintln!("os lock proof runs on Windows");
    ExitCode::from(2)
}

fn decide_cli(args: &[String]) -> u8 {
    let lock = match flag(args, "--lock") {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let service = match flag(args, "--service") {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let own = match flag(args, "--own") {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let pick = verdict::pick(lock, service, own);
    print_json(&Decision::from(pick));
    0
}

fn flag(args: &[String], name: &str) -> Result<bool, String> {
    let mut values = args.iter();
    while let Some(arg) = values.next() {
        if arg == name {
            let Some(value) = values.next() else {
                return Err(format!("missing value for {name}"));
            };
            return match value.as_str() {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(format!("{name} wants true or false")),
            };
        }
    }
    Err(format!("missing {name}"))
}

#[derive(Serialize)]
struct Decision {
    primary: &'static str,
    fallback: &'static str,
    lock_workstation: bool,
    service_disconnect: bool,
    own_session: bool,
}

impl From<Pick> for Decision {
    fn from(pick: Pick) -> Self {
        Self {
            primary: pick.primary,
            fallback: pick.fallback,
            lock_workstation: pick.lock_workstation,
            service_disconnect: pick.service_disconnect,
            own_session: pick.own_session,
        }
    }
}

fn print_json(value: &impl Serialize) -> bool {
    match serde_json::to_string(value) {
        Ok(line) => {
            println!("{line}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            true
        }
        Err(error) => {
            eprintln!("json: {error}");
            false
        }
    }
}

#[cfg(windows)]
pub(crate) use print_json as emit;
