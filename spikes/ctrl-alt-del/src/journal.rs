use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::policy::PolicyId;
use crate::value::RecordedValue;

pub const JOURNAL_FILE: &str = "journal.json";
pub const JOURNAL_ENV: &str = "SPIKE_CTRL_ALT_DEL_JOURNAL";
pub const WATCHER_LOG: &str = "watcher.log";
pub const RESTORE_RESULT: &str = "restore-result.txt";
const JOURNAL_TMP: &str = "journal.json.tmp";
const LOCK_FILE: &str = "lock";

/// Saved values for every setting this program has changed and not yet restored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    pub deadline_unix_ms: u64,
    pub timeout_secs: u64,
    pub entries: Vec<JournalEntry>,
    /// Policy keys this program created. Restore deletes one only when it is empty.
    #[serde(default)]
    pub created_keys: Vec<String>,
}

/// The value that was there before the first change, and the value written last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub policy: PolicyId,
    pub previous: RecordedValue,
    pub applied: RecordedValue,
}

impl Journal {
    pub fn fresh(now: SystemTime, timeout: Duration) -> Result<Self, Error> {
        Ok(Self {
            version: 1,
            deadline_unix_ms: deadline_millis(now, timeout)?,
            timeout_secs: timeout.as_secs(),
            entries: Vec::new(),
            created_keys: Vec::new(),
        })
    }

    pub fn is_due(&self, now: SystemTime) -> Result<bool, Error> {
        Ok(unix_millis(now)? >= self.deadline_unix_ms)
    }
}

/// Where a journal is stored. `flush` finishes before the caller may change the registry.
pub trait JournalStore {
    fn load(&self) -> Result<Option<Journal>, Error>;
    fn flush(&mut self, journal: &Journal) -> Result<(), Error>;
    fn remove(&mut self) -> Result<(), Error>;
}

/// Journal kept in memory. Tests use this.
#[derive(Debug, Default, Clone)]
pub struct MemoryJournal {
    state: Option<Journal>,
}

impl MemoryJournal {
    pub fn new() -> Self {
        Self::default()
    }
}

impl JournalStore for MemoryJournal {
    fn load(&self) -> Result<Option<Journal>, Error> {
        Ok(self.state.clone())
    }

    fn flush(&mut self, journal: &Journal) -> Result<(), Error> {
        self.state = Some(journal.clone());
        Ok(())
    }

    fn remove(&mut self) -> Result<(), Error> {
        self.state = None;
        Ok(())
    }
}

/// Journal file. The write is flushed to disk before `flush` returns.
#[derive(Debug, Clone)]
pub struct FileJournal {
    directory: PathBuf,
}

impl FileJournal {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    fn path(&self) -> PathBuf {
        self.directory.join(JOURNAL_FILE)
    }

    fn tmp_path(&self) -> PathBuf {
        self.directory.join(JOURNAL_TMP)
    }
}

impl JournalStore for FileJournal {
    fn load(&self) -> Result<Option<Journal>, Error> {
        if self.path().is_file() {
            return read_journal(&self.path());
        }
        if self.tmp_path().is_file() {
            return read_journal(&self.tmp_path());
        }
        Ok(None)
    }

    fn flush(&mut self, journal: &Journal) -> Result<(), Error> {
        fs::create_dir_all(&self.directory)?;
        let tmp = self.tmp_path();
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)?;
            serde_json::to_writer_pretty(&mut file, journal)
                .map_err(|err| Error::new(format!("Could not write {}: {err}", tmp.display())))?;
            file.write_all(b"\n")?;
            file.sync_all()?;
        }
        let path = self.path();
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn remove(&mut self) -> Result<(), Error> {
        for path in [self.path(), self.tmp_path()] {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }
        Ok(())
    }
}

fn read_journal(path: &Path) -> Result<Option<Journal>, Error> {
    let text = fs::read_to_string(path)?;
    let journal: Journal = serde_json::from_str(&text).map_err(|err| {
        Error::new(format!(
            "Could not read {}. {err} Do not delete that file before asking for help.",
            path.display()
        ))
    })?;
    if journal.version != 1 {
        return Err(Error::new(format!(
            "{} is from a different version of this program. Nothing was changed.",
            path.display()
        )));
    }
    Ok(Some(journal))
}

/// Folder for the saved values on a real machine.
/// Journal folder for the detached watcher. A person always uses the default folder.
pub fn watcher_journal_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(JOURNAL_ENV)
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    default_journal_dir()
}

pub fn default_journal_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PROGRAMDATA") {
        return PathBuf::from(dir).join("spike-ctrl-alt-del");
    }
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(dir).join("spike-ctrl-alt-del");
    }
    std::env::temp_dir().join("spike-ctrl-alt-del")
}

/// Exclusive lock around a registry change or a restore.
pub struct FileLock {
    path: PathBuf,
}

impl FileLock {
    pub fn acquire(directory: &Path) -> Result<Self, Error> {
        fs::create_dir_all(directory)?;
        let path = directory.join(LOCK_FILE);
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "{}", std::process::id())?;
                    file.sync_all()?;
                    return Ok(Self { path });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if !lock_owner_alive(&path) {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(err) => return Err(err.into()),
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// A lock is free to take when its owner process is gone. A live owner is never stolen.
fn lock_owner_alive(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Some(pid) = text.trim().parse::<u32>().ok() else {
        return false;
    };
    crate::process::process_is_running(pid)
}

/// Delay before another restore attempt. The delay grows and then stays at 30 seconds.
pub fn retry_delay(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(5);
    Duration::from_secs((1_u64 << shift).min(30))
}

pub fn deadline_millis(now: SystemTime, timeout: Duration) -> Result<u64, Error> {
    let deadline = now
        .checked_add(timeout)
        .ok_or_else(|| Error::new("The timeout is too long."))?;
    unix_millis(deadline)
}

pub fn unix_millis(time: SystemTime) -> Result<u64, Error> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::new("The clock is set before 1970."))?;
    u64::try_from(duration.as_millis()).map_err(|_| Error::new("The clock is too far ahead."))
}

pub fn time_from_millis(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
}

/// How long until `deadline`. Zero when the deadline has already passed.
pub fn remaining(deadline: SystemTime, now: SystemTime) -> Duration {
    deadline.duration_since(now).unwrap_or_default()
}
