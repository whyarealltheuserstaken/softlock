use std::time::{Duration, SystemTime};

use crate::error::Error;
use crate::journal::{Journal, JournalStore, MemoryJournal, deadline_millis, time_from_millis};
use crate::policy::PolicyId;
use crate::value::RecordedValue;

/// Safety timer used when a caller does not pass one.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Reads and writes one policy value.
pub trait PolicyStore {
    fn read(&self, policy: PolicyId) -> Result<RecordedValue, Error>;
    fn apply(&mut self, policy: PolicyId, value: &RecordedValue) -> Result<(), Error>;
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
            self.store.apply(entry.policy, &entry.previous)?;
            let actual = self.store.read(entry.policy)?;
            if actual != entry.previous {
                return Err(Error::new(format!(
                    "Could not put {} back. Expected {}, read back {}. Run restore again.",
                    entry.policy, entry.previous, actual
                )));
            }
            reports.push(ReadBack {
                policy: entry.policy,
                previous: entry.previous.clone(),
                expected: entry.previous.clone(),
                actual,
                warnings: Vec::new(),
            });
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

/// Writes a snapshot back and reads each value to confirm it.
pub fn restore_snapshot<S: PolicyStore>(
    store: &mut S,
    values: &[(PolicyId, RecordedValue)],
) -> Result<(), Error> {
    for (policy, value) in values {
        store.apply(*policy, value)?;
        let actual = store.read(*policy)?;
        if actual != *value {
            return Err(Error::new(format!(
                "Could not put {policy} back. Expected {value}, read back {actual}."
            )));
        }
    }
    Ok(())
}

/// Sets, clears, and restores every policy, then checks the read-back.
///
/// The store is returned to the values it had when this function started.
pub fn prove_exact_restore<S: PolicyStore>(store: &mut S) -> Result<(), Error> {
    let originals = snapshot(store)?;
    let result = run_proof(store);
    restore_snapshot(store, &originals)?;
    let after = snapshot(store)?;
    if after != originals {
        return Err(Error::new(
            "After the proof, a read-back did not match the saved values.",
        ));
    }
    result
}

fn run_proof<S: PolicyStore>(store: &mut S) -> Result<(), Error> {
    for policy in PolicyId::ALL {
        prove_absent(store, policy)?;
        prove_dword_round_trip(store, policy, 0)?;
        prove_dword_round_trip(store, policy, 7)?;
        prove_raw_round_trip(store, policy, &RecordedValue::reg_sz("keep"))?;
    }
    prove_restore_everything(store)?;
    Ok(())
}

fn prove_absent<S: PolicyStore>(store: &mut S, policy: PolicyId) -> Result<(), Error> {
    store.apply(policy, &RecordedValue::Absent)?;
    let now = proof_now();
    let restored = {
        let mut session = Session::open(store, MemoryJournal::new(), NoSafety, DEFAULT_TIMEOUT)?;
        let applied = expect_applied(session.set(policy, now)?)?;
        expect_readback(&applied, &RecordedValue::dword(1))?;
        session.restore_all()?
    };
    expect_one(&restored, policy, &RecordedValue::Absent)?;
    expect_value(store, policy, &RecordedValue::Absent)
}

fn prove_dword_round_trip<S: PolicyStore>(
    store: &mut S,
    policy: PolicyId,
    previous: u32,
) -> Result<(), Error> {
    let planted = RecordedValue::dword(previous);
    store.apply(policy, &planted)?;
    let now = proof_now();
    let restored = {
        let mut session = Session::open(store, MemoryJournal::new(), NoSafety, DEFAULT_TIMEOUT)?;
        let applied = expect_applied(session.set(policy, now)?)?;
        expect_readback(&applied, &RecordedValue::dword(1))?;
        if previous == 0 {
            let cleared = expect_applied(session.clear(policy, now)?)?;
            expect_readback(&cleared, &RecordedValue::Absent)?;
            if cleared.previous != planted {
                return Err(Error::new(format!(
                    "{policy}: clear forgot the saved value {}.",
                    cleared.previous
                )));
            }
        }
        session.restore_all()?
    };
    expect_one(&restored, policy, &planted)?;
    expect_value(store, policy, &planted)
}

fn prove_raw_round_trip<S: PolicyStore>(
    store: &mut S,
    policy: PolicyId,
    planted: &RecordedValue,
) -> Result<(), Error> {
    store.apply(policy, planted)?;
    let now = proof_now();
    let restored = {
        let mut session = Session::open(store, MemoryJournal::new(), NoSafety, DEFAULT_TIMEOUT)?;
        let applied = expect_applied(session.set(policy, now)?)?;
        expect_readback(&applied, &RecordedValue::dword(1))?;
        session.restore_all()?
    };
    expect_one(&restored, policy, planted)?;
    expect_value(store, policy, planted)
}

fn prove_restore_everything<S: PolicyStore>(store: &mut S) -> Result<(), Error> {
    let planted = [
        RecordedValue::Absent,
        RecordedValue::dword(0),
        RecordedValue::dword(4),
        RecordedValue::reg_sz("switch"),
        RecordedValue::dword(2),
        RecordedValue::Absent,
    ];
    for (policy, value) in PolicyId::ALL.into_iter().zip(planted.iter()) {
        store.apply(policy, value)?;
    }
    let now = proof_now();
    let restored = {
        let mut session = Session::open(store, MemoryJournal::new(), NoSafety, DEFAULT_TIMEOUT)?;
        for policy in PolicyId::ALL {
            let applied = expect_applied(session.set(policy, now)?)?;
            expect_readback(&applied, &RecordedValue::dword(1))?;
        }
        session.restore_all()?
    };
    if restored.len() != PolicyId::ALL.len() {
        return Err(Error::new(
            "restore everything did not report every setting.",
        ));
    }
    for (policy, value) in PolicyId::ALL.into_iter().zip(planted.iter()) {
        expect_value(store, policy, value)?;
    }
    Ok(())
}

fn proof_now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn expect_applied(mutation: Mutation) -> Result<ReadBack, Error> {
    match mutation {
        Mutation::Applied(report) => Ok(report),
        Mutation::ExpiredAndRestored(_) => Err(Error::new(
            "The safety timer fired before the proof finished.",
        )),
    }
}

fn expect_readback(report: &ReadBack, expected: &RecordedValue) -> Result<(), Error> {
    if report.matches() && report.actual == *expected {
        Ok(())
    } else {
        Err(Error::new(format!(
            "{}: expected {expected}, read back {}.",
            report.policy, report.actual
        )))
    }
}

fn expect_one(
    reports: &[ReadBack],
    policy: PolicyId,
    expected: &RecordedValue,
) -> Result<(), Error> {
    let Some(report) = reports.iter().find(|report| report.policy == policy) else {
        return Err(Error::new(format!("{policy}: restore did not report it.")));
    };
    expect_readback(report, expected)
}

fn expect_value<S: PolicyStore>(
    store: &S,
    policy: PolicyId,
    expected: &RecordedValue,
) -> Result<(), Error> {
    let actual = store.read(policy)?;
    if actual == *expected {
        Ok(())
    } else {
        Err(Error::new(format!(
            "{policy}: expected {expected} after restore, read back {actual}."
        )))
    }
}
