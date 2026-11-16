use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_NOT_ALL_ASSIGNED, ERROR_SERVICE_DOES_NOT_EXIST,
    ERROR_SERVICE_MARKED_FOR_DELETE, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::{
    AdjustTokenPrivileges, DuplicateTokenEx, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
    SE_ASSIGNPRIMARYTOKEN_NAME, SE_INCREASE_QUOTA_NAME, SE_PRIVILEGE_ENABLED, SE_TCB_NAME,
    SecurityImpersonation, TOKEN_ADJUST_PRIVILEGES, TOKEN_ALL_ACCESS, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_PRIVILEGES, TOKEN_QUERY, TokenPrimary,
};
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTS_SESSION_INFOW, WTSEnumerateSessionsW, WTSFreeMemory,
    WTSGetActiveConsoleSessionId, WTSQueryUserToken,
};
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
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateEventW, CreateProcessAsUserW,
    GetCurrentProcess, GetCurrentProcessId, GetExitCodeProcess, OpenProcess, OpenProcessToken,
    PROCESS_INFORMATION, PROCESS_QUERY_INFORMATION, STARTUPINFOW, SetEvent, TerminateProcess,
    WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use crate::report::{ServiceReport, SessionInfo};

use super::args::Args;
use super::diag;
use super::util::{self, SERVICE_NAME, explain, log_line, wide, win32_code, write_json};

const CHILD_WAIT_MS: u32 = 120_000;
const STOP_WAIT_MS: u32 = 60_000;

static STATUS_HANDLE: AtomicIsize = AtomicIsize::new(0);
static STOP_EVENT: AtomicIsize = AtomicIsize::new(0);

pub struct Exercise {
    pub registered: bool,
    pub started: bool,
    pub stopped: bool,
    pub deleted: bool,
    pub error: Option<String>,
}

pub fn run_as_service() -> i32 {
    let mut name = service_name_wide();
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

pub fn exercise(args: &Args) -> Exercise {
    let mut outcome = Exercise {
        registered: false,
        started: false,
        stopped: false,
        deleted: false,
        error: None,
    };
    let scm = match unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) }
    {
        Ok(scm) => scm,
        Err(error) => {
            outcome.error = Some(explain(error, "OpenSCManagerW"));
            return outcome;
        }
    };
    let name = wide(SERVICE_NAME);
    remove_existing(scm, &name);
    let binary = match service_command(args) {
        Ok(binary) => binary,
        Err(error) => {
            outcome.error = Some(error);
            close_scm(scm);
            return outcome;
        }
    };
    let binary_wide = wide(&binary);
    let display = wide("Runner desktop probe");
    let service = match create_service(scm, &name, &display, &binary_wide) {
        Ok(service) => service,
        Err(error) => {
            outcome.error = Some(error);
            close_scm(scm);
            return outcome;
        }
    };
    outcome.registered = true;
    match unsafe { StartServiceW(service, None) } {
        Ok(()) => outcome.started = true,
        Err(error) => {
            outcome.error = Some(explain(error, "StartServiceW"));
            outcome.deleted = delete_service(service);
            close_scm(scm);
            return outcome;
        }
    }
    wait_for_report(&args.output_dir, service, Duration::from_secs(150));
    outcome.stopped = stop_service(service);
    outcome.deleted = delete_service(service);
    close_scm(scm);
    if !outcome.stopped {
        outcome.error = Some(
            outcome
                .error
                .clone()
                .unwrap_or_else(|| "service did not reach stopped".to_string()),
        );
    }
    outcome
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    unsafe {
        let name = service_name_ptr();
        if let Ok(handle) = RegisterServiceCtrlHandlerExW(name, Some(control_handler), None) {
            STATUS_HANDLE.store(handle.0 as isize, Ordering::SeqCst);
        }
        if let Ok(event) = CreateEventW(None, true, false, PCWSTR::null()) {
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
                let _ = SetEvent(event);
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
    let mut privilege_notes = Vec::new();
    for (label, privilege) in [
        ("SeTcbPrivilege", SE_TCB_NAME),
        ("SeAssignPrimaryTokenPrivilege", SE_ASSIGNPRIMARYTOKEN_NAME),
        ("SeIncreaseQuotaPrivilege", SE_INCREASE_QUOTA_NAME),
    ] {
        if let Err(error) = enable_privilege(privilege) {
            privilege_notes.push(format!("{label}: {error}"));
            log_line(&log, &format!("privilege {label}: {error}"));
        } else {
            log_line(&log, &format!("privilege {label}: enabled"));
        }
    }
    let identity = diag::identity();
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    let sessions = enumerate_sessions();
    log_line(
        &log,
        &format!(
            "service session={} user={} console={console} sessions={}",
            identity.session_id,
            identity.username,
            sessions.len()
        ),
    );
    let launch = launch_child(&args, &log);
    let (token_source, child_pid, child_exit_code, launch_error) = match launch {
        Ok(child) => (
            child.token_source,
            Some(child.pid),
            Some(child.exit_code),
            None,
        ),
        Err(error) => ("none".to_string(), None, None, Some(error)),
    };
    let mut error = launch_error;
    if let Some(note) = privilege_notes.first()
        && let Some(existing) = error.take()
    {
        error = Some(format!("{existing}; {note}"));
    }
    if error.is_none() && !args.output_dir.join("interactive.json").exists() {
        error = Some(format!(
            "child exited {} without an interactive report",
            child_exit_code.unwrap_or(u32::MAX)
        ));
    }
    let report = ServiceReport {
        session_id: identity.session_id,
        pid: identity.pid,
        username: identity.username,
        console_session_id: console,
        sessions,
        token_source,
        child_pid,
        child_exit_code,
        error,
    };
    if let Err(failure) = write_json(&args.output_dir.join("service.json"), &report) {
        log_line(&log, &format!("write service report: {failure}"));
    } else {
        log_line(&log, "service report written");
    }
    let event = stop_event();
    if event.0.is_null() {
        return;
    }
    let _ = unsafe { WaitForSingleObject(event, STOP_WAIT_MS) };
    log_line(&log, "service stop wait finished");
}

struct Child {
    token_source: String,
    pid: u32,
    exit_code: u32,
}

fn launch_child(args: &Args, log: &std::path::Path) -> Result<Child, String> {
    let (token, token_source) = user_token(args.parent_pid)?;
    log_line(log, &format!("token source={token_source}"));
    let exe = util::exe_path()?;
    let exe_text = exe.display().to_string();
    let command = format!(
        "\"{exe_text}\" --interactive-proof --output-dir \"{}\" --runner-image \"{}\"",
        args.output_dir.display(),
        args.runner_image
    );
    log_line(log, &format!("launch {command}"));
    let exe_wide = wide(&exe_text);
    let mut command_wide = wide(&command);
    let mut desktop = wide(r"winsta0\default");
    let mut environment: *mut core::ffi::c_void = std::ptr::null_mut();
    let environment_result =
        unsafe { CreateEnvironmentBlock(&mut environment, Some(token), false) };
    if let Err(error) = environment_result {
        log_line(log, &explain(error, "CreateEnvironmentBlock"));
        environment = std::ptr::null_mut();
    }
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        lpDesktop: util::pwstr(&mut desktop),
        ..STARTUPINFOW::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    let created = unsafe {
        CreateProcessAsUserW(
            Some(token),
            PCWSTR(exe_wide.as_ptr()),
            Some(util::pwstr(&mut command_wide)),
            None,
            None,
            false,
            CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            if environment.is_null() {
                None
            } else {
                Some(environment.cast_const())
            },
            PCWSTR::null(),
            &startup,
            &mut info,
        )
    };
    if !environment.is_null() {
        unsafe {
            let _ = DestroyEnvironmentBlock(environment);
        }
    }
    unsafe {
        let _ = CloseHandle(token);
    }
    created.map_err(|error| explain(error, "CreateProcessAsUserW"))?;
    unsafe {
        let _ = CloseHandle(info.hThread);
    }
    let wait = unsafe { WaitForSingleObject(info.hProcess, CHILD_WAIT_MS) };
    if wait == WAIT_TIMEOUT {
        unsafe {
            let _ = TerminateProcess(info.hProcess, 1);
            let _ = WaitForSingleObject(info.hProcess, 5_000);
        }
    }
    let mut exit_code = u32::MAX;
    unsafe {
        let _ = GetExitCodeProcess(info.hProcess, &mut exit_code);
        let _ = CloseHandle(info.hProcess);
    }
    log_line(
        log,
        &format!(
            "child pid={} wait={:?} exit={exit_code}",
            info.dwProcessId, wait.0
        ),
    );
    if wait != WAIT_OBJECT_0 && wait != WAIT_TIMEOUT {
        return Err(format!("WaitForSingleObject returned {}", wait.0));
    }
    Ok(Child {
        token_source,
        pid: info.dwProcessId,
        exit_code,
    })
}

fn user_token(parent_pid: u32) -> Result<(HANDLE, String), String> {
    let mut errors = Vec::new();
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    if console != u32::MAX && console != 0 {
        match query_user_token(console) {
            Ok(token) => return Ok((token, "console-user".to_string())),
            Err(error) => errors.push(error),
        }
    }
    for session in enumerate_sessions() {
        if session.id == 0 || session.state != "active" {
            continue;
        }
        match query_user_token(session.id) {
            Ok(token) => return Ok((token, format!("session-{}", session.id))),
            Err(error) => errors.push(error),
        }
    }
    if parent_pid != 0 {
        match parent_token(parent_pid) {
            Ok(token) => return Ok((token, "parent-process".to_string())),
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Err("no interactive session token was available".to_string())
    } else {
        Err(errors.join("; "))
    }
}

fn query_user_token(session: u32) -> Result<HANDLE, String> {
    let mut token = HANDLE::default();
    unsafe {
        WTSQueryUserToken(session, &mut token)
            .map_err(|error| explain(error, &format!("WTSQueryUserToken {session}")))?
    };
    Ok(token)
}

fn parent_token(pid: u32) -> Result<HANDLE, String> {
    let session = process_session(pid)?;
    if session == 0 {
        return Err(format!("parent {pid} is in session 0"));
    }
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid)
            .map_err(|error| explain(error, "OpenProcess parent"))?;
        let mut token = HANDLE::default();
        let opened = OpenProcessToken(
            process,
            TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY,
            &mut token,
        );
        let _ = CloseHandle(process);
        opened.map_err(|error| explain(error, "OpenProcessToken parent"))?;
        let mut primary = HANDLE::default();
        let duplicated = DuplicateTokenEx(
            token,
            TOKEN_ALL_ACCESS,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        );
        let _ = CloseHandle(token);
        duplicated.map_err(|error| explain(error, "DuplicateTokenEx"))?;
        Ok(primary)
    }
}

fn process_session(pid: u32) -> Result<u32, String> {
    let mut session = 0u32;
    unsafe {
        ProcessIdToSessionId(pid, &mut session)
            .map_err(|error| explain(error, "ProcessIdToSessionId"))?
    };
    Ok(session)
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
        let looked = LookupPrivilegeValueW(PCWSTR::null(), name, &mut luid);
        if let Err(error) = looked {
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
        if last == ERROR_NOT_ALL_ASSIGNED {
            return Err("privilege is not present on the token".to_string());
        }
        Ok(())
    }
}

fn enumerate_sessions() -> Vec<SessionInfo> {
    let mut raw: *mut WTS_SESSION_INFOW = std::ptr::null_mut();
    let mut count = 0u32;
    let listed = unsafe { WTSEnumerateSessionsW(None, 0, 1, &mut raw, &mut count) };
    if listed.is_err() || raw.is_null() {
        return Vec::new();
    }
    let mut sessions = Vec::new();
    unsafe {
        for index in 0..count {
            let info = &*raw.add(index as usize);
            sessions.push(SessionInfo {
                id: info.SessionId,
                state: session_state(info.State.0),
                name: wide_ptr(info.pWinStationName),
            });
        }
        WTSFreeMemory(raw.cast());
    }
    sessions
}

fn session_state(state: i32) -> String {
    match state {
        0 => "active",
        1 => "connected",
        2 => "connect-query",
        3 => "shadow",
        4 => "disconnected",
        5 => "idle",
        6 => "listen",
        7 => "reset",
        8 => "down",
        9 => "init",
        other => return format!("state-{other}"),
    }
    .to_string()
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
    wait_state(service, SERVICE_STOPPED, Duration::from_secs(30))
}

fn delete_service(service: SC_HANDLE) -> bool {
    let deleted = unsafe { DeleteService(service) };
    unsafe {
        let _ = CloseServiceHandle(service);
    }
    match deleted {
        Ok(()) => true,
        Err(error) => {
            win32_code(&error) == ERROR_SERVICE_MARKED_FOR_DELETE.0
                || win32_code(&error) == ERROR_SERVICE_DOES_NOT_EXIST.0
        }
    }
}

fn wait_for_report(output: &Path, service: SC_HANDLE, timeout: Duration) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if output.join("service.json").exists() {
            return;
        }
        if current_state(service) == Some(SERVICE_STOPPED) {
            return;
        }
        thread::sleep(Duration::from_millis(250));
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
            15_000
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

fn service_name_wide() -> Vec<u16> {
    wide(SERVICE_NAME)
}

fn service_name_ptr() -> PCWSTR {
    static NAME: OnceLock<Vec<u16>> = OnceLock::new();
    let name = NAME.get_or_init(|| wide(SERVICE_NAME));
    PCWSTR(name.as_ptr())
}

fn wide_ptr(value: PWSTR) -> String {
    if value.0.is_null() {
        return String::new();
    }
    unsafe {
        let mut len = 0usize;
        while *value.0.add(len) != 0 && len < 256 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(value.0, len))
    }
}
