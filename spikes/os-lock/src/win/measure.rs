use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use windows::Win32::System::RemoteDesktop::WTSDisconnectSession;
use windows::Win32::System::Shutdown::LockWorkStation;

use super::notify::{self, Note};
use super::session::{self, console_session, own_session};
use super::util::{self, write_json};

#[derive(Serialize)]
pub struct Report {
    pub ok: bool,
    pub pass: bool,
    pub api_ok: bool,
    pub win32: u32,
    pub notified: bool,
    pub kind: String,
    pub elapsed_us: u64,
    pub session_locked: Option<bool>,
    pub reconnected: bool,
    pub error: String,
}

pub fn lock(mode: &str) -> u8 {
    emit_report(&measure(mode, false, call_lock))
}

pub fn own(args: &[String]) -> u8 {
    let reconnect = util::has_flag(args, "--reconnect");
    let mut report = measure("disconnect", false, || call_disconnect(own_session()));
    if reconnect {
        let session = own_session().unwrap_or_else(console_session);
        report.reconnected = session::wait_until_active(session, Duration::from_secs(8));
    }
    emit_report(&report)
}

/// Lock, confirmed by a notification or session state within one second.
/// Otherwise disconnect, confirmed the same way. The API return is ignored.
pub fn fail_closed() -> u8 {
    if let Err(error) = notify::arm() {
        return emit_report(&error_report(error));
    }
    let session = own_session().unwrap_or_else(console_session);
    let lock_report = measure("lock", false, call_lock);
    let mut disconnect_report = None;
    let outcome = crate::verdict::run_close_chain(
        || lock_report.pass,
        || {
            let report = measure("disconnect", false, || call_disconnect(Some(session)));
            let pass = report.pass;
            disconnect_report = Some(report);
            pass
        },
    );
    let mut chosen = if lock_report.pass {
        lock_report
    } else {
        disconnect_report.unwrap_or_else(|| error_report("fail-closed-failed".to_string()))
    };
    if outcome == crate::verdict::CloseOutcome::Failed {
        chosen.pass = false;
        chosen.error = "fail-closed-failed".to_string();
    }
    emit_report(&chosen)
}

pub fn measure_disconnect(session: u32, reconnect: bool) -> Report {
    let target = session;
    measure_after(
        "disconnect",
        reconnect,
        move || call_disconnect(Some(target)),
        move || {
            if reconnect {
                let _ = session::connect_session(target);
            }
        },
    )
}

fn measure(mode: &str, reconnect_planned: bool, call: impl FnOnce() -> (bool, u32)) -> Report {
    measure_after(mode, reconnect_planned, call, || {})
}

fn measure_after(
    mode: &str,
    reconnect_planned: bool,
    call: impl FnOnce() -> (bool, u32),
    after: impl FnOnce() + Send + 'static,
) -> Report {
    if let Err(error) = notify::arm() {
        return error_report(error);
    }
    notify::reset();
    let session = own_session().unwrap_or_else(console_session);
    let start = Instant::now();
    let (api_ok, win32) = call();
    let worker = thread::spawn(after);
    let budget = Duration::from_millis(1000).saturating_sub(start.elapsed());
    let note = wait_signal(mode, session, budget);
    let elapsed_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
    let join_deadline = Instant::now() + Duration::from_secs(8);
    while !worker.is_finished() && Instant::now() < join_deadline {
        notify::pump();
        thread::sleep(Duration::from_millis(5));
    }
    if worker.is_finished() {
        let _ = worker.join();
    }
    report_from(
        mode,
        api_ok,
        win32,
        &note,
        elapsed_us,
        session,
        reconnect_planned,
    )
}

fn report_from(
    mode: &str,
    api_ok: bool,
    win32: u32,
    note: &Note,
    elapsed_us: u64,
    session: u32,
    reconnected: bool,
) -> Report {
    let session_locked = session::session_is_locked(session);
    let elapsed = u128::from(elapsed_us);
    let pass = match mode {
        "blocked" => crate::verdict::blocked_proven(
            api_ok,
            note.notified && note.kind == "lock",
            session_locked,
        ),
        "lock" => crate::verdict::confirmed_lock(note.kind, note.notified, session_locked, elapsed),
        "disconnect" => crate::verdict::confirmed_disconnect(
            note.kind,
            note.notified,
            session::session_is_disconnected(session),
            elapsed,
        ),
        _ => false,
    };
    Report {
        ok: true,
        pass,
        api_ok,
        win32,
        notified: note.notified,
        kind: note.kind.to_string(),
        elapsed_us,
        session_locked,
        reconnected,
        error: String::new(),
    }
}

fn wait_signal(mode: &str, session: u32, budget: Duration) -> Note {
    let start = Instant::now();
    loop {
        notify::pump();
        let note = notify::snapshot();
        if note.notified {
            return note;
        }
        if mode != "disconnect" && session::session_is_locked(session) == Some(true) {
            return Note {
                notified: true,
                kind: "lock",
            };
        }
        if mode == "disconnect" && session::session_is_disconnected(session) {
            return Note {
                notified: true,
                kind: "console-disconnect",
            };
        }
        if start.elapsed() >= budget {
            return notify::snapshot();
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn call_lock() -> (bool, u32) {
    match unsafe { LockWorkStation() } {
        Ok(()) => (true, 0),
        Err(error) => (false, util::win32_code(&error)),
    }
}

fn call_disconnect(session: Option<u32>) -> (bool, u32) {
    let Some(session) = session else {
        return (false, 0);
    };
    match unsafe { WTSDisconnectSession(None, session, false) } {
        Ok(()) => (true, 0),
        Err(error) => (false, util::win32_code(&error)),
    }
}

fn error_report(error: String) -> Report {
    Report {
        ok: false,
        pass: false,
        api_ok: false,
        win32: 0,
        notified: false,
        kind: "none".to_string(),
        elapsed_us: 0,
        session_locked: None,
        reconnected: false,
        error,
    }
}

fn emit_report(report: &Report) -> u8 {
    if crate::emit(report) { 0 } else { 1 }
}

pub fn publish(path: &Path, report: &Report) -> Result<(), String> {
    write_json(path, report)
}
