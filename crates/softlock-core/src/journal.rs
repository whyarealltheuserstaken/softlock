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
    says_locked: bool,
    flushed: bool,
}

impl Journal {
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            durable: Vec::new(),
            failures: 0,
            says_locked: false,
            flushed: true,
        }
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

    pub fn flush(&mut self) {
        self.durable.clone_from(&self.entries);
        self.flushed = true;
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.durable.clear();
        self.failures = 0;
        self.says_locked = false;
        self.flushed = true;
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.failures == 0 && !self.says_locked
    }

    pub fn failures(&self) -> u32 {
        self.failures
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
