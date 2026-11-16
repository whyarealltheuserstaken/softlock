//! Log file for one scenario. Each tick is flushed so a sleep still leaves a trail.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use crate::logline::LogEvent;
use crate::signals::PowerSignals;
use crate::timefmt::unix_now_ms;

pub struct LogFile {
    file: File,
    scenario: String,
    ended: bool,
}

impl LogFile {
    pub fn create(path: &Path, header: &str) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(path)?;
        file.write_all(header.as_bytes())?;
        file.flush()?;
        let _ = file.sync_data();
        let scenario = header
            .lines()
            .find_map(|line| line.trim().strip_prefix("# scenario="))
            .unwrap_or("unknown")
            .to_string();
        Ok(Self {
            file,
            scenario,
            ended: false,
        })
    }

    pub fn tick(
        &mut self,
        at_ms: u64,
        ui_ok: bool,
        kind: &'static str,
        signals: PowerSignals,
    ) -> io::Result<()> {
        let line = LogEvent::Tick {
            at_ms,
            scenario: self.scenario.clone(),
            ui_ok,
            kind: kind.to_string(),
            signals,
        }
        .format_line();
        self.write_line(&line)
    }

    pub fn end(&mut self, at_ms: u64, reason: &str) -> io::Result<()> {
        if self.ended {
            return Ok(());
        }
        let line = LogEvent::End {
            at_ms,
            scenario: self.scenario.clone(),
            reason: reason.to_string(),
        }
        .format_line();
        self.write_line(&line)?;
        self.ended = true;
        Ok(())
    }

    fn write_line(&mut self, line: &str) -> io::Result<()> {
        writeln!(self.file, "{line}")?;
        self.file.flush()?;
        let _ = self.file.sync_data();
        Ok(())
    }
}

impl Drop for LogFile {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        let line = LogEvent::End {
            at_ms: unix_now_ms(),
            scenario: self.scenario.clone(),
            reason: "dropped".to_string(),
        }
        .format_line();
        let _ = writeln!(self.file, "{line}");
        let _ = self.file.flush();
    }
}
