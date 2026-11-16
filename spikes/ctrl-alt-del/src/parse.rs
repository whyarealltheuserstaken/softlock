use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::Error;
use crate::journal::default_journal_dir;
use crate::policy::PolicyId;
use crate::session::DEFAULT_TIMEOUT;

/// One invocation of the proof tool.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Status {
        journal: PathBuf,
    },
    Set {
        policies: Vec<PolicyId>,
        timeout: Duration,
        journal: PathBuf,
    },
    Clear {
        policies: Vec<PolicyId>,
        timeout: Duration,
        journal: PathBuf,
    },
    Restore {
        journal: PathBuf,
    },
    Check,
    Watch {
        journal: PathBuf,
    },
}

pub fn parse_args<I, T>(args: I) -> Result<Command, Error>
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect();
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(Command::Help);
    }

    let mut timeout = DEFAULT_TIMEOUT;
    let mut journal = None;
    let mut positionals = Vec::new();
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "help" => return Ok(Command::Help),
            "--timeout-minutes" => {
                let value = rest.next().ok_or_else(|| {
                    Error::new("Add the number of minutes after --timeout-minutes.")
                })?;
                let minutes: u64 = value
                    .parse()
                    .map_err(|_| Error::new("The timeout must be a whole number of minutes."))?;
                if !(1..=120).contains(&minutes) {
                    return Err(Error::new("The timeout must be between 1 and 120 minutes."));
                }
                timeout = Duration::from_secs(minutes * 60);
            }
            "--journal" => {
                let value = rest
                    .next()
                    .ok_or_else(|| Error::new("Add the folder after --journal."))?;
                journal = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') => {
                return Err(Error::new(format!(
                    "Unknown option {other}. Run spike-ctrl-alt-del help."
                )));
            }
            other => positionals.push(other.to_string()),
        }
    }

    let journal = journal.unwrap_or_else(default_journal_dir);
    let command = positionals.first().map(String::as_str).unwrap_or("help");
    match command {
        "status" => {
            require_len(&positionals, 1, "status does not take a setting name.")?;
            Ok(Command::Status { journal })
        }
        "restore" => {
            require_len(&positionals, 1, "restore does not take a setting name.")?;
            Ok(Command::Restore { journal })
        }
        "check" => {
            require_len(&positionals, 1, "check does not take a setting name.")?;
            if timeout != DEFAULT_TIMEOUT {
                return Err(Error::new("check does not take a timeout."));
            }
            Ok(Command::Check)
        }
        "watch" => {
            require_len(&positionals, 1, "watch does not take a setting name.")?;
            Ok(Command::Watch { journal })
        }
        "set" => Ok(Command::Set {
            policies: one_policy(&positionals)?,
            timeout,
            journal,
        }),
        "clear" => Ok(Command::Clear {
            policies: one_policy(&positionals)?,
            timeout,
            journal,
        }),
        other => Err(Error::new(format!(
            "Unknown command {other}. Run spike-ctrl-alt-del help."
        ))),
    }
}

fn require_len(positionals: &[String], len: usize, message: &str) -> Result<(), Error> {
    if positionals.len() == len {
        Ok(())
    } else {
        Err(Error::new(message))
    }
}

fn one_policy(positionals: &[String]) -> Result<Vec<PolicyId>, Error> {
    let Some(name) = positionals.get(1) else {
        return Err(Error::new(
            "Say which setting, or all. Example: set DisableTaskMgr",
        ));
    };
    if positionals.len() != 2 {
        return Err(Error::new(
            "Say one setting, or all. Example: set DisableTaskMgr",
        ));
    }
    if name.eq_ignore_ascii_case("all") {
        return Ok(PolicyId::ALL.to_vec());
    }
    Ok(vec![PolicyId::parse(name)?])
}

/// Command stored for the next sign-in, with the program path in quotes.
pub fn run_once_command(exe: &Path) -> String {
    format!("\"{}\" restore", exe.display())
}

/// Quote one Windows command-line argument.
pub fn quote_windows_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    if !arg.contains(' ') && !arg.contains('"') {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    for ch in arg.chars() {
        if ch == '"' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}
