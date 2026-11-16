//! Read-back seam. Setting, clearing, and restoring returns the previous value,
//! including a value that was absent.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use spike_ctrl_alt_del::{
    Command as ToolCommand, DEFAULT_TIMEOUT, EXPLORER_KEY, Error, FileJournal, FileLock, Hive,
    JournalStore, MemoryJournal, MemoryStore, Mutation, NoSafety, PolicyId, PolicyStore,
    RecordedValue, SYSTEM_KEY, Session, parse_args, prove_exact_restore, quote_windows_arg,
    run_once_command,
};

#[test]
fn policies_match_the_spec_locations() {
    assert_eq!(PolicyId::ALL.len(), 6);
    expect_policy(
        PolicyId::DisableTaskMgr,
        Hive::CurrentUser,
        SYSTEM_KEY,
        "Task Manager",
    );
    expect_policy(
        PolicyId::DisableLockWorkstation,
        Hive::CurrentUser,
        SYSTEM_KEY,
        "Lock",
    );
    expect_policy(
        PolicyId::DisableChangePassword,
        Hive::CurrentUser,
        SYSTEM_KEY,
        "Change a password",
    );
    expect_policy(
        PolicyId::NoLogoff,
        Hive::CurrentUser,
        EXPLORER_KEY,
        "Sign out",
    );
    expect_policy(
        PolicyId::HideFastUserSwitching,
        Hive::LocalMachine,
        SYSTEM_KEY,
        "Switch user",
    );
    expect_policy(PolicyId::NoClose, Hive::CurrentUser, EXPLORER_KEY, "Power");
}

#[test]
fn default_timeout_is_ten_minutes() {
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(600));
    let command = parse_args(["set", "all"]).expect("parse");
    match command {
        ToolCommand::Set {
            timeout, policies, ..
        } => {
            assert_eq!(timeout, Duration::from_secs(600));
            assert_eq!(policies, PolicyId::ALL);
        }
        other => panic!("expected set, got {other:?}"),
    }
}

#[test]
fn set_then_restore_puts_back_an_absent_value() {
    let mut store = MemoryStore::new();
    let now = clock();
    let restored = {
        let mut session = open(&mut store, DEFAULT_TIMEOUT);
        let applied = applied(session.set(PolicyId::DisableTaskMgr, now).unwrap());
        assert_eq!(applied.previous, RecordedValue::Absent);
        assert_eq!(applied.actual, RecordedValue::dword(1));
        session.restore_all().unwrap()
    };
    assert_eq!(restored[0].actual, RecordedValue::Absent);
    assert_eq!(
        store.read(PolicyId::DisableTaskMgr).unwrap(),
        RecordedValue::Absent
    );
}

#[test]
fn set_then_restore_puts_back_an_existing_dword() {
    let mut store = MemoryStore::new();
    store
        .apply(PolicyId::NoClose, &RecordedValue::dword(7))
        .unwrap();
    let restored = {
        let mut session = open(&mut store, DEFAULT_TIMEOUT);
        let applied = applied(session.set(PolicyId::NoClose, clock()).unwrap());
        assert_eq!(applied.previous, RecordedValue::dword(7));
        assert_eq!(applied.actual, RecordedValue::dword(1));
        session.restore_all().unwrap()
    };
    assert_eq!(restored[0].actual, RecordedValue::dword(7));
    assert_eq!(
        store.read(PolicyId::NoClose).unwrap(),
        RecordedValue::dword(7)
    );
}

#[test]
fn clear_then_restore_puts_back_the_original_dword() {
    let mut store = MemoryStore::new();
    store
        .apply(PolicyId::NoLogoff, &RecordedValue::dword(5))
        .unwrap();
    let now = clock();
    let restored = {
        let mut session = open(&mut store, DEFAULT_TIMEOUT);
        let turned_on = applied(session.set(PolicyId::NoLogoff, now).unwrap());
        assert_eq!(turned_on.actual, RecordedValue::dword(1));
        let cleared = applied(session.clear(PolicyId::NoLogoff, now).unwrap());
        assert_eq!(cleared.previous, RecordedValue::dword(5));
        assert_eq!(cleared.actual, RecordedValue::Absent);
        session.restore_all().unwrap()
    };
    assert_eq!(restored[0].actual, RecordedValue::dword(5));
    assert_eq!(
        store.read(PolicyId::NoLogoff).unwrap(),
        RecordedValue::dword(5)
    );
}

#[test]
fn non_dword_bytes_are_restored_exactly() {
    let planted = RecordedValue::reg_sz("keep");
    let mut store = MemoryStore::new();
    store
        .apply(PolicyId::DisableChangePassword, &planted)
        .unwrap();
    let restored = {
        let mut session = open(&mut store, DEFAULT_TIMEOUT);
        applied(
            session
                .set(PolicyId::DisableChangePassword, clock())
                .unwrap(),
        );
        session.restore_all().unwrap()
    };
    assert_eq!(restored[0].actual, planted);
    assert_eq!(
        store.read(PolicyId::DisableChangePassword).unwrap(),
        planted
    );
}

#[test]
fn restore_everything_covers_every_policy() {
    let mut store = MemoryStore::new();
    let restored = {
        let mut session = open(&mut store, DEFAULT_TIMEOUT);
        let now = clock();
        for policy in PolicyId::ALL {
            applied(session.set(policy, now).unwrap());
        }
        let restored = session.restore_all().unwrap();
        assert!(session.restore_all().unwrap().is_empty());
        restored
    };
    assert_eq!(restored.len(), PolicyId::ALL.len());
    for policy in PolicyId::ALL {
        assert_eq!(store.read(policy).unwrap(), RecordedValue::Absent);
    }
}

#[test]
fn timeout_restores_when_due_and_not_before() {
    let mut store = MemoryStore::new();
    let start = clock();
    let restored = {
        let mut session = open(&mut store, Duration::from_secs(600));
        let applied = applied(
            session
                .set(PolicyId::DisableLockWorkstation, start)
                .unwrap(),
        );
        assert_eq!(applied.actual, RecordedValue::dword(1));
        assert!(
            session
                .restore_if_due(start + Duration::from_secs(599))
                .unwrap()
                .is_none()
        );
        session
            .restore_if_due(start + Duration::from_secs(600))
            .unwrap()
            .expect("due")
    };
    assert_eq!(restored[0].actual, RecordedValue::Absent);
    assert_eq!(
        store.read(PolicyId::DisableLockWorkstation).unwrap(),
        RecordedValue::Absent
    );
}

#[test]
fn a_later_change_extends_the_deadline_and_keeps_the_original_value() {
    let mut store = MemoryStore::new();
    store
        .apply(PolicyId::HideFastUserSwitching, &RecordedValue::dword(3))
        .unwrap();
    let mut session = open(&mut store, Duration::from_secs(600));
    let start = clock();
    applied(session.set(PolicyId::HideFastUserSwitching, start).unwrap());
    applied(
        session
            .clear(
                PolicyId::HideFastUserSwitching,
                start + Duration::from_secs(500),
            )
            .unwrap(),
    );
    assert_eq!(
        session.journal().unwrap().entries[0].previous,
        RecordedValue::dword(3)
    );
    assert!(
        session
            .restore_if_due(start + Duration::from_secs(600))
            .unwrap()
            .is_none()
    );
    let restored = session
        .restore_if_due(start + Duration::from_secs(1_100))
        .unwrap()
        .expect("extended deadline");
    assert_eq!(restored[0].actual, RecordedValue::dword(3));
}

#[test]
fn a_change_after_the_deadline_restores_instead_of_applying() {
    let mut store = MemoryStore::new();
    let start = clock();
    {
        let mut session = open(&mut store, Duration::from_secs(600));
        applied(session.set(PolicyId::NoClose, start).unwrap());
        let mutation = session
            .set(PolicyId::NoClose, start + Duration::from_secs(600))
            .unwrap();
        match mutation {
            Mutation::ExpiredAndRestored(reports) => {
                assert_eq!(reports[0].actual, RecordedValue::Absent);
            }
            Mutation::Applied(_) => panic!("the timer should have put the value back"),
        }
    }
    assert_eq!(
        store.read(PolicyId::NoClose).unwrap(),
        RecordedValue::Absent
    );
}

#[test]
fn journal_is_flushed_before_the_registry_write() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut store = LogStore {
        log: Arc::clone(&log),
        inner: MemoryStore::new(),
    };
    let mut session = Session::open(
        &mut store,
        LogJournal {
            log: Arc::clone(&log),
            inner: MemoryJournal::new(),
        },
        NoSafety,
        DEFAULT_TIMEOUT,
    )
    .unwrap();
    applied(session.set(PolicyId::DisableTaskMgr, clock()).unwrap());
    assert_eq!(log.lock().unwrap().as_slice(), ["flush", "write"]);
}

#[test]
fn a_failed_write_keeps_the_saved_previous_value() {
    let mut store = FailStore;
    let mut session =
        Session::open(&mut store, MemoryJournal::new(), NoSafety, DEFAULT_TIMEOUT).unwrap();
    let error = session.set(PolicyId::DisableTaskMgr, clock()).unwrap_err();
    assert!(error.to_string().contains("disk full"));
    let saved = session.journal().expect("journal kept");
    assert_eq!(saved.entries[0].previous, RecordedValue::Absent);
}

#[test]
fn file_journal_records_absent_and_restores_it() {
    let dir = temp_dir("journal");
    let mut store = MemoryStore::new();
    {
        let mut session = Session::open(
            &mut store,
            FileJournal::new(&dir),
            NoSafety,
            DEFAULT_TIMEOUT,
        )
        .unwrap();
        applied(session.set(PolicyId::DisableTaskMgr, clock()).unwrap());
    }
    let text = fs::read_to_string(dir.join("journal.json")).unwrap();
    assert!(text.contains("\"previous\": \"absent\""));
    assert!(text.contains("DisableTaskMgr"));
    assert_eq!(
        store.read(PolicyId::DisableTaskMgr).unwrap(),
        RecordedValue::dword(1)
    );
    {
        let mut session = Session::open(
            &mut store,
            FileJournal::new(&dir),
            NoSafety,
            DEFAULT_TIMEOUT,
        )
        .unwrap();
        let restored = session.restore_all().unwrap();
        assert_eq!(restored[0].actual, RecordedValue::Absent);
    }
    assert!(!dir.join("journal.json").exists());
    assert_eq!(
        store.read(PolicyId::DisableTaskMgr).unwrap(),
        RecordedValue::Absent
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn file_lock_is_exclusive_until_dropped() {
    let dir = temp_dir("lock");
    let lock = FileLock::acquire(&dir).unwrap();
    let dir_for_waiter = dir.clone();
    let handle = thread::spawn(move || {
        let started = Instant::now();
        let _lock = FileLock::acquire(&dir_for_waiter).unwrap();
        started.elapsed()
    });
    thread::sleep(Duration::from_millis(200));
    drop(lock);
    let waited = handle.join().unwrap();
    assert!(waited >= Duration::from_millis(150));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn proof_covers_absent_present_clear_and_every_policy() {
    let mut store = MemoryStore::new();
    store
        .apply(PolicyId::NoClose, &RecordedValue::dword(9))
        .unwrap();
    prove_exact_restore(&mut store).unwrap();
    assert_eq!(
        store.read(PolicyId::NoClose).unwrap(),
        RecordedValue::dword(9)
    );
    assert_eq!(
        store.read(PolicyId::DisableTaskMgr).unwrap(),
        RecordedValue::Absent
    );
}

#[test]
fn spike_is_outside_the_default_release_build() {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(repo_root().join("Cargo.toml"))
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let meta: serde_json::Value = serde_json::from_slice(&output.stdout).expect("metadata json");
    let packages = meta["packages"].as_array().expect("packages");
    let spike = packages
        .iter()
        .find(|package| package["name"] == "spike-ctrl-alt-del")
        .expect("spike package");
    assert_eq!(spike["publish"], serde_json::json!([]));
    let spike_id = spike["id"].as_str().expect("spike id");
    let members = meta["workspace_members"]
        .as_array()
        .expect("workspace members");
    let defaults = meta["workspace_default_members"]
        .as_array()
        .expect("default members");
    assert!(members.iter().any(|id| id == spike_id));
    assert!(defaults.iter().all(|id| id != spike_id));
    for member in members {
        let id = member.as_str().expect("member id");
        if id == spike_id {
            continue;
        }
        assert!(
            defaults.iter().any(|default| default == member),
            "{id} is missing from the default release build"
        );
        let package = packages
            .iter()
            .find(|package| package["id"] == id)
            .expect("member package");
        for dependency in package["dependencies"].as_array().expect("dependencies") {
            assert_ne!(
                dependency["name"].as_str().unwrap_or_default(),
                "spike-ctrl-alt-del",
                "{} depends on the spike",
                package["name"]
            );
        }
    }
}

#[test]
fn unknown_setting_names_are_rejected() {
    let error = parse_args(["set", "AllowList"]).unwrap_err();
    assert!(error.to_string().contains("Unknown setting"));
}

#[test]
fn restore_command_quotes_the_program_path() {
    let command = run_once_command(std::path::Path::new(
        r"C:\Program Files\spike-ctrl-alt-del.exe",
    ));
    assert_eq!(
        command,
        r#""C:\Program Files\spike-ctrl-alt-del.exe" restore"#
    );
    assert_eq!(quote_windows_arg("all"), "all");
    assert_eq!(
        quote_windows_arg(r"C:\Program Files\tool.exe"),
        r#""C:\Program Files\tool.exe""#
    );
}

fn expect_policy(policy: PolicyId, hive: Hive, key_path: &str, screen_label: &str) {
    let info = policy.info();
    assert_eq!(info.hive, hive);
    assert_eq!(info.key_path, key_path);
    assert_eq!(info.value_name, policy.name());
    assert_eq!(info.screen_label, screen_label);
}

fn open(
    store: &mut MemoryStore,
    timeout: Duration,
) -> Session<'_, MemoryStore, MemoryJournal, NoSafety> {
    Session::open(store, MemoryJournal::new(), NoSafety, timeout).unwrap()
}

fn applied(mutation: Mutation) -> spike_ctrl_alt_del::ReadBack {
    match mutation {
        Mutation::Applied(report) => report,
        Mutation::ExpiredAndRestored(_) => panic!("timer fired"),
    }
}

fn clock() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "spike-ctrl-alt-del-{name}-{}-{}",
        std::process::id(),
        clock().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

struct LogStore {
    log: Arc<Mutex<Vec<&'static str>>>,
    inner: MemoryStore,
}

impl PolicyStore for LogStore {
    fn read(&self, policy: PolicyId) -> Result<RecordedValue, Error> {
        self.inner.read(policy)
    }

    fn apply(&mut self, policy: PolicyId, value: &RecordedValue) -> Result<(), Error> {
        self.log.lock().expect("log").push("write");
        self.inner.apply(policy, value)
    }
}

struct LogJournal {
    log: Arc<Mutex<Vec<&'static str>>>,
    inner: MemoryJournal,
}

impl JournalStore for LogJournal {
    fn load(&self) -> Result<Option<spike_ctrl_alt_del::Journal>, Error> {
        self.inner.load()
    }

    fn flush(&mut self, journal: &spike_ctrl_alt_del::Journal) -> Result<(), Error> {
        self.log.lock().expect("log").push("flush");
        self.inner.flush(journal)
    }

    fn remove(&mut self) -> Result<(), Error> {
        self.inner.remove()
    }
}

struct FailStore;

impl PolicyStore for FailStore {
    fn read(&self, _policy: PolicyId) -> Result<RecordedValue, Error> {
        Ok(RecordedValue::Absent)
    }

    fn apply(&mut self, _policy: PolicyId, _value: &RecordedValue) -> Result<(), Error> {
        Err(Error::new("disk full"))
    }
}
