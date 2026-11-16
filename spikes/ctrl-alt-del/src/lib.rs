//! Admin proof for the six Ctrl+Alt+Del policy values.
//!
//! This crate is a spike. It is a workspace member so its tests run, and it is
//! not a default member, so a release build does not include it. Shipping crates
//! must not depend on it.

mod error;
mod journal;
mod parse;
mod policy;
mod session;
mod value;

#[cfg(windows)]
mod windows;

pub use error::Error;
pub use journal::{
    FileJournal, FileLock, Journal, JournalEntry, JournalStore, MemoryJournal, default_journal_dir,
    remaining, time_from_millis,
};
pub use parse::{Command, parse_args, quote_windows_arg, run_once_command};
pub use policy::{EXPLORER_KEY, Hive, Policy, PolicyId, SYSTEM_KEY};
pub use session::{
    DEFAULT_TIMEOUT, MemoryStore, Mutation, NoSafety, PolicyStore, ReadBack, SafetyNet, Session,
    prove_exact_restore, restore_snapshot, snapshot,
};
pub use value::{REG_DWORD, REG_SZ, RecordedValue};

#[cfg(windows)]
pub use windows::{
    RunOnceSafety, WindowsStore, can_write_machine_policy, process_is_running, relaunch_elevated,
};
