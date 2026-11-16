use std::process::ExitCode;

#[cfg(windows)]
use std::time::SystemTime;

use spike_ctrl_alt_del::{Command, Error, parse_args};
#[cfg(windows)]
use spike_ctrl_alt_del::{JournalStore, PolicyStore};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), Error> {
    match parse_args(std::env::args().skip(1))? {
        Command::Help => {
            print_help();
            Ok(())
        }
        command => dispatch(command),
    }
}

fn print_help() {
    println!(
        r#"spike-ctrl-alt-del

Turns six Ctrl+Alt+Del settings on and off for a short test, then puts them back.

Commands:
  status                  Show the six settings and any saved values
  set <name|all>          Turn a setting on
  clear <name|all>        Remove a setting
  restore                 Put every saved setting back
  check                   Automated read-back test

Names:
  DisableTaskMgr            Task Manager
  NoLogoff                  Sign out
  DisableChangePassword     Change a password
  HideFastUserSwitching     Switch user
  DisableLockWorkstation    Lock
  NoClose                   Power

Options:
  --timeout-minutes <1-120>   Safety timer. The default is 10.
  --journal <folder>          Where the saved values are stored

Run this from an Administrator terminal.
The safety timer puts every setting back 10 minutes after the last change.
Closing the window does not cancel the timer.
If the computer restarts first, the next sign-in asks to put the settings back.
To put them back immediately, run: spike-ctrl-alt-del restore
"#
    );
}

#[cfg(not(windows))]
fn dispatch(_command: Command) -> Result<(), Error> {
    Err(Error::new(
        "This program changes Windows settings. Run it on Windows.",
    ))
}

#[cfg(windows)]
fn dispatch(command: Command) -> Result<(), Error> {
    match command {
        Command::Help => Ok(()),
        Command::Status { journal } => cmd_status(&journal),
        Command::Set {
            policies,
            timeout,
            journal,
        } => change(&policies, timeout, &journal, false),
        Command::Clear {
            policies,
            timeout,
            journal,
        } => change(&policies, timeout, &journal, true),
        Command::Restore { journal } => cmd_restore(&journal),
        Command::Check => cmd_check(),
        Command::Watch { journal } => cmd_watch(&journal),
    }
}

#[cfg(windows)]
fn cmd_status(dir: &std::path::Path) -> Result<(), Error> {
    use spike_ctrl_alt_del::{FileJournal, PolicyId, WindowsStore, remaining, time_from_millis};

    let store = WindowsStore;
    println!("Current settings:");
    for policy in PolicyId::ALL {
        let info = policy.info();
        let value = store.read(policy)?;
        println!(
            "  {} ({}) {}\\{} = {value}",
            info.value_name,
            info.screen_label,
            info.hive.label(),
            info.key_path
        );
    }
    match FileJournal::new(dir).load()? {
        None => println!("Nothing is waiting to be put back."),
        Some(journal) => {
            println!("Saved values that restore will put back:");
            for entry in &journal.entries {
                println!("  {}: {}", entry.policy, entry.previous);
            }
            let left = remaining(
                time_from_millis(journal.deadline_unix_ms),
                SystemTime::now(),
            );
            println!(
                "Safety timer: {} minutes and {} seconds left.",
                left.as_secs() / 60,
                left.as_secs() % 60
            );
            println!("To put them back now, run: {}", restore_hint());
            println!("Saved in {}", dir.display());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn change(
    policies: &[spike_ctrl_alt_del::PolicyId],
    timeout: std::time::Duration,
    dir: &std::path::Path,
    clear: bool,
) -> Result<(), Error> {
    use spike_ctrl_alt_del::{
        FileJournal, FileLock, Mutation, RunOnceSafety, Session, WindowsStore,
    };

    ensure_elevated()?;
    std::fs::create_dir_all(dir)?;
    let deadline = {
        let _lock = FileLock::acquire(dir)?;
        let mut store = WindowsStore;
        let safety = RunOnceSafety::for_current_exe()?;
        let mut session = Session::open(&mut store, FileJournal::new(dir), safety, timeout)?;
        let now = SystemTime::now();
        for policy in policies {
            let mutation = if clear {
                session.clear(*policy, now)?
            } else {
                session.set(*policy, now)?
            };
            match mutation {
                Mutation::ExpiredAndRestored(reports) => {
                    print_restored(&reports);
                    println!("The safety timer already put every setting back.");
                    println!("Run the command again if you still want to change a setting.");
                    return Ok(());
                }
                Mutation::Applied(report) => print_change(&report, clear),
            }
        }
        session.deadline()
    };
    print_timer(deadline);
    if let Err(err) = spawn_watcher(dir) {
        println!("Warning: the safety timer did not start ({err}).");
        println!(
            "Run {} yourself when you are done. Do not wait.",
            restore_hint()
        );
    }
    Ok(())
}

#[cfg(windows)]
fn cmd_restore(dir: &std::path::Path) -> Result<(), Error> {
    use spike_ctrl_alt_del::{
        DEFAULT_TIMEOUT, FileJournal, FileLock, RunOnceSafety, Session, WindowsStore,
    };

    ensure_elevated()?;
    if !dir.exists() {
        println!("Nothing to put back.");
        return Ok(());
    }
    let _lock = FileLock::acquire(dir)?;
    let mut store = WindowsStore;
    let safety = RunOnceSafety::for_current_exe()?;
    let mut session = Session::open(&mut store, FileJournal::new(dir), safety, DEFAULT_TIMEOUT)?;
    let reports = session.restore_all()?;
    if reports.is_empty() {
        println!("Nothing to put back.");
        return Ok(());
    }
    print_restored(&reports);
    println!("Read-back matched the saved values, including any that were absent.");
    Ok(())
}

#[cfg(windows)]
fn cmd_check() -> Result<(), Error> {
    use std::time::Duration;

    use spike_ctrl_alt_del::{
        FileJournal, FileLock, Mutation, NoSafety, PolicyId, Session, WindowsStore,
        can_write_machine_policy, prove_exact_restore, restore_snapshot, snapshot,
    };

    if !can_write_machine_policy() {
        return Err(Error::new(
            "This check needs an Administrator terminal. Nothing was changed.",
        ));
    }
    let mut store = WindowsStore;
    let originals = snapshot(&store)?;
    let journal_dir =
        std::env::temp_dir().join(format!("spike-ctrl-alt-del-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&journal_dir);
    let _guard = RestoreGuard {
        originals: originals.clone(),
        journal_dir: journal_dir.clone(),
    };
    prove_exact_restore(&mut store)?;

    std::fs::create_dir_all(&journal_dir)?;
    {
        let _lock = FileLock::acquire(&journal_dir)?;
        let mut session = Session::open(
            &mut store,
            FileJournal::new(&journal_dir),
            NoSafety,
            Duration::from_secs(1),
        )?;
        let now = SystemTime::now();
        for policy in PolicyId::ALL {
            match session.set(policy, now)? {
                Mutation::Applied(report) if report.matches() => {}
                Mutation::Applied(report) => {
                    return Err(Error::new(format!(
                        "{}: expected {}, read back {}.",
                        report.policy, report.expected, report.actual
                    )));
                }
                Mutation::ExpiredAndRestored(_) => {
                    return Err(Error::new(
                        "The safety timer fired before the values were set.",
                    ));
                }
            }
        }
    }
    spawn_watcher(&journal_dir)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if std::time::Instant::now() > deadline {
            return Err(Error::new(
                "The safety timer did not restore the values within 20 seconds.",
            ));
        }
        let journal_gone = !journal_dir.join("journal.json").exists();
        let mut all_match = true;
        for (policy, expected) in &originals {
            if store.read(*policy)? != *expected {
                all_match = false;
                break;
            }
        }
        if all_match && journal_gone {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    restore_snapshot(&mut store, &originals)?;
    println!("Read-back check passed.");
    println!("Every setting was set, cleared, and restored.");
    println!("Absent values were absent again, and existing values came back unchanged.");
    Ok(())
}

#[cfg(windows)]
struct RestoreGuard {
    originals: Vec<(
        spike_ctrl_alt_del::PolicyId,
        spike_ctrl_alt_del::RecordedValue,
    )>,
    journal_dir: std::path::PathBuf,
}

#[cfg(windows)]
impl Drop for RestoreGuard {
    fn drop(&mut self) {
        use spike_ctrl_alt_del::{FileJournal, WindowsStore, restore_snapshot};

        let mut store = WindowsStore;
        if let Err(err) = restore_snapshot(&mut store, &self.originals) {
            eprintln!("Could not put the original settings back: {err}");
        }
        let mut journal = FileJournal::new(&self.journal_dir);
        let _ = journal.remove();
        let _ = std::fs::remove_dir_all(&self.journal_dir);
    }
}

#[cfg(windows)]
fn cmd_watch(dir: &std::path::Path) -> Result<(), Error> {
    use spike_ctrl_alt_del::{
        DEFAULT_TIMEOUT, FileJournal, FileLock, RunOnceSafety, Session, WindowsStore,
    };

    std::fs::create_dir_all(dir)?;
    let pid_path = dir.join("watcher.pid");
    std::fs::write(&pid_path, std::process::id().to_string())?;
    let _pid = PidGuard(pid_path);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let done = {
            let _lock = FileLock::acquire(dir)?;
            let mut store = WindowsStore;
            let safety = RunOnceSafety::for_current_exe()?;
            let mut session =
                Session::open(&mut store, FileJournal::new(dir), safety, DEFAULT_TIMEOUT)?;
            if session.restore_if_due(SystemTime::now())?.is_some() {
                true
            } else {
                !session.has_journal()
            }
        };
        if done {
            return Ok(());
        }
    }
}

#[cfg(windows)]
fn ensure_elevated() -> Result<(), Error> {
    use spike_ctrl_alt_del::{can_write_machine_policy, relaunch_elevated};

    if can_write_machine_policy() {
        return Ok(());
    }
    println!("This needs an Administrator window. Click Yes if Windows asks for permission.");
    relaunch_elevated()?;
    std::process::exit(0);
}

#[cfg(windows)]
fn spawn_watcher(dir: &std::path::Path) -> Result<(), Error> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    if watcher_running(dir) {
        return Ok(());
    }
    let exe = std::env::current_exe()
        .map_err(|err| Error::new(format!("Could not find this program: {err}")))?;
    let mut command = Command::new(exe);
    command
        .arg("watch")
        .arg("--journal")
        .arg(dir)
        .stdin(Stdio::null());
    if let Ok(log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("watcher.log"))
    {
        if let Ok(err_log) = log.try_clone() {
            command.stdout(log).stderr(err_log);
        }
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }

    const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let base = CREATE_UNICODE_ENVIRONMENT | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    if command
        .creation_flags(base | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
        .is_err()
    {
        command.creation_flags(base).spawn()?;
    }

    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(5) {
        if watcher_running(dir) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err(Error::new("the safety timer did not start"))
}

#[cfg(windows)]
fn watcher_running(dir: &std::path::Path) -> bool {
    use spike_ctrl_alt_del::process_is_running;

    let path = dir.join("watcher.pid");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(pid) = text.trim().parse::<u32>() else {
        let _ = std::fs::remove_file(&path);
        return false;
    };
    if process_is_running(pid) {
        true
    } else {
        let _ = std::fs::remove_file(&path);
        false
    }
}

#[cfg(windows)]
struct PidGuard(std::path::PathBuf);

#[cfg(windows)]
impl Drop for PidGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(windows)]
fn print_change(report: &spike_ctrl_alt_del::ReadBack, clear: bool) {
    let info = report.policy.info();
    if clear {
        println!(
            "{} ({}): saved the old value ({}) and removed the setting.",
            info.value_name, info.screen_label, report.previous
        );
    } else {
        println!(
            "{} ({}): saved the old value ({}) and turned the setting on.",
            info.value_name, info.screen_label, report.previous
        );
    }
    println!("Read it back: {}.", report.actual);
    for warning in &report.warnings {
        println!("Warning: {warning}");
    }
}

#[cfg(windows)]
fn print_restored(reports: &[spike_ctrl_alt_del::ReadBack]) {
    for report in reports {
        println!("Put {} back to {}.", report.policy, report.actual);
        for warning in &report.warnings {
            println!("Warning: {warning}");
        }
    }
}

#[cfg(windows)]
fn print_timer(deadline: Option<SystemTime>) {
    let Some(deadline) = deadline else {
        return;
    };
    let left = spike_ctrl_alt_del::remaining(deadline, SystemTime::now());
    println!(
        "If you do nothing, every setting goes back in {} minutes and {} seconds.",
        left.as_secs() / 60,
        left.as_secs() % 60
    );
    println!("Closing this window does not cancel that.");
    println!("To put every setting back now, run: {}", restore_hint());
}

#[cfg(windows)]
fn restore_hint() -> String {
    let name = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "spike-ctrl-alt-del.exe".to_string());
    format!(".\\{name} restore")
}
