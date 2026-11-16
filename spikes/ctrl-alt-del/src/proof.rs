//! Read-back proof used by `spike-ctrl-alt-del check` and by the seam tests.
//! The store is put back to the values it had when the proof started.

use std::time::{Duration, SystemTime};

use crate::error::Error;
use crate::journal::MemoryJournal;
use crate::policy::PolicyId;
use crate::session::{
    DEFAULT_TIMEOUT, Mutation, NoSafety, PolicyStore, ReadBack, Session, snapshot,
};
use crate::value::RecordedValue;

/// Sets, clears, and restores every policy, then checks the read-back.
pub fn prove_exact_restore<S: PolicyStore>(store: &mut S) -> Result<(), Error> {
    let originals = snapshot(store)?;
    let result = run_proof(store);
    crate::session::restore_snapshot(store, &originals)?;
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
        prove_round_trip(store, policy, &RecordedValue::Absent, false)?;
        prove_round_trip(store, policy, &RecordedValue::dword(0), true)?;
        prove_round_trip(store, policy, &RecordedValue::dword(7), false)?;
        prove_round_trip(store, policy, &RecordedValue::reg_sz("keep"), false)?;
    }
    prove_restore_everything(store)
}

fn prove_round_trip<S: PolicyStore>(
    store: &mut S,
    policy: PolicyId,
    planted: &RecordedValue,
    also_clear: bool,
) -> Result<(), Error> {
    store.apply(policy, planted)?;
    let now = proof_now();
    let restored = {
        let mut session = Session::open(store, MemoryJournal::new(), NoSafety, DEFAULT_TIMEOUT)?;
        let applied = expect_applied(session.set(policy, now)?)?;
        expect_readback(&applied, &RecordedValue::dword(1))?;
        if also_clear {
            let cleared = expect_applied(session.clear(policy, now)?)?;
            expect_readback(&cleared, &RecordedValue::Absent)?;
            if &cleared.previous != planted {
                return Err(Error::new(format!(
                    "{policy}: clear forgot the saved value {}.",
                    cleared.previous
                )));
            }
        }
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
