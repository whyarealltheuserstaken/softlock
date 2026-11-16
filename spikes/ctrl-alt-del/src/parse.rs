use std::path::Path;
use std::time::Duration;

use crate::error::Error;
use crate::policy::PolicyId;
use crate::session::DEFAULT_TIMEOUT;

/// Longest safety timer a person can ask for. The icon step uses 20.
pub const MAX_TIMEOUT_MINUTES: u64 = 30;

/// One invocation of the proof tool.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Status,
    Set {
        policies: Vec<PolicyId>,
        timeout: Duration,
    },
    Clear {
        policies: Vec<PolicyId>,
        timeout: Duration,
    },
    Restore,
    Check,
    Watch,
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
    let mut positionals = Vec::new();
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "help" => return Ok(Command::Help),
            "--minutes" | "--timeout-minutes" => {
                let value = rest
                    .next()
                    .ok_or_else(|| Error::new("Add the number of minutes after --minutes."))?;
                timeout = parse_minutes(&value)?;
            }
            other if other.starts_with('-') => {
                return Err(Error::new(format!(
                    "Unknown option {other}. Run spike-ctrl-alt-del help."
                )));
            }
            other => positionals.push(other.to_string()),
        }
    }

    let command = positionals.first().map(String::as_str).unwrap_or("help");
    match command {
        "status" => {
            require_len(&positionals, 1, "status does not take a setting name.")?;
            Ok(Command::Status)
        }
        "restore" => {
            require_len(&positionals, 1, "restore does not take a setting name.")?;
            Ok(Command::Restore)
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
            Ok(Command::Watch)
        }
        "set" => Ok(Command::Set {
            policies: one_policy(&positionals)?,
            timeout,
        }),
        "clear" => Ok(Command::Clear {
            policies: one_policy(&positionals)?,
            timeout,
        }),
        other => Err(Error::new(format!(
            "Unknown command {other}. Run spike-ctrl-alt-del help."
        ))),
    }
}

fn parse_minutes(value: &str) -> Result<Duration, Error> {
    let minutes: u64 = value
        .parse()
        .map_err(|_| Error::new("The timeout must be a whole number of minutes."))?;
    if !(1..=MAX_TIMEOUT_MINUTES).contains(&minutes) {
        return Err(Error::new(format!(
            "The timeout must be between 1 and {MAX_TIMEOUT_MINUTES} minutes."
        )));
    }
    Ok(Duration::from_secs(minutes * 60))
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

/// Command stored for the next sign-in. It always restores the default folder.
pub fn run_once_command(exe: &Path) -> String {
    format!("{} restore", quote_windows_arg(&exe.display().to_string()))
}

/// Quote one Windows command-line argument.
///
/// Backslashes are doubled when they sit in front of a quote, including the
/// closing quote. `CommandLineToArgvW` treats a backslash as an escape only then.
pub fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) && !arg.ends_with('\\') {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }
        if ch == '"' {
            out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
            out.push('"');
            backslashes = 0;
            continue;
        }
        out.extend(std::iter::repeat_n('\\', backslashes));
        backslashes = 0;
        out.push(ch);
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}
