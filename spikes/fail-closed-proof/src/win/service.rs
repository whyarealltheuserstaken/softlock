//! Session-0 probe. It disconnects the console session and times the notification.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_MARKED_FOR_DELETE, GetLastError,
    HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
    SE_TCB_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId;
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, CreateServiceW, DeleteService, OpenSCManagerW,
    OpenServiceW, QueryServiceStatus, RegisterServiceCtrlHandlerExW, SC_HANDLE,
    SC_MANAGER_ALL_ACCESS, SERVICE_ACCEPT_STOP, SERVICE_ALL_ACCESS, SERVICE_CONTROL_STOP,
    SERVICE_DEMAND_START, SERVICE_ERROR_NORMAL, SERVICE_RUNNING, SERVICE_START_PENDING,
    SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STOP_PENDING, SERVICE_STOPPED,
    SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS, SetServiceStatus, StartServiceCtrlDispatcherW,
    StartServiceW,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_SYNCHRONIZE,
    WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use crate::protocol::{
    COMMAND_FILE, Command, CommandResult, OP_DISCONNECT_CONSOLE, OP_STOP, OP_WATCH_USER,
    READY_FILE, RESULT_FILE, TIMING_FILE, Timing, WATCHING_FILE,
};
use crate::verdict::WTS_CONSOLE_DISCONNECT;

use super::args::Args;
use super::notify::{self, Note};
use super::session;
use super::util::{self, SERVICE_NAME, explain, log_line, wide, win32_code};

const STOP_WAIT_HINT_MS: u32 = 15_000;

static STATUS_HANDLE: AtomicIsize = AtomicIsize::new(0);
static STOP_EVENT: AtomicIsize = AtomicIsize::new(0);

pub fn run_as_service() -> i32 {
    let mut name = wide(SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR(name.as_mut_ptr()),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW::default(),
    ];
    match unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{}", explain(error, "StartServiceCtrlDispatcherW"));
            1
        }
    }
}

pub fn install_and_start(args: &Args) -> Result<(), String> {
    let scm = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) }
        .map_err(|error| explain(error, "OpenSCManagerW"))?;
    let name = wide(SERVICE_NAME);
    remove_existing(scm, &name);
    let binary = service_command(args)?;
    let binary_wide = wide(&binary);
    let display = wide("Runner fail-closed probe");
    let service = create_service(scm, &name, &display, &binary_wide)?;
    let started = unsafe { StartServiceW(service, None) };
    close_service(service);
    close_scm(scm);
    started.map_err(|error| explain(error, "StartServiceW"))?;
    Ok(())
}

pub fn stop_and_delete() {
    let Ok(scm) =
        (unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) })
    else {
        return;
    };
    let name = wide(SERVICE_NAME);
    remove_existing(scm, &name);
    close_scm(scm);
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    unsafe {
        let name = service_name_ptr();
        if let Ok(handle) = RegisterServiceCtrlHandlerExW(name, Some(control_handler), None) {
            STATUS_HANDLE.store(handle.0 as isize, Ordering::SeqCst);
        }
        if let Ok(event) =
            windows::Win32::System::Threading::CreateEventW(None, true, false, PCWSTR::null())
        {
            STOP_EVENT.store(event.0 as isize, Ordering::SeqCst);
        }
        set_status(SERVICE_RUNNING, SERVICE_ACCEPT_STOP, 0);
        let _ = std::panic::catch_unwind(service_body);
        set_status(SERVICE_STOPPED, 0, 0);
        let event = stop_event();
        if !event.0.is_null() {
            let _ = CloseHandle(event);
        }
    }
}

unsafe extern "system" fn control_handler(
    control: u32,
    _event_type: u32,
    _data: *mut core::ffi::c_void,
    _context: *mut core::ffi::c_void,
) -> u32 {
    unsafe {
        if control == SERVICE_CONTROL_STOP {
            set_status(SERVICE_STOP_PENDING, 0, 0);
            let event = stop_event();
            if !event.0.is_null() {
                let _ = windows::Win32::System::Threading::SetEvent(event);
            }
        }
        0
    }
}

fn service_body() {
    let args = match super::args::parse() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            return;
        }
    };
    let log = args.output_dir.join("service.log");
    log_line(&log, "service body starting");
    if let Err(error) = enable_privilege(SE_TCB_NAME) {
        log_line(&log, &format!("SeTcbPrivilege: {error}"));
    } else {
        log_line(&log, "SeTcbPrivilege: enabled");
    }
    let session = util::current_session();
    log_line(
        &log,
        &format!(
            "service session={session} pid={} console={}",
            unsafe { GetCurrentProcessId() },
            unsafe { WTSGetActiveConsoleSessionId() }
        ),
    );
    let listener = match notify::start(true) {
        Ok(listener) => listener,
        Err(error) => {
            log_line(&log, &format!("listener: {error}"));
            return;
        }
    };
    let ready = crate::protocol::Ready {
        session,
        pid: unsafe { GetCurrentProcessId() },
    };
    if let Err(error) = util::write_json(&args.output_dir.join(READY_FILE), &ready) {
        log_line(&log, &format!("ready: {error}"));
        return;
    }
    let parent = if args.parent_pid == 0 {
        HANDLE::default()
    } else {
        unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, args.parent_pid) }.unwrap_or_default()
    };
    let started = Instant::now();
    loop {
        if stop_requested() || parent_exited(parent) || started.elapsed() > Duration::from_secs(600)
        {
            log_line(&log, "service loop stopping");
            break;
        }
        dump_notes(&log);
        if let Some(command) = take_command(&args.output_dir) {
            log_line(&log, &format!("command {} {}", command.id, command.op));
            let result = match command.op.as_str() {
                OP_DISCONNECT_CONSOLE => disconnect_console(&command, &log),
                OP_WATCH_USER => watch_user(&command, &args, &log),
                OP_STOP => break,
                other => CommandResult {
                    id: command.id,
                    op: other.to_string(),
                    api_ok: false,
                    win32: 0,
                    error: Some(format!("unknown command {other}")),
                    session: 0,
                    t0: 0,
                    t1: 0,
                    freq: util::qpc_freq(),
                    notification: None,
                    notifications: Vec::new(),
                    reconnect_ok: false,
                    reconnect: "not-tried".to_string(),
                },
            };
            if let Err(error) = util::write_json(&args.output_dir.join(RESULT_FILE), &result) {
                log_line(&log, &format!("result: {error}"));
            }
        }
        thread::sleep(Duration::from_millis(15));
    }
    if !parent.0.is_null() {
        unsafe {
            let _ = CloseHandle(parent);
        }
    }
    listener.stop();
    log_line(&log, "service body finished");
}

fn disconnect_console(command: &Command, log: &Path) -> CommandResult {
    let session = unsafe { WTSGetActiveConsoleSessionId() };
    let freq = util::qpc_freq();
    let t0 = util::qpc();
    let api = session::disconnect(session, true);
    let note = await_note(WTS_CONSOLE_DISCONNECT, session, t0);
    let notes = notify::since(t0);
    log_notes(log, &notes);
    finish(
        command,
        Observed {
            session,
            t0,
            freq,
            api,
            note,
            notes,
            reconnect_ok: false,
            reconnect: "left-to-user-session".to_string(),
        },
    )
}

fn watch_user(command: &Command, args: &Args, log: &Path) -> CommandResult {
    let watching = args.output_dir.join(WATCHING_FILE);
    let _ = std::fs::write(&watching, b"watching\n");
    let freq = util::qpc_freq();
    let start = Instant::now();
    let mut timing: Option<Timing> = None;
    let mut note: Option<Note> = None;
    while start.elapsed() < Duration::from_secs(20) {
        if stop_requested() {
            break;
        }
        if timing.is_none() {
            timing = util::read_json(&args.output_dir.join(TIMING_FILE)).ok();
        }
        if let Some(timing) = &timing
            && let Some(found) = notify::find(WTS_CONSOLE_DISCONNECT, command.session, timing.t0)
        {
            note = Some(found);
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let notes = timing
        .as_ref()
        .map(|timing| notify::since(timing.t0))
        .unwrap_or_default();
    log_notes(log, &notes);
    let _ = std::fs::remove_file(&watching);
    finish(
        command,
        Observed {
            session: command.session,
            t0: timing.as_ref().map(|timing| timing.t0).unwrap_or(0),
            freq,
            api: Ok(()),
            note,
            notes,
            reconnect_ok: false,
            reconnect: "left-to-user-session".to_string(),
        },
    )
}

fn await_note(kind: u32, session: u32, t0: u64) -> Option<Note> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(900) {
        if let Some(note) = notify::find(kind, session, t0) {
            return Some(note);
        }
        thread::sleep(Duration::from_millis(5));
    }
    notify::find(kind, session, t0)
}

struct Observed {
    session: u32,
    t0: u64,
    freq: u64,
    api: Result<(), String>,
    note: Option<Note>,
    notes: Vec<Note>,
    reconnect_ok: bool,
    reconnect: String,
}

fn finish(command: &Command, observed: Observed) -> CommandResult {
    let (api_ok, win32, error) = match &observed.api {
        Ok(()) => (true, 0, None),
        Err(error) => (false, session::win32_of(error), Some(error.clone())),
    };
    let t1 = observed.note.map(|note| note.qpc).unwrap_or(0);
    CommandResult {
        id: command.id,
        op: command.op.clone(),
        api_ok,
        win32,
        error,
        session: observed.session,
        t0: observed.t0,
        t1,
        freq: observed.freq,
        notification: observed.note.map(|note| note.kind),
        notifications: observed.notes.iter().map(|note| note.kind).collect(),
        reconnect_ok: observed.reconnect_ok,
        reconnect: observed.reconnect,
    }
}

fn take_command(dir: &Path) -> Option<Command> {
    let path = dir.join(COMMAND_FILE);
    let command = util::read_json::<Command>(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(dir.join(RESULT_FILE));
    Some(command)
}

fn dump_notes(log: &Path) {
    let notes = notify::drain();
    log_notes(log, &notes);
}

fn log_notes(log: &Path, notes: &[Note]) {
    for note in notes {
        log_line(
            log,
            &format!(
                "note kind={} name={} session={} qpc={}",
                note.kind,
                crate::verdict::notification_name(note.kind),
                note.session,
                note.qpc
            ),
        );
    }
}

fn parent_exited(parent: HANDLE) -> bool {
    if parent.0.is_null() {
        return false;
    }
    (unsafe { WaitForSingleObject(parent, 0) }) == WAIT_OBJECT_0
}

fn stop_requested() -> bool {
    let event = stop_event();
    if event.0.is_null() {
        return false;
    }
    (unsafe { WaitForSingleObject(event, 0) }) == WAIT_OBJECT_0
}

fn service_command(args: &Args) -> Result<String, String> {
    let exe = util::exe_path()?;
    let parent = unsafe { GetCurrentProcessId() };
    Ok(format!(
        "\"{}\" --service --output-dir \"{}\" --runner-image \"{}\" --parent-pid {parent}",
        exe.display(),
        args.output_dir.display(),
        args.runner_image
    ))
}

fn enable_privilege(name: PCWSTR) -> Result<(), String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|error| explain(error, "OpenProcessToken"))?;
        let mut luid = windows::Win32::Foundation::LUID::default();
        if let Err(error) = LookupPrivilegeValueW(PCWSTR::null(), name, &mut luid) {
            let _ = CloseHandle(token);
            return Err(explain(error, "LookupPrivilegeValueW"));
        }
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let adjusted = AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None);
        let last = GetLastError();
        let _ = CloseHandle(token);
        adjusted.map_err(|error| explain(error, "AdjustTokenPrivileges"))?;
        if last == windows::Win32::Foundation::ERROR_NOT_ALL_ASSIGNED {
            return Err("privilege is not present on the token".to_string());
        }
        Ok(())
    }
}

fn create_service(
    scm: SC_HANDLE,
    name: &[u16],
    display: &[u16],
    binary: &[u16],
) -> Result<SC_HANDLE, String> {
    let mut last = String::new();
    for _ in 0..10 {
        match unsafe {
            CreateServiceW(
                scm,
                PCWSTR(name.as_ptr()),
                PCWSTR(display.as_ptr()),
                SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_DEMAND_START,
                SERVICE_ERROR_NORMAL,
                PCWSTR(binary.as_ptr()),
                PCWSTR::null(),
                None,
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
            )
        } {
            Ok(service) => return Ok(service),
            Err(error) => {
                let marked = win32_code(&error) == ERROR_SERVICE_MARKED_FOR_DELETE.0;
                last = explain(error, "CreateServiceW");
                if !marked {
                    return Err(last);
                }
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
    Err(last)
}

fn remove_existing(scm: SC_HANDLE, name: &[u16]) {
    let Ok(service) = (unsafe { OpenServiceW(scm, PCWSTR(name.as_ptr()), SERVICE_ALL_ACCESS) })
    else {
        return;
    };
    let _ = stop_service(service);
    let _ = delete_service(service);
}

fn stop_service(service: SC_HANDLE) -> bool {
    let mut status = SERVICE_STATUS::default();
    let control = unsafe { ControlService(service, SERVICE_CONTROL_STOP, &mut status) };
    if let Err(error) = control
        && win32_code(&error) != 1062
        && current_state(service) != Some(SERVICE_STOPPED)
    {
        return false;
    }
    wait_state(service, SERVICE_STOPPED, Duration::from_secs(20))
}

fn delete_service(service: SC_HANDLE) -> bool {
    let deleted = unsafe { DeleteService(service) };
    close_service(service);
    match deleted {
        Ok(()) => true,
        Err(error) => {
            win32_code(&error) == ERROR_SERVICE_MARKED_FOR_DELETE.0
                || win32_code(&error) == ERROR_SERVICE_DOES_NOT_EXIST.0
        }
    }
}

fn wait_state(
    service: SC_HANDLE,
    wanted: windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE,
    timeout: Duration,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if current_state(service) == Some(wanted) {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    current_state(service) == Some(wanted)
}

fn current_state(
    service: SC_HANDLE,
) -> Option<windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE> {
    let mut status = SERVICE_STATUS::default();
    unsafe { QueryServiceStatus(service, &mut status).ok()? };
    Some(status.dwCurrentState)
}

fn close_service(service: SC_HANDLE) {
    unsafe {
        let _ = CloseServiceHandle(service);
    }
}

fn close_scm(scm: SC_HANDLE) {
    unsafe {
        let _ = CloseServiceHandle(scm);
    }
}

fn set_status(
    state: windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE,
    controls: u32,
    exit_code: u32,
) {
    let raw = STATUS_HANDLE.load(Ordering::SeqCst);
    if raw == 0 {
        return;
    }
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: controls,
        dwWin32ExitCode: exit_code,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: if state == SERVICE_START_PENDING || state == SERVICE_STOP_PENDING {
            1
        } else {
            0
        },
        dwWaitHint: if state == SERVICE_START_PENDING || state == SERVICE_STOP_PENDING {
            STOP_WAIT_HINT_MS
        } else {
            0
        },
    };
    unsafe {
        let _ = SetServiceStatus(SERVICE_STATUS_HANDLE(raw as *mut _), &status);
    }
}

fn stop_event() -> HANDLE {
    HANDLE(STOP_EVENT.load(Ordering::SeqCst) as *mut _)
}

fn service_name_ptr() -> PCWSTR {
    static NAME: OnceLock<Vec<u16>> = OnceLock::new();
    let name = NAME.get_or_init(|| wide(SERVICE_NAME));
    PCWSTR(name.as_ptr())
}
