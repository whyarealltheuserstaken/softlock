//! User-session orchestrator. It applies the journaled policies and times the three paths.

use std::fs::File;
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::System::Console::{SetConsoleCtrlHandler, SetConsoleTitleW};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};
use windows::core::w;

use crate::guard::service_install_allowed;
use crate::protocol::{
    self, Beat, COMMAND_FILE, HEARTBEAT_FILE, HEARTBEAT_STOP, READY_FILE, REPORT_FILE, RESULT_FILE,
    TIMING_FILE, Timing, WATCHING_FILE, max_beat_gap, parse_beats,
};
use crate::report::render;
use crate::verdict::{self, ObservedPath, Outcome, WTS_SESSION_LOCK, elapsed_us, lock_blocked};

use super::args::Args;
use super::notify::Listener;
use super::policy;
use super::service;
use super::session;
use super::util::{self, log_line};

const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub fn dispatch(args: Args) -> i32 {
    if args.service {
        service::run_as_service()
    } else if args.guard {
        run_guard(&args)
    } else if args.revert {
        run_revert(&args)
    } else if args.heartbeat {
        run_heartbeat(&args)
    } else {
        run_proof(&args)
    }
}

fn run_revert(args: &Args) -> i32 {
    let path = policy::journal_path(&args.output_dir);
    match policy::restore_at(&path) {
        Ok(true) => {
            println!("policies-restored=true");
            0
        }
        Ok(false) => {
            println!("journal=absent");
            println!("policies-restored=true");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn run_guard(args: &Args) -> i32 {
    if args.parent_pid != 0
        && let Ok(parent) = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, args.parent_pid) }
    {
        unsafe {
            let _ = WaitForSingleObject(parent, 15 * 60 * 1000);
            let _ = windows::Win32::Foundation::CloseHandle(parent);
        }
    }
    let _ = std::fs::write(args.output_dir.join(HEARTBEAT_STOP), b"stop\n");
    match policy::restore_at(&policy::journal_path(&args.output_dir)) {
        Ok(_) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn run_heartbeat(args: &Args) -> i32 {
    unsafe {
        let _ = SetConsoleTitleW(w!("Fail-closed check - still running"));
    }
    let path = args.output_dir.join(HEARTBEAT_FILE);
    let stop = args.output_dir.join(HEARTBEAT_STOP);
    let mut seq = 0u64;
    while !stop.exists() {
        seq += 1;
        let line = format!("{} {seq}\n", util::unix_ms());
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
        println!("still running {seq}");
        let _ = std::io::stdout().flush();
        thread::sleep(Duration::from_millis(50));
    }
    0
}

fn run_proof(args: &Args) -> i32 {
    if let Err(reason) = service_install_allowed(
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
        args.allow_local_service,
    ) {
        eprintln!("{reason}");
        return 2;
    }
    if let Err(error) = std::fs::create_dir_all(&args.output_dir) {
        eprintln!("create output dir: {error}");
        return 1;
    }
    let log = args.output_dir.join("proof.log");
    let journal = policy::journal_path(&args.output_dir);
    policy::arm(journal.clone());
    install_ctrl_handler();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        policy::restore_armed();
        previous(info);
    }));
    let _restore_on_drop = RestoreOnDrop;
    if let Err(error) = policy::restore_at(&journal) {
        log_line(&log, &format!("leftover journal: {error}"));
        eprintln!("{error}");
        return 1;
    }
    log_line(&log, "proof starting");
    if let Err(error) = spawn_guard(args) {
        eprintln!("{error}");
        return 1;
    }
    if let Err(error) = spawn_heartbeat(args) {
        log_line(&log, &format!("heartbeat: {error}"));
    }
    let mut outcome = Outcome::failed(args.runner_image.clone(), "");
    outcome.error = None;
    outcome.session_id = util::current_session();
    let ran = measure(args, &log, &mut outcome);
    if let Err(error) = ran {
        log_line(&log, &format!("measure: {error}"));
        outcome.error = Some(error);
    }
    match policy::restore_at(&journal) {
        Ok(_) => outcome.policies_restored = true,
        Err(error) => {
            outcome.policies_restored = false;
            let text = format!("restore: {error}");
            outcome.error = Some(match outcome.error.take() {
                Some(existing) => format!("{existing}; {text}"),
                None => text,
            });
        }
    }
    let _ = std::fs::write(args.output_dir.join(HEARTBEAT_STOP), b"stop\n");
    service::stop_and_delete();
    let text = render(&outcome);
    let _ = std::fs::write(args.output_dir.join(protocol::REPORT_FILE), &text);
    println!("{text}");
    let _ = std::io::stdout().flush();
    if verdict::passed(&outcome) { 0 } else { 1 }
}

fn measure(args: &Args, log: &Path, outcome: &mut Outcome) -> Result<(), String> {
    if !policy::is_elevated()? {
        return Err("Run this from an Administrator terminal.".to_string());
    }
    let (document, mut store) = policy::apply_lockdown(&args.output_dir)?;
    outcome.policies = crate::journal::LOCKDOWN
        .iter()
        .map(|policy| policy.name.to_string())
        .collect();
    log_line(
        log,
        &format!(
            "policies applied for {} console={}",
            document.sid,
            session::console_session()
        ),
    );
    let listener = super::notify::start(false)?;
    service::install_and_start(args)?;
    let ready: protocol::Ready =
        wait_json(&args.output_dir.join(READY_FILE), Duration::from_secs(30))?;
    outcome.service_session_id = Some(ready.session);
    log_line(
        log,
        &format!("service session={} pid={}", ready.session, ready.pid),
    );
    if ready.session != 0 {
        return Err(format!(
            "the probe service is in session {}, not session 0",
            ready.session
        ));
    }
    let session = outcome.session_id;
    let before_negative = read_beat(&args.output_dir)?;
    let t0 = util::qpc();
    let locked = session::lock_workstation();
    let saw = listener
        .wait(WTS_SESSION_LOCK, session, t0, Duration::from_millis(1100))
        .is_some();
    outcome.lock_saw_notification = saw;
    outcome.lock_blocked = lock_blocked(locked.is_ok(), saw);
    outcome.lock_blocked_win32 = locked.as_ref().err().map(|error| session::win32_of(error));
    snapshot(args, outcome);
    log_line(
        log,
        &format!(
            "lock-while-policy api={} saw={saw} detail={}",
            locked.is_ok(),
            locked.as_ref().err().map(String::as_str).unwrap_or("ok")
        ),
    );
    if locked.is_ok() {
        return Err("LockWorkStation succeeded while DisableLockWorkstation was set".to_string());
    }
    let _ = before_negative;

    outcome.service_disconnect = Some(service_disconnect(args, log, &listener)?);
    snapshot(args, outcome);
    thread::sleep(Duration::from_millis(300));
    outcome.session_disconnect = Some(user_disconnect(args, log, session)?);
    snapshot(args, outcome);
    thread::sleep(Duration::from_millis(300));
    policy::clear_disable_lock(&args.output_dir, &document, &mut store)?;
    outcome.lock_cleared = true;
    log_line(log, "cleared DisableLockWorkstation");
    outcome.user_lock = Some(user_lock(args, log, &listener, session)?);
    snapshot(args, outcome);
    let _ = listener;
    Ok(())
}

fn service_disconnect(
    args: &Args,
    log: &Path,
    _listener: &Listener,
) -> Result<ObservedPath, String> {
    let before = read_beat(&args.output_dir)?;
    let result = call_service(args, protocol::OP_DISCONNECT_CONSOLE, 0)?;
    log_line(log, &format!("service disconnect {}", describe(&result)));
    if !result.reconnect_ok {
        match session::reconnect(result.session) {
            Ok(label) => log_line(log, &format!("user reconnect={label}")),
            Err(error) => log_line(log, &format!("user reconnect failed: {error}")),
        }
    }
    Ok(observed(&result, result.api_ok, before, &args.output_dir))
}

fn user_disconnect(args: &Args, log: &Path, session: u32) -> Result<ObservedPath, String> {
    let _ = std::fs::remove_file(args.output_dir.join(WATCHING_FILE));
    let _ = std::fs::remove_file(args.output_dir.join(TIMING_FILE));
    let _ = std::fs::remove_file(args.output_dir.join(RESULT_FILE));
    let command = protocol::Command {
        id: NEXT_ID.fetch_add(1, Ordering::SeqCst),
        op: protocol::OP_WATCH_USER.to_string(),
        session,
    };
    util::write_json(&args.output_dir.join(COMMAND_FILE), &command)?;
    wait_for_file(
        &args.output_dir.join(WATCHING_FILE),
        Duration::from_secs(10),
    )?;
    let before = read_beat(&args.output_dir)?;
    let timing = Timing {
        t0: util::qpc(),
        freq: util::qpc_freq(),
        session,
    };
    write_synced(&args.output_dir.join(TIMING_FILE), &timing)?;
    let api = session::disconnect(session, false);
    log_line(
        log,
        &format!(
            "user disconnect api={} detail={}",
            api.is_ok(),
            api.as_ref().err().map(String::as_str).unwrap_or("ok")
        ),
    );
    let mut result = wait_result(&args.output_dir, command.id, Duration::from_secs(20))?;
    result.api_ok = api.is_ok();
    if let Err(error) = &api {
        result.win32 = session::win32_of(error);
        result.error = Some(error.clone());
    }
    log_line(log, &format!("user disconnect {}", describe(&result)));
    if !result.reconnect_ok {
        match session::reconnect(session) {
            Ok(label) => log_line(log, &format!("user reconnect={label}")),
            Err(error) => log_line(log, &format!("user reconnect failed: {error}")),
        }
    }
    Ok(observed(&result, result.api_ok, before, &args.output_dir))
}

fn user_lock(
    args: &Args,
    log: &Path,
    listener: &Listener,
    session: u32,
) -> Result<ObservedPath, String> {
    let before = read_beat(&args.output_dir)?;
    let t0 = util::qpc();
    let freq = util::qpc_freq();
    let api = session::lock_workstation();
    let note = listener.wait(WTS_SESSION_LOCK, session, t0, Duration::from_millis(1500));
    log_line(
        log,
        &format!(
            "user lock api={} note={} detail={}",
            api.is_ok(),
            note.map(|note| note.kind).unwrap_or(0),
            api.as_ref().err().map(String::as_str).unwrap_or("ok")
        ),
    );
    let gap = wait_gap(&args.output_dir, before);
    Ok(ObservedPath {
        api_ok: api.is_ok(),
        notification: note.map(|note| note.kind),
        elapsed_us: note.and_then(|note| elapsed_us(t0, note.qpc, freq)),
        heartbeat_gap_ms: gap,
    })
}

fn observed(
    result: &protocol::CommandResult,
    api_ok: bool,
    before: Beat,
    dir: &Path,
) -> ObservedPath {
    ObservedPath {
        api_ok,
        notification: result.notification,
        elapsed_us: elapsed_us(result.t0, result.t1, result.freq),
        heartbeat_gap_ms: wait_gap(dir, before),
    }
}

fn call_service(args: &Args, op: &str, session: u32) -> Result<protocol::CommandResult, String> {
    let _ = std::fs::remove_file(args.output_dir.join(RESULT_FILE));
    let command = protocol::Command {
        id: NEXT_ID.fetch_add(1, Ordering::SeqCst),
        op: op.to_string(),
        session,
    };
    util::write_json(&args.output_dir.join(COMMAND_FILE), &command)?;
    wait_result(&args.output_dir, command.id, Duration::from_secs(25))
}

fn wait_result(dir: &Path, id: u64, timeout: Duration) -> Result<protocol::CommandResult, String> {
    let start = Instant::now();
    let path = dir.join(RESULT_FILE);
    while start.elapsed() < timeout {
        if let Ok(result) = util::read_json::<protocol::CommandResult>(&path)
            && result.id == id
        {
            return Ok(result);
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err(format!("timed out waiting for command {id}"))
}

fn wait_json<T: serde::de::DeserializeOwned>(path: &Path, timeout: Duration) -> Result<T, String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(value) = util::read_json(path) {
            return Ok(value);
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(format!("timed out waiting for {}", path.display()))
}

fn wait_for_file(path: &Path, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err(format!("timed out waiting for {}", path.display()))
}

fn read_beat(dir: &Path) -> Result<Beat, String> {
    let path = dir.join(HEARTBEAT_FILE);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let beat = parse_beats(&text);
            if beat.count > 0 {
                return Ok(beat);
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err("the background heartbeat did not start".to_string())
}

fn wait_gap(dir: &Path, before: Beat) -> Option<u128> {
    let path = dir.join(HEARTBEAT_FILE);
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(1500) {
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Some(gap) = max_beat_gap(&text, before)
        {
            return Some(gap);
        }
        thread::sleep(Duration::from_millis(20));
    }
    None
}

fn write_synced(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    let mut file =
        File::create(path).map_err(|error| format!("create {}: {error}", path.display()))?;
    file.write_all(&text)
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("flush {}: {error}", path.display()))?;
    Ok(())
}

fn snapshot(args: &Args, outcome: &Outcome) {
    let _ = std::fs::write(args.output_dir.join(REPORT_FILE), render(outcome));
}

fn describe(result: &protocol::CommandResult) -> String {
    format!(
        "api={} win32={} note={:?} notes={:?} reconnect={} t0={} t1={} freq={}",
        result.api_ok,
        result.win32,
        result.notification,
        result.notifications,
        result.reconnect,
        result.t0,
        result.t1,
        result.freq
    )
}

fn spawn_guard(args: &Args) -> Result<(), String> {
    let exe = util::exe_path()?;
    Command::new(exe)
        .args([
            "--guard",
            "--parent-pid",
            &unsafe { GetCurrentProcessId() }.to_string(),
            "--output-dir",
            &args.output_dir.display().to_string(),
        ])
        .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("spawn guard: {error}"))?;
    Ok(())
}

fn spawn_heartbeat(args: &Args) -> Result<(), String> {
    let exe = util::exe_path()?;
    let output = args.output_dir.display().to_string();
    let spawned = Command::new(&exe)
        .args(["--heartbeat", "--output-dir", &output])
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn();
    if spawned.is_ok() {
        return Ok(());
    }
    Command::new(exe)
        .args(["--heartbeat", "--output-dir", &output])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("spawn heartbeat: {error}"))?;
    Ok(())
}

fn install_ctrl_handler() {
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), true);
    }
}

unsafe extern "system" fn ctrl_handler(ctrl: u32) -> windows::core::BOOL {
    if matches!(ctrl, 0 | 1 | 2 | 5 | 6) {
        policy::restore_armed();
        if ctrl == 0 || ctrl == 1 {
            unsafe {
                windows::Win32::System::Threading::ExitProcess(3);
            }
        }
        windows::core::BOOL(1)
    } else {
        windows::core::BOOL(0)
    }
}

struct RestoreOnDrop;

impl Drop for RestoreOnDrop {
    fn drop(&mut self) {
        policy::restore_armed();
    }
}
