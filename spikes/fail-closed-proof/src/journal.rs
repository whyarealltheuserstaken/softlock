//! Policy journal. The file is flushed before each registry write, and revert puts
//! every previous value back, including values that were absent.

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const SYSTEM_POLICIES: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\System";
pub const EXPLORER_POLICIES: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";
pub const DISABLE_LOCK: &str = "DisableLockWorkstation";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Hive {
    User,
    Machine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub hive: Hive,
    pub subkey: &'static str,
    pub name: &'static str,
    pub applied: u32,
}

pub const LOCKDOWN: &[Policy] = &[
    Policy {
        hive: Hive::User,
        subkey: SYSTEM_POLICIES,
        name: "DisableTaskMgr",
        applied: 1,
    },
    Policy {
        hive: Hive::User,
        subkey: SYSTEM_POLICIES,
        name: "DisableChangePassword",
        applied: 1,
    },
    Policy {
        hive: Hive::User,
        subkey: SYSTEM_POLICIES,
        name: DISABLE_LOCK,
        applied: 1,
    },
    Policy {
        hive: Hive::User,
        subkey: EXPLORER_POLICIES,
        name: "NoLogoff",
        applied: 1,
    },
    Policy {
        hive: Hive::User,
        subkey: EXPLORER_POLICIES,
        name: "NoClose",
        applied: 1,
    },
    Policy {
        hive: Hive::Machine,
        subkey: SYSTEM_POLICIES,
        name: "HideFastUserSwitching",
        applied: 1,
    },
    Policy {
        hive: Hive::Machine,
        subkey: SYSTEM_POLICIES,
        name: "ConsentPromptBehaviorAdmin",
        applied: 1,
    },
    Policy {
        hive: Hive::Machine,
        subkey: SYSTEM_POLICIES,
        name: "ConsentPromptBehaviorUser",
        applied: 0,
    },
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub hive: Hive,
    pub subkey: String,
    pub name: String,
    /// `None` means the value was not present.
    pub previous: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub sid: String,
    pub entries: Vec<Entry>,
}

impl Document {
    pub fn new(sid: impl Into<String>) -> Self {
        Self {
            sid: sid.into(),
            entries: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub trait PolicyStore {
    fn read(&self, hive: Hive, subkey: &str, name: &str) -> Result<Option<u32>, String>;
    fn write(&mut self, hive: Hive, subkey: &str, name: &str, value: u32) -> Result<(), String>;
    fn delete(&mut self, hive: Hive, subkey: &str, name: &str) -> Result<(), String>;
}

pub trait JournalSink {
    fn persist(&mut self, document: &Document) -> Result<(), String>;
    fn remove(&mut self) -> Result<(), String>;
}

/// Record `previous`, flush the journal, then write the new value.
pub fn apply(
    document: &mut Document,
    store: &mut dyn PolicyStore,
    sink: &mut dyn JournalSink,
    hive: Hive,
    subkey: &str,
    name: &str,
    value: u32,
) -> Result<(), String> {
    if document
        .entries
        .iter()
        .any(|entry| entry.hive == hive && entry.subkey == subkey && entry.name == name)
    {
        return Err(format!("{name} is already in the journal"));
    }
    let previous = store.read(hive, subkey, name)?;
    document.entries.push(Entry {
        hive,
        subkey: subkey.to_string(),
        name: name.to_string(),
        previous,
    });
    sink.persist(document)?;
    store.write(hive, subkey, name, value)?;
    Ok(())
}

/// Change a value that is already journaled. The saved previous value stays as it was.
pub fn overwrite(
    document: &Document,
    store: &mut dyn PolicyStore,
    hive: Hive,
    subkey: &str,
    name: &str,
    value: u32,
) -> Result<(), String> {
    if !document
        .entries
        .iter()
        .any(|entry| entry.hive == hive && entry.subkey == subkey && entry.name == name)
    {
        return Err(format!("{name} was never journaled"));
    }
    store.write(hive, subkey, name, value)
}

/// Restore previous values from last to first. Each removal is flushed after the restore.
pub fn revert(
    document: &mut Document,
    store: &mut dyn PolicyStore,
    sink: &mut dyn JournalSink,
) -> Result<(), String> {
    while let Some(entry) = document.entries.last().cloned() {
        match entry.previous {
            Some(value) => store.write(entry.hive, &entry.subkey, &entry.name, value)?,
            None => store.delete(entry.hive, &entry.subkey, &entry.name)?,
        }
        document.entries.pop();
        sink.persist(document)?;
    }
    sink.remove()?;
    Ok(())
}

pub struct FileJournal {
    path: PathBuf,
}

impl FileJournal {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> Result<Option<Document>, String> {
        let mut corrupt = None;
        for candidate in [self.path.clone(), self.backup(), self.temporary()] {
            let Ok(text) = fs::read_to_string(&candidate) else {
                continue;
            };
            if text.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Document>(&text) {
                Ok(document) => return Ok(Some(document)),
                Err(error) => {
                    corrupt = Some(format!("read {}: {error}", candidate.display()));
                }
            }
        }
        if let Some(error) = corrupt {
            return Err(error);
        }
        Ok(None)
    }

    fn backup(&self) -> PathBuf {
        self.path.with_extension("bak")
    }

    fn temporary(&self) -> PathBuf {
        self.path.with_extension("tmp")
    }
}

impl JournalSink for FileJournal {
    fn persist(&mut self, document: &Document) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        let text = serde_json::to_vec_pretty(document).map_err(|error| error.to_string())?;
        let temporary = self.temporary();
        {
            let mut file = File::create(&temporary)
                .map_err(|error| format!("create {}: {error}", temporary.display()))?;
            file.write_all(&text)
                .map_err(|error| format!("write {}: {error}", temporary.display()))?;
            file.sync_all()
                .map_err(|error| format!("flush {}: {error}", temporary.display()))?;
        }
        if self.path.exists() {
            let backup = self.backup();
            let _ = fs::remove_file(&backup);
            fs::rename(&self.path, &backup)
                .map_err(|error| format!("backup {}: {error}", self.path.display()))?;
        }
        fs::rename(&temporary, &self.path).map_err(|error| {
            format!(
                "replace {} with {}: {error}",
                self.path.display(),
                temporary.display()
            )
        })?;
        Ok(())
    }

    fn remove(&mut self) -> Result<(), String> {
        for path in [self.path.clone(), self.backup(), self.temporary()] {
            if path.exists() {
                fs::remove_file(&path)
                    .map_err(|error| format!("remove {}: {error}", path.display()))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    type Key = (Hive, String, String);

    struct Memory {
        events: Rc<RefCell<Vec<&'static str>>>,
        values: Rc<RefCell<BTreeMap<Key, Option<u32>>>>,
    }

    struct Sink {
        events: Rc<RefCell<Vec<&'static str>>>,
        flushed: Rc<RefCell<Option<Document>>>,
    }

    impl PolicyStore for Memory {
        fn read(&self, hive: Hive, subkey: &str, name: &str) -> Result<Option<u32>, String> {
            Ok(self
                .values
                .borrow()
                .get(&(hive, subkey.to_string(), name.to_string()))
                .copied()
                .unwrap_or(None))
        }

        fn write(
            &mut self,
            hive: Hive,
            subkey: &str,
            name: &str,
            value: u32,
        ) -> Result<(), String> {
            self.events.borrow_mut().push("write");
            self.values
                .borrow_mut()
                .insert((hive, subkey.to_string(), name.to_string()), Some(value));
            Ok(())
        }

        fn delete(&mut self, hive: Hive, subkey: &str, name: &str) -> Result<(), String> {
            self.events.borrow_mut().push("delete");
            self.values
                .borrow_mut()
                .insert((hive, subkey.to_string(), name.to_string()), None);
            Ok(())
        }
    }

    impl JournalSink for Sink {
        fn persist(&mut self, document: &Document) -> Result<(), String> {
            self.events.borrow_mut().push("persist");
            *self.flushed.borrow_mut() = Some(document.clone());
            Ok(())
        }

        fn remove(&mut self) -> Result<(), String> {
            self.events.borrow_mut().push("remove");
            *self.flushed.borrow_mut() = None;
            Ok(())
        }
    }

    struct Rig {
        events: Rc<RefCell<Vec<&'static str>>>,
        store: Memory,
        sink: Sink,
    }

    impl Rig {
        fn new(existing: Option<u32>) -> Self {
            let events = Rc::new(RefCell::new(Vec::new()));
            let values = Rc::new(RefCell::new(BTreeMap::new()));
            if let Some(value) = existing {
                values.borrow_mut().insert(
                    (
                        Hive::User,
                        SYSTEM_POLICIES.to_string(),
                        DISABLE_LOCK.to_string(),
                    ),
                    Some(value),
                );
            }
            Self {
                events: Rc::clone(&events),
                store: Memory {
                    events: Rc::clone(&events),
                    values,
                },
                sink: Sink {
                    events,
                    flushed: Rc::new(RefCell::new(None)),
                },
            }
        }
    }

    #[test]
    fn lockdown_table_names_every_policy_once() {
        assert_eq!(LOCKDOWN.len(), 8);
        let mut names = LOCKDOWN
            .iter()
            .map(|policy| policy.name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 8);
        assert!(LOCKDOWN.iter().any(|policy| policy.name == DISABLE_LOCK));
        assert_eq!(
            LOCKDOWN
                .iter()
                .find(|policy| policy.name == "ConsentPromptBehaviorUser")
                .expect("user consent")
                .applied,
            0
        );
    }

    #[test]
    fn apply_flushes_the_absent_value_before_writing() {
        let mut rig = Rig::new(None);
        let mut document = Document::new("S-1-5-21-1");
        apply(
            &mut document,
            &mut rig.store,
            &mut rig.sink,
            Hive::User,
            SYSTEM_POLICIES,
            DISABLE_LOCK,
            1,
        )
        .expect("apply");
        assert_eq!(rig.events.borrow().as_slice(), ["persist", "write"]);
        assert_eq!(document.entries[0].previous, None);
        let flushed = rig.sink.flushed.borrow().clone().expect("flushed");
        assert_eq!(flushed.entries[0].previous, None);
    }

    #[test]
    fn revert_restores_an_administrator_value_and_an_absent_value() {
        let mut rig = Rig::new(Some(7));
        let mut document = Document::new("S-1-5-21-1");
        apply(
            &mut document,
            &mut rig.store,
            &mut rig.sink,
            Hive::User,
            SYSTEM_POLICIES,
            DISABLE_LOCK,
            1,
        )
        .expect("apply lock");
        apply(
            &mut document,
            &mut rig.store,
            &mut rig.sink,
            Hive::Machine,
            SYSTEM_POLICIES,
            "HideFastUserSwitching",
            1,
        )
        .expect("apply switch");
        overwrite(
            &document,
            &mut rig.store,
            Hive::User,
            SYSTEM_POLICIES,
            DISABLE_LOCK,
            0,
        )
        .expect("clear");
        assert_eq!(document.entries[0].previous, Some(7));
        revert(&mut document, &mut rig.store, &mut rig.sink).expect("revert");
        assert!(document.is_empty());
        assert_eq!(
            rig.store
                .read(Hive::User, SYSTEM_POLICIES, DISABLE_LOCK)
                .expect("read"),
            Some(7)
        );
        assert_eq!(
            rig.store
                .read(Hive::Machine, SYSTEM_POLICIES, "HideFastUserSwitching")
                .expect("read"),
            None
        );
        assert!(rig.sink.flushed.borrow().is_none());
        revert(&mut document, &mut rig.store, &mut rig.sink).expect("second revert");
    }

    #[test]
    fn file_journal_round_trips_and_removes_the_files() {
        let dir = std::env::temp_dir().join(format!(
            "fail-closed-journal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let mut journal = FileJournal::new(dir.join("journal.json"));
        let mut document = Document::new("S-1-5-21-9");
        document.entries.push(Entry {
            hive: Hive::User,
            subkey: SYSTEM_POLICIES.to_string(),
            name: DISABLE_LOCK.to_string(),
            previous: None,
        });
        journal.persist(&document).expect("persist");
        let loaded = journal.load().expect("load").expect("document");
        assert_eq!(loaded, document);
        journal.remove().expect("remove");
        assert!(journal.load().expect("load").is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
