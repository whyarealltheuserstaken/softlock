//! Admin proof for the six Ctrl+Alt+Del policy values.
//!
//! This crate is a spike. It is a workspace member so its tests run, and it is
//! not a default member, so a release build does not include it. Shipping crates
//! must not depend on it.

mod account;
mod error;
mod journal;
mod parse;
mod policy;
mod process;
mod proof;
mod session;
mod value;

#[cfg(windows)]
mod windows;

pub use account::{Account, same_account};
pub use error::Error;
pub use journal::{
    FileJournal, FileLock, JOURNAL_ENV, JOURNAL_FILE, Journal, JournalEntry, JournalStore,
    MemoryJournal, RESTORE_RESULT, WATCHER_LOG, default_journal_dir, remaining, retry_delay,
    time_from_millis, unix_millis, watcher_gave_up, watcher_journal_dir,
};
pub use parse::{Command, MAX_TIMEOUT_MINUTES, parse_args, quote_windows_arg, run_once_command};
pub use policy::{
    EXPLORER_KEY, Hive, Policy, PolicyId, SYSTEM_KEY, is_deletable_policy_key, split_key_id,
};
pub use process::process_is_running;
pub use proof::prove_exact_restore;
pub use session::{
    DEFAULT_TIMEOUT, MemoryStore, Mutation, NoSafety, PolicyStore, ReadBack, SafetyNet, Session,
    confirm_write, restore_snapshot, snapshot,
};
pub use value::{REG_DWORD, REG_SZ, RecordedValue};

#[cfg(windows)]
pub use windows::{
    RunOnceSafety, WindowsStore, can_write_machine_policy, console_account, current_account,
    ensure_interactive_account, lock_down_directory, relaunch_elevated,
};
