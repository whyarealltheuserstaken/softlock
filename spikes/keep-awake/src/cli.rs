//! Command line for the four measurement runs, the practice minute, and the analyzer.

use std::path::{Path, PathBuf};

use softlock_core::PRODUCT_DISPLAY_NAME;

use crate::plan::OverlayLook;
use crate::scenario::{MEASUREMENT_SCENARIOS, Scenario, parse_scenario};

pub const PACKAGE: &str = "softlock-spike-keep-awake";
pub const LOG_DIR: &str = "spikes/keep-awake/logs";

pub fn run_command(id: &str) -> String {
    format!("cargo run -p {PACKAGE} -- run --scenario {id}")
}

pub fn practice_command() -> String {
    format!("cargo run -p {PACKAGE} -- run --scenario practice --minutes 1")
}

pub fn analyze_command() -> String {
    format!("cargo run -p {PACKAGE} -- analyze")
}

pub fn smoke_command() -> String {
    format!("cargo run -p {PACKAGE} -- smoke")
}

pub fn usage() -> String {
    let mut runs = String::new();
    for id in MEASUREMENT_SCENARIOS {
        runs.push_str("  ");
        runs.push_str(&run_command(id));
        runs.push('\n');
    }
    format!(
        "{name} keep-awake spike\n\
         \n\
         Commands, from the repository root:\n\
         {practice}\n\
         {runs}\
         {analyze}\n\
         {smoke}\n\
         \n\
         Options for run:\n\
           --minutes <1-180>       length of the run (60 for the four tests, 1 for practice)\n\
           --interval-secs <1-60>  seconds between log lines (default 5)\n\
           --black                 opaque black overlay (the default is dim)\n\
           --allow-no-modern-standby\n\
                                   start a long test even if Modern Standby is not reported\n\
           --log-dir <path>        where to write logs (default {LOG_DIR})\n\
         \n\
         Each log line has a UTC timestamp, a small UI-automation result, the battery\n\
         percent, and thermal / power-throttling signals. Logs go to {LOG_DIR}/<scenario>.log.\n",
        name = PRODUCT_DISPLAY_NAME,
        practice = practice_command(),
        analyze = analyze_command(),
        smoke = smoke_command(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    pub scenario: Scenario,
    pub duration_secs: u64,
    pub interval_secs: u64,
    pub look: OverlayLook,
    pub allow_no_modern_standby: bool,
    pub log_dir: PathBuf,
}

impl RunSpec {
    pub fn smoke(log_dir: PathBuf) -> Self {
        Self {
            scenario: Scenario::Smoke,
            duration_secs: 6,
            interval_secs: 1,
            look: OverlayLook::Dim,
            allow_no_modern_standby: true,
            log_dir,
        }
    }

    pub fn log_path(&self) -> PathBuf {
        self.log_dir.join(format!("{}.log", self.scenario.id()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(RunSpec),
    Analyze { dir: PathBuf },
    Smoke,
}

pub fn parse_args<I, S>(args: I, cwd: &Path) -> Result<Parsed, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect();
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(Parsed::Help);
    }
    match args[0].as_str() {
        "run" => parse_run(&args[1..], cwd).map(Parsed::Run),
        "analyze" => parse_analyze(&args[1..], cwd),
        "smoke" => {
            if args.len() == 1 {
                Ok(Parsed::Smoke)
            } else {
                Err(format!("smoke takes no options.\n\n{}", usage()))
            }
        }
        other => Err(format!("Unknown command \"{other}\".\n\n{}", usage())),
    }
}

fn parse_run(args: &[String], cwd: &Path) -> Result<RunSpec, String> {
    let mut scenario: Option<Scenario> = None;
    let mut minutes: Option<u64> = None;
    let mut interval: Option<u64> = None;
    let mut look = OverlayLook::Dim;
    let mut allow_no_modern_standby = false;
    let mut log_dir = None;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "--scenario" => {
                let value = need_value(args, &mut index, "--scenario")?;
                scenario = Some(parse_scenario(&value)?);
            }
            "--minutes" => {
                let value = need_value(args, &mut index, "--minutes")?;
                let parsed: u64 = value.parse().map_err(|_| {
                    format!("Minutes must be a number from 1 to 180, not \"{value}\".")
                })?;
                if !(1..=180).contains(&parsed) {
                    return Err("Minutes has to be from 1 to 180.".to_string());
                }
                minutes = Some(parsed);
            }
            "--interval-secs" => {
                let value = need_value(args, &mut index, "--interval-secs")?;
                let parsed: u64 = value.parse().map_err(|_| {
                    format!("Interval must be a number from 1 to 60, not \"{value}\".")
                })?;
                if !(1..=60).contains(&parsed) {
                    return Err("Interval has to be from 1 to 60 seconds.".to_string());
                }
                interval = Some(parsed);
            }
            "--log-dir" => {
                let value = need_value(args, &mut index, "--log-dir")?;
                log_dir = Some(PathBuf::from(value));
            }
            "--black" => look = OverlayLook::Black,
            "--allow-no-modern-standby" => allow_no_modern_standby = true,
            other => {
                return Err(format!("Unknown option \"{other}\".\n\n{}", usage()));
            }
        }
        index += 1;
    }
    let Some(scenario) = scenario else {
        return Err(format!("Name the scenario with --scenario.\n\n{}", usage()));
    };
    let minutes = minutes.unwrap_or_else(|| scenario.default_minutes());
    Ok(RunSpec {
        scenario,
        duration_secs: minutes * 60,
        interval_secs: interval.unwrap_or(5),
        look,
        allow_no_modern_standby,
        log_dir: log_dir.unwrap_or_else(|| cwd.join(LOG_DIR)),
    })
}

fn parse_analyze(args: &[String], cwd: &Path) -> Result<Parsed, String> {
    match args {
        [] => Ok(Parsed::Analyze {
            dir: cwd.join(LOG_DIR),
        }),
        [path] => Ok(Parsed::Analyze {
            dir: PathBuf::from(path),
        }),
        _ => Err(format!(
            "analyze takes an optional log folder and nothing else.\n\n{}",
            usage()
        )),
    }
}

fn need_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    let next = args
        .get(*index + 1)
        .ok_or_else(|| format!("{flag} needs a value."))?;
    *index += 1;
    Ok(next.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::MEASUREMENT_SCENARIOS;

    #[test]
    fn checklist_commands_are_the_four_sixty_minute_runs() {
        let cwd = Path::new("/repo");
        for id in MEASUREMENT_SCENARIOS {
            let command = run_command(id);
            let args: Vec<String> = command
                .split_whitespace()
                .skip(5)
                .map(str::to_string)
                .collect();
            // `cargo run -p <pkg> --` is the prefix. What remains is `run --scenario <id>`.
            let parsed = parse_args(args, cwd).unwrap();
            let Parsed::Run(spec) = parsed else {
                panic!("{id} did not parse as a run");
            };
            assert_eq!(spec.scenario.id(), id);
            assert_eq!(spec.duration_secs, 3600);
            assert_eq!(spec.interval_secs, 5);
            assert_eq!(spec.look, OverlayLook::Dim);
            assert!(!spec.allow_no_modern_standby);
            assert_eq!(spec.log_path(), cwd.join(LOG_DIR).join(format!("{id}.log")));
        }
    }

    #[test]
    fn practice_defaults_to_one_minute_and_can_be_set_explicitly() {
        let cwd = Path::new(".");
        let parsed = parse_args(["run", "--scenario", "practice"], cwd).unwrap();
        let Parsed::Run(spec) = parsed else {
            panic!("practice")
        };
        assert_eq!(spec.duration_secs, 60);
        let parsed = parse_args(practice_command().split_whitespace().skip(5), cwd).unwrap();
        let Parsed::Run(spec) = parsed else {
            panic!("practice command")
        };
        assert_eq!(spec.duration_secs, 60);
        assert_eq!(spec.scenario, Scenario::Practice);
    }

    #[test]
    fn analyze_and_smoke_and_help() {
        let cwd = Path::new("/repo");
        let parsed = parse_args(["analyze"], cwd).unwrap();
        assert_eq!(
            parsed,
            Parsed::Analyze {
                dir: cwd.join(LOG_DIR)
            }
        );
        assert!(matches!(parse_args(["smoke"], cwd).unwrap(), Parsed::Smoke));
        assert!(matches!(parse_args(["--help"], cwd).unwrap(), Parsed::Help));
        assert!(parse_args(["run"], cwd).is_err());
        assert!(parse_args(["run", "--scenario", "desk"], cwd).is_err());
        assert!(
            parse_args(
                ["run", "--scenario", "lid-open-plugged", "--minutes", "0"],
                cwd
            )
            .is_err()
        );
    }

    #[test]
    fn usage_lists_the_measurement_commands() {
        let text = usage();
        for id in MEASUREMENT_SCENARIOS {
            assert!(text.contains(&run_command(id)), "{id}");
        }
        assert!(text.contains(&practice_command()));
        assert!(text.contains(&analyze_command()));
        assert!(text.contains("battery"));
        assert!(text.contains("throttl"));
    }
}
