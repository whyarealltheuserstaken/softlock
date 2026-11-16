use std::process::ExitCode;

#[cfg(windows)]
use std::time::SystemTime;

use spike_ctrl_alt_del::{Command, Error, parse_args};

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
  status                  Show the six settings, the account, and any saved values
  set <name|all>          Turn a setting on
  clear <name|all>        Remove a setting
  restore                 Put every saved setting back
  check                   Automated read-back test

Names:
  DisableTaskMgr            Task Manager
  NoLogoff                  Sign out
  DisableChangePassword     Change a password
  HideFastUserSwitching     Switch user (this one is for every account on the computer)
  DisableLockWorkstation    Lock
  NoClose                   Power

Options:
  --minutes <1-30>        Safety timer. The default is 10. 30 is the longest.

Run this from an Administrator terminal on your own account.
If Windows asks for a password instead of just Yes, stop.
The safety timer puts every setting back 10 minutes after the last change.
Closing the window does not cancel the timer.
If the computer restarts first, the settings stay on until restore runs.
The next sign-in asks to put the settings back. If you click No, run restore yourself.
Saved settings are kept in the default folder. To put them back now, run: spike-ctrl-alt-del restore
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
        Command::Status => cmd_status(),
        Command::Set { policies, timeout } => change(&policies, timeout, false),
        Command::Clear { policies, timeout } => change(&policies, timeout, true),
        Command::Restore => cmd_restore(),
        Command::Check => cmd_check(),
        Command::Watch => cmd_watch(),
    }
}

#[cfg(windows)]
fn journal_dir() -> std::path::PathBuf {
    spike_ctrl_alt_del::default_journal_dir()
}

#[cfg(windows)]
fn cmd_status() -> Result<(), Error> {
    use spike_ctrl_alt_del::{
        FileJournal, JournalStore, PolicyId, PolicyStore, WindowsStore, console_account,
        current_account, remaining, time_from_millis,
    };

    match current_account() {
        Ok(account) => println!("This window would change settings for {account}."),
        Err(err) => println!("Could not read this window's account: {err}"),
    }
    match console_account() {
        Some(account) => println!("The person at the keyboard is {account}."),
        None => println!(
            "Could not see who is at the keyboard. If Windows asks for a password instead of just Yes, stop."
        ),
    }

    let dir = journal_dir();
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
    println!("HideFastUserSwitching is HKLM. It affects every account on this computer.");
    match FileJournal::new(&dir).load()? {
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
    clear: bool,
) -> Result<(), Error> {
    use spike_ctrl_alt_del::{
        FileJournal, FileLock, Mutation, RunOnceSafety, Session, WindowsStore,
    };

    ensure_elevated()?;
    ensure_account()?;
    let dir = journal_dir();
    prepare_dir(&dir)?;
    let deadline = {
        let _lock = FileLock::acquire(&dir)?;
        let mut store = WindowsStore;
        let safety = RunOnceSafety::for_current_exe()?;
        let mut session = Session::open(&mut store, FileJournal::new(&dir), safety, timeout)?;
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
    if let Err(err) = spawn_watcher(&dir) {
        println!("Warning: the safety timer did not start ({err}).");
        println!(
            "Run {} yourself when you are done. Do not wait.",
            restore_hint()
        );
    }
    Ok(())
}

#[cfg(windows)]
fn cmd_restore() -> Result<(), Error> {
    use spike_ctrl_alt_del::{
        DEFAULT_TIMEOUT, FileJournal, FileLock, RunOnceSafety, Session, WindowsStore,
    };

    ensure_elevated()?;
    ensure_account()?;
    let dir = journal_dir();
    let outcome = if !dir.exists() {
        "Nothing to put back.".to_string()
    } else {
        let _lock = FileLock::acquire(&dir)?;
        let mut store = WindowsStore;
        let safety = RunOnceSafety::for_current_exe()?;
        let mut session =
            Session::open(&mut store, FileJournal::new(&dir), safety, DEFAULT_TIMEOUT)?;
        let reports = session.restore_all()?;
        let _ = std::fs::remove_file(dir.join(spike_ctrl_alt_del::WATCHER_LOG));
        if reports.is_empty() {
            "Nothing to put back.".to_string()
        } else {
            let mut text = String::new();
            for report in &reports {
                text.push_str(&format!(
                    "Put {} back to {}.\n",
                    report.policy, report.actual
                ));
                for warning in &report.warnings {
                    text.push_str(&format!("Warning: {warning}\n"));
                }
            }
            text.push_str("Read-back matched the saved values, including any that were absent.\n");
            text
        }
    };
    show_and_pause(&dir, &outcome)?;
    Ok(())
}

#[cfg(windows)]
fn cmd_check() -> Result<(), Error> {
    use std::time::Duration;

    use spike_ctrl_alt_del::{
        FileJournal, FileLock, Mutation, NoSafety, PolicyId, PolicyStore, Session, WindowsStore,
        can_write_machine_policy, prove_exact_restore, restore_snapshot, snapshot,
    };

    if !can_write_machine_policy() {
        return Err(Error::new(
            "This check needs an Administrator terminal. Nothing was changed.",
        ));
    }
    ensure_account()?;
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

    prepare_dir(&journal_dir)?;
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
        let journal_gone = !journal_dir.join(spike_ctrl_alt_del::JOURNAL_FILE).exists();
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
        use spike_ctrl_alt_del::{FileJournal, JournalStore, WindowsStore, restore_snapshot};

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
fn cmd_watch() -> Result<(), Error> {
    use spike_ctrl_alt_del::{
        DEFAULT_TIMEOUT, FileJournal, FileLock, JOURNAL_FILE, JournalStore, RunOnceSafety, Session,
        WATCHER_LOG, WindowsStore, retry_delay, watcher_journal_dir,
    };

    let dir = watcher_journal_dir();
    prepare_dir(&dir)?;
    let pid_path = dir.join("watcher.pid");
    std::fs::write(&pid_path, std::process::id().to_string())?;
    let _pid = PidGuard(pid_path);
    append_log(&dir, "watcher started");

    let mut seen: Option<FileStamp> = None;
    let mut deadline_ms: Option<u64> = None;
    let mut failures = 0u32;
    loop {
        let path = dir.join(JOURNAL_FILE);
        match file_stamp(&path) {
            None => {
                if seen.is_some() {
                    let _ = std::fs::remove_file(dir.join(WATCHER_LOG));
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
                continue;
            }
            Some(stamp) => {
                if seen.as_ref() != Some(&stamp) {
                    match FileJournal::new(&dir).load() {
                        Ok(Some(journal)) => {
                            deadline_ms = Some(journal.deadline_unix_ms);
                            seen = Some(stamp);
                        }
                        Ok(None) => {
                            std::thread::sleep(std::time::Duration::from_millis(200));
                            continue;
                        }
                        Err(err) => {
                            failures += 1;
                            append_log(&dir, &format!("could not read the saved settings: {err}"));
                            std::thread::sleep(retry_delay(failures));
                            continue;
                        }
                    }
                }
            }
        }

        let Some(deadline) = deadline_ms else {
            std::thread::sleep(std::time::Duration::from_millis(200));
            continue;
        };
        let now = match spike_ctrl_alt_del::unix_millis(SystemTime::now()) {
            Ok(now) => now,
            Err(err) => {
                failures += 1;
                append_log(&dir, &err.to_string());
                std::thread::sleep(retry_delay(failures));
                continue;
            }
        };
        if now < deadline {
            let wait = (deadline - now).min(1_000);
            std::thread::sleep(std::time::Duration::from_millis(wait));
            continue;
        }

        let restored = {
            let _lock = match FileLock::acquire(&dir) {
                Ok(lock) => lock,
                Err(err) => {
                    failures += 1;
                    append_log(&dir, &format!("could not lock the saved settings: {err}"));
                    std::thread::sleep(retry_delay(failures));
                    continue;
                }
            };
            let mut store = WindowsStore;
            let safety = match RunOnceSafety::for_current_exe() {
                Ok(safety) => safety,
                Err(err) => {
                    failures += 1;
                    append_log(&dir, &err.to_string());
                    std::thread::sleep(retry_delay(failures));
                    continue;
                }
            };
            let mut session =
                match Session::open(&mut store, FileJournal::new(&dir), safety, DEFAULT_TIMEOUT) {
                    Ok(session) => session,
                    Err(err) => {
                        failures += 1;
                        append_log(&dir, &err.to_string());
                        std::thread::sleep(retry_delay(failures));
                        continue;
                    }
                };
            match session.restore_if_due(SystemTime::now()) {
                Ok(restored) => restored,
                Err(err) => {
                    failures += 1;
                    append_log(&dir, &format!("restore failed, trying again: {err}"));
                    std::thread::sleep(retry_delay(failures));
                    continue;
                }
            }
        };
        if restored.is_some() {
            let _ = std::fs::remove_file(dir.join(WATCHER_LOG));
            return Ok(());
        }
        seen = None;
        failures = 0;
    }
}

#[cfg(windows)]
#[derive(PartialEq, Eq)]
struct FileStamp {
    modified: Option<std::time::SystemTime>,
    len: u64,
}

#[cfg(windows)]
fn file_stamp(path: &std::path::Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(FileStamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

#[cfg(windows)]
fn append_log(dir: &std::path::Path, line: &str) {
    use std::io::Write;
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(spike_ctrl_alt_del::WATCHER_LOG))
    else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

#[cfg(windows)]
fn ensure_elevated() -> Result<(), Error> {
    use spike_ctrl_alt_del::{can_write_machine_policy, relaunch_elevated};

    if can_write_machine_policy() {
        return Ok(());
    }
    println!("This needs an Administrator window. Click Yes if Windows asks for permission.");
    println!("If Windows asks for a password instead of just Yes, click No and stop.");
    relaunch_elevated()?;
    std::process::exit(0);
}

#[cfg(windows)]
fn ensure_account() -> Result<(), Error> {
    spike_ctrl_alt_del::ensure_interactive_account()
}

#[cfg(windows)]
fn prepare_dir(dir: &std::path::Path) -> Result<(), Error> {
    std::fs::create_dir_all(dir)?;
    spike_ctrl_alt_del::lock_down_directory(dir)
}

#[cfg(windows)]
fn spawn_watcher(dir: &std::path::Path) -> Result<(), Error> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    use spike_ctrl_alt_del::{JOURNAL_ENV, default_journal_dir};

    if watcher_running(dir) {
        return Ok(());
    }
    let exe = std::env::current_exe()
        .map_err(|err| Error::new(format!("Could not find this program: {err}")))?;
    let mut command = Command::new(exe);
    command
        .arg("watch")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if dir != default_journal_dir() {
        command.env(JOURNAL_ENV, dir);
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
fn show_and_pause(dir: &std::path::Path, text: &str) -> Result<(), Error> {
    use spike_ctrl_alt_del::RESTORE_RESULT;

    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(RESTORE_RESULT);
    std::fs::write(&path, text)?;
    println!("{text}");
    println!("A copy of this result is in {}", path.display());
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        println!("Press Enter to close this window.");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
    Ok(())
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
