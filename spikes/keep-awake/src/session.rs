//! Starts a run. The Windows host lives in `win`. Other systems refuse.

use crate::cli::{RunSpec, analyze_command};
use crate::plan::format_duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub ticks: usize,
    pub overlay_shown: bool,
    pub power_held: bool,
    pub stop: String,
    pub log_path: std::path::PathBuf,
}

pub enum SessionError {
    #[cfg_attr(windows, allow(dead_code))]
    NotWindows,
    #[cfg_attr(not(windows), allow(dead_code))]
    Failed(String),
}

impl SessionError {
    pub fn code(&self) -> u8 {
        match self {
            SessionError::NotWindows => 2,
            SessionError::Failed(_) => 1,
        }
    }

    pub fn message(&self) -> String {
        match self {
            SessionError::NotWindows => "This proof runs on Windows.".to_string(),
            SessionError::Failed(message) => message.clone(),
        }
    }
}

pub fn run(spec: &RunSpec) -> Result<RunReport, SessionError> {
    #[cfg(windows)]
    {
        crate::win::run(spec).map_err(SessionError::Failed)
    }
    #[cfg(not(windows))]
    {
        let _ = spec;
        Err(SessionError::NotWindows)
    }
}

pub fn smoke() -> Result<(), SessionError> {
    #[cfg(windows)]
    {
        crate::win::smoke().map_err(SessionError::Failed)
    }
    #[cfg(not(windows))]
    {
        Err(SessionError::NotWindows)
    }
}

pub fn print_finished(spec: &RunSpec, report: &RunReport) {
    println!("Log: {}", report.log_path.display());
    println!(
        "Stop: {}. Samples: {}. Planned length: {}.",
        report.stop,
        report.ticks,
        format_duration(spec.duration_secs)
    );
    if spec.scenario.is_measurement() {
        println!("After all four tests, run: {}", analyze_command());
        println!(
            "Copy the lowest battery, the heat line, and the policy or cap line into the checklist table."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::RunSpec;
    use std::path::PathBuf;

    #[test]
    fn non_windows_hosts_do_not_pretend_to_run() {
        if cfg!(windows) {
            return;
        }
        let error = run(&RunSpec::smoke(PathBuf::from("unused"))).unwrap_err();
        assert!(matches!(error, SessionError::NotWindows));
        assert_eq!(error.code(), 2);
    }
}
