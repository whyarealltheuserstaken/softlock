use std::time::{Duration, SystemTime};

use crate::error::Error;
use crate::journal::{Journal, JournalStore, deadline_millis, time_from_millis};
use crate::policy::PolicyId;
use crate::value::RecordedValue;

/// Safety timer used when a caller does not pass one.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Reads and writes one policy value.
pub trait PolicyStore {
    fn read(&self, policy: PolicyId) -> Result<RecordedValue, Error>;
    fn apply(&mut self, policy: PolicyId, value: &RecordedValue) -> Result<(), Error>;

    /// The key id when that policy's key is not there yet.
    fn missing_key_id(&self, _policy: PolicyId) -> Result<Option<String>, Error> {
        Ok(None)
    }

    /// Deletes a key this program created, and only when the key is empty.
    fn delete_key_if_empty(&mut self, _key_id: &str) -> Result<(), Error> {
        Ok(())
    }
}

/// Extra safety net armed after the journal is flushed and before the registry changes.
pub trait SafetyNet {
    fn arm(&mut self) -> Result<(), Error>;
    fn disarm(&mut self) -> Result<(), Error>;
}

/// Registry stand-in used by tests and by the read-back proof.
#[derive(Debug, Default)]
pub struct MemoryStore {
    values: std::collections::HashMap<PolicyId, RecordedValue>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PolicyStore for MemoryStore {
    fn read(&self, policy: PolicyId) -> Result<RecordedValue, Error> {
        Ok(self
            .values
            .get(&policy)
            .cloned()
            .unwrap_or(RecordedValue::Absent))
    }

    fn apply(&mut self, policy: PolicyId, value: &RecordedValue) -> Result<(), Error> {
        match value {
            RecordedValue::Absent => {
                self.values.remove(&policy);
            }
            present => {
                self.values.insert(policy, present.clone());
            }
        }
        Ok(())
    }
}

/// No extra safety net. Tests use this.
#[derive(Debug, Default)]
pub struct NoSafety;

impl SafetyNet for NoSafety {
    fn arm(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn disarm(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

/// What the registry was asked to hold, and what a read returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadBack {
    pub policy: PolicyId,
    pub previous: RecordedValue,
    pub expected: RecordedValue,
    pub actual: RecordedValue,
    pub warnings: Vec<String>,
}

impl ReadBack {
    pub fn matches(&self) -> bool {
        self.expected == self.actual
    }
}

/// A change was applied, or the safety timer had already put the values back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    Applied(ReadBack),
    ExpiredAndRestored(Vec<ReadBack>),
}

/// Applies, clears, and restores policy values through a journal.
pub struct Session<'a, S, J, N> {
    store: &'a mut S,
    journal_store: J,
    safety: N,
    timeout: Duration,
    state: Option<Journal>,
}

impl<'a, S, J, N> Session<'a, S, J, N>
where
    S: PolicyStore,
    J: JournalStore,
    N: SafetyNet,
{
    pub fn open(
        store: &'a mut S,
        journal_store: J,
        safety: N,
        timeout: Duration,
    ) -> Result<Self, Error> {
        let state = journal_store.load()?;
        Ok(Self {
            store,
            journal_store,
            safety,
            timeout,
            state,
        })
    }

    pub fn has_journal(&self) -> bool {
        self.state.is_some()
    }

    pub fn deadline(&self) -> Option<SystemTime> {
        self.state
            .as_ref()
            .map(|journal| time_from_millis(journal.deadline_unix_ms))
    }

    pub fn journal(&self) -> Option<&Journal> {
        self.state.as_ref()
    }

    pub fn set(&mut self, policy: PolicyId, now: SystemTime) -> Result<Mutation, Error> {
        self.mutate(policy, now, RecordedValue::dword(1))
    }

    pub fn clear(&mut self, policy: PolicyId, now: SystemTime) -> Result<Mutation, Error> {
        self.mutate(policy, now, RecordedValue::Absent)
    }

    pub fn restore_if_due(&mut self, now: SystemTime) -> Result<Option<Vec<ReadBack>>, Error> {
        if !self.is_due(now)? {
            return Ok(None);
        }
        Ok(Some(self.restore_all()?))
    }

    /// Puts every saved previous value back, including values that were absent.
    pub fn restore_all(&mut self) -> Result<Vec<ReadBack>, Error> {
        let Some(state) = self.state.clone() else {
            self.safety.disarm()?;
            return Ok(Vec::new());
        };

        let mut reports = Vec::with_capacity(state.entries.len());
        for entry in &state.entries {
            reports.push(confirm_write(self.store, entry.policy, &entry.previous)?);
        }
        for key_id in &state.created_keys {
            self.store.delete_key_if_empty(key_id)?;
        }

        self.journal_store.remove()?;
        self.state = None;
        if let Err(err) = self.safety.disarm()
            && let Some(last) = reports.last_mut()
        {
            last.warnings
                .push(format!("Could not remove the next-sign-in restore: {err}"));
        }
        Ok(reports)
    }

    fn mutate(
        &mut self,
        policy: PolicyId,
        now: SystemTime,
        new_value: RecordedValue,
    ) -> Result<Mutation, Error> {
        if self.is_due(now)? {
            return Ok(Mutation::ExpiredAndRestored(self.restore_all()?));
        }

        let mut state = match self.state.clone() {
            Some(state) => state,
            None => Journal::fresh(now, self.timeout)?,
        };
        if let Some(entry) = state
            .entries
            .iter_mut()
            .find(|entry| entry.policy == policy)
        {
            entry.applied = new_value.clone();
        } else {
            let previous = self.store.read(policy)?;
            state.entries.push(crate::journal::JournalEntry {
                policy,
                previous,
                applied: new_value.clone(),
            });
        }
        if !matches!(new_value, RecordedValue::Absent)
            && let Some(key_id) = self.store.missing_key_id(policy)?
            && !state
                .created_keys
                .iter()
                .any(|existing| existing == &key_id)
        {
            state.created_keys.push(key_id);
        }
        state.deadline_unix_ms = deadline_millis(now, self.timeout)?;
        state.timeout_secs = self.timeout.as_secs();

        // The saved previous value is on disk before the registry changes.
        self.journal_store.flush(&state)?;
        self.state = Some(state.clone());

        let mut warnings = Vec::new();
        if let Err(err) = self.safety.arm() {
            warnings.push(format!(
                "Could not schedule a restore at the next sign-in: {err}"
            ));
        }

        self.store.apply(policy, &new_value)?;
        let actual = self.store.read(policy)?;
        let previous = state
            .entries
            .iter()
            .find(|entry| entry.policy == policy)
            .map(|entry| entry.previous.clone())
            .ok_or_else(|| Error::new("The saved settings lost a value."))?;
        if actual != new_value {
            return Err(Error::new(format!(
                "{policy}: expected {new_value} after the change, but read back {actual}. The saved value is still {previous}. Run restore."
            )));
        }
        Ok(Mutation::Applied(ReadBack {
            policy,
            previous,
            expected: new_value,
            actual,
            warnings,
        }))
    }

    fn is_due(&self, now: SystemTime) -> Result<bool, Error> {
        let Some(state) = &self.state else {
            return Ok(false);
        };
        state.is_due(now)
    }
}

/// Every policy value currently in the store.
pub fn snapshot<S: PolicyStore>(store: &S) -> Result<Vec<(PolicyId, RecordedValue)>, Error> {
    PolicyId::ALL
        .into_iter()
        .map(|policy| Ok((policy, store.read(policy)?)))
        .collect()
}

/// Writes one value and reads it back.
pub fn confirm_write<S: PolicyStore>(
    store: &mut S,
    policy: PolicyId,
    value: &RecordedValue,
) -> Result<ReadBack, Error> {
    store.apply(policy, value)?;
    let actual = store.read(policy)?;
    if actual != *value {
        return Err(Error::new(format!(
            "Could not put {policy} back. Expected {value}, read back {actual}. Run restore again."
        )));
    }
    Ok(ReadBack {
        policy,
        previous: value.clone(),
        expected: value.clone(),
        actual,
        warnings: Vec::new(),
    })
}

/// Writes a snapshot back and reads each value to confirm it.
pub fn restore_snapshot<S: PolicyStore>(
    store: &mut S,
    values: &[(PolicyId, RecordedValue)],
) -> Result<(), Error> {
    for (policy, value) in values {
        confirm_write(store, *policy, value)?;
    }
    Ok(())
}
