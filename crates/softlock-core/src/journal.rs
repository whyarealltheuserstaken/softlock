//! Policy journal. Entries are recorded and flushed before a change is applied.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub target: String,
    pub previous: Option<String>,
    pub new_value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Journal {
    entries: Vec<JournalEntry>,
    durable: Vec<JournalEntry>,
    failures: u32,
    durable_failures: u32,
    /// Milliseconds of backoff a restart should still enforce.
    wait_ms: u64,
    durable_wait_ms: u64,
    says_locked: bool,
    durable_says_locked: bool,
    /// Policies were reverted after a confirmed OS lock. A restart stays
    /// `OsLocked` until the owner is back on the console.
    os_held: bool,
    durable_os_held: bool,
    flushed: bool,
}

impl Journal {
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            durable: Vec::new(),
            failures: 0,
            durable_failures: 0,
            wait_ms: 0,
            durable_wait_ms: 0,
            says_locked: false,
            durable_says_locked: false,
            os_held: false,
            durable_os_held: false,
            flushed: true,
        }
    }

    pub fn mark_os_held(&mut self) {
        self.os_held = true;
    }

    pub fn record(&mut self, entry: JournalEntry) {
        self.entries.push(entry);
        self.flushed = false;
    }

    pub fn mark_locked(&mut self) {
        self.says_locked = true;
        self.flushed = false;
    }

    pub fn set_failures(&mut self, failures: u32) {
        self.failures = failures;
        self.flushed = false;
    }

    pub fn set_wait_ms(&mut self, wait_ms: u64) {
        self.wait_ms = wait_ms;
        self.flushed = false;
    }

    pub fn flush(&mut self) {
        self.durable.clone_from(&self.entries);
        self.durable_failures = self.failures;
        self.durable_wait_ms = self.wait_ms;
        self.durable_says_locked = self.says_locked;
        self.durable_os_held = self.os_held;
        self.flushed = true;
    }

    /// The snapshot a restart would load. Unflushed failure counts and the
    /// locked flag are left behind.
    pub fn persisted(&self) -> Self {
        Self {
            entries: self.durable.clone(),
            durable: self.durable.clone(),
            failures: self.durable_failures,
            durable_failures: self.durable_failures,
            wait_ms: self.durable_wait_ms,
            durable_wait_ms: self.durable_wait_ms,
            says_locked: self.durable_says_locked,
            durable_says_locked: self.durable_says_locked,
            os_held: self.durable_os_held,
            durable_os_held: self.durable_os_held,
            flushed: true,
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.durable.clear();
        self.failures = 0;
        self.durable_failures = 0;
        self.wait_ms = 0;
        self.durable_wait_ms = 0;
        self.says_locked = false;
        self.durable_says_locked = false;
        self.os_held = false;
        self.durable_os_held = false;
        self.flushed = true;
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.failures == 0 && !self.says_locked && !self.os_held
    }

    pub fn os_held(&self) -> bool {
        self.os_held
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    pub fn wait_ms(&self) -> u64 {
        self.wait_ms
    }

    pub fn says_locked(&self) -> bool {
        self.says_locked
    }

    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    pub fn durable_entries(&self) -> &[JournalEntry] {
        &self.durable
    }

    pub fn is_flushed(&self) -> bool {
        self.flushed
    }
}
