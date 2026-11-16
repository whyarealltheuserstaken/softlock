use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_MARKED_FOR_DELETE, HANDLE,
    WAIT_OBJECT_0,
};
use windows::Win32::Security::{SE_ASSIGNPRIMARYTOKEN_NAME, SE_INCREASE_QUOTA_NAME, SE_TCB_NAME};
use windows::Win32::System::RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken};
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
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, PROCESS_INFORMATION,
    STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use crate::protocol::{LaunchChild, LaunchReport, ROLE_ELEVATED, ROLE_MEDIUM, ROLE_SYSTEM};
use crate::verdict::{IL_HIGH, IL_MEDIUM};

use super::args::Args;
use super::token::{self, OwnedToken};
use super::util::{self, SERVICE_NAME, explain, log_line, wide, win32_code, write_json};

const STOP_WAIT_MS: u32 = 900_000;

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

pub fn start(args: &Args, port: u16) -> Result<(), String> {
    let scm = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) }
        .map_err(|error| explain(error, "OpenSCManagerW"))?;
    let name = wide(SERVICE_NAME);
    remove_existing(scm, &name);
    let command = service_command(args, port)?;
    let command_wide = wide(&command);
    let display = wide("Input hook probe");
    let service = create_service(scm, &name, &display, &command_wide)?;
    let started = unsafe { StartServiceW(service, None) };
    unsafe {
        let _ = CloseServiceHandle(service);
        let _ = CloseServiceHandle(scm);
    }
    started.map_err(|error| explain(error, "StartServiceW"))
}

pub fn stop() {
    let Ok(scm) =
        (unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) })
    else {
        return;
    };
    let name = wide(SERVICE_NAME);
    if let Ok(service) = unsafe { OpenServiceW(scm, PCWSTR(name.as_ptr()), SERVICE_ALL_ACCESS) } {
        let _ = stop_service(service);
        let _ = delete_service(service);
    }
    unsafe {
        let _ = CloseServiceHandle(scm);
    }
}

pub fn wait_for_launch(output: &Path, timeout: Duration) -> Result<LaunchReport, String> {
    let path = output.join("launch.json");
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(text) = util::read_text(&path) {
            return serde_json::from_str(&text).map_err(|error| format!("launch.json: {error}"));
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err("service did not write launch.json".to_string())
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    unsafe {
        if let Ok(handle) =
            RegisterServiceCtrlHandlerExW(service_name_ptr(), Some(control_handler), None)
        {
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
    let (mut report, mut processes) = match prepare_and_launch(&args, &log) {
        Ok(launched) => launched,
        Err(error) => (
            LaunchReport {
                console_session: unsafe { WTSGetActiveConsoleSessionId() },
                user_il: 0,
                elevation_type: 0,
                linked_il: None,
                children: Vec::new(),
                notes: Vec::new(),
                error: Some(error),
            },
            Vec::new(),
        ),
    };
    if let Err(failure) = write_json(&args.output_dir.join("launch.json"), &report) {
        log_line(&log, &format!("write launch report: {failure}"));
        report.error = Some(failure);
    } else {
        log_line(&log, "launch report written");
    }
    let event = stop_event();
    if !event.0.is_null() {
        let _ = unsafe { WaitForSingleObject(event, STOP_WAIT_MS) };
    }
    for process in processes.drain(..) {
        let state = unsafe { WaitForSingleObject(process.handle, 0) };
        if state != WAIT_OBJECT_0 {
            unsafe {
                let _ = TerminateProcess(process.handle, 1);
                let _ = WaitForSingleObject(process.handle, 2_000);
            }
        }
    }
    log_line(&log, "service stop wait finished");
}

struct Launched {
    handle: HANDLE,
}

impl Drop for Launched {
    fn drop(&mut self) {
        if !self.handle.0.is_null() {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

fn prepare_and_launch(args: &Args, log: &Path) -> Result<(LaunchReport, Vec<Launched>), String> {
    let mut notes = Vec::new();
    for (label, privilege) in [
        ("SeTcbPrivilege", SE_TCB_NAME),
        ("SeAssignPrimaryTokenPrivilege", SE_ASSIGNPRIMARYTOKEN_NAME),
        ("SeIncreaseQuotaPrivilege", SE_INCREASE_QUOTA_NAME),
    ] {
        match token::enable_privilege(privilege) {
            Ok(()) => {
                notes.push(format!("{label}: enabled"));
                log_line(log, &format!("privilege {label}: enabled"));
            }
            Err(error) => {
                notes.push(format!("{label}: {error}"));
                log_line(log, &format!("privilege {label}: {error}"));
            }
        }
    }
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    if console == 0 || console == u32::MAX {
        return Err(format!("no active console session ({console})"));
    }
    log_line(log, &format!("console session {console}"));
    let prepared = prepare_tokens(console, &mut notes, log)?;
    let mut children = Vec::new();
    let mut processes = Vec::new();
    for (role, source, il, token) in [
        (
            ROLE_MEDIUM,
            prepared.medium_source,
            prepared.medium_il,
            prepared.medium,
        ),
        (
            ROLE_ELEVATED,
            prepared.elevated_source,
            prepared.elevated_il,
            prepared.elevated,
        ),
        (
            ROLE_SYSTEM,
            "system-session".to_string(),
            prepared.system_il,
            prepared.system,
        ),
    ] {
        match launch_child(args, role, &token, log) {
            Ok((pid, handle)) => {
                log_line(log, &format!("launched {role} pid={pid} token={source}"));
                processes.push(Launched { handle });
                children.push(LaunchChild {
                    role: role.to_string(),
                    pid: Some(pid),
                    token: source,
                    il: Some(il),
                    error: None,
                });
            }
            Err(error) => {
                log_line(log, &format!("launch {role} failed: {error}"));
                children.push(LaunchChild {
                    role: role.to_string(),
                    pid: None,
                    token: source,
                    il: Some(il),
                    error: Some(error),
                });
            }
        }
    }
    let failed = children.iter().any(|child| child.error.is_some());
    Ok((
        LaunchReport {
            console_session: console,
            user_il: prepared.user_il,
            elevation_type: prepared.elevation_type,
            linked_il: prepared.linked_il,
            children,
            notes,
            error: if failed {
                Some("one or more hook processes failed to launch".to_string())
            } else {
                None
            },
        },
        processes,
    ))
}

struct Prepared {
    user_il: u32,
    elevation_type: i32,
    linked_il: Option<u32>,
    medium: OwnedToken,
    medium_il: u32,
    medium_source: String,
    elevated: OwnedToken,
    elevated_il: u32,
    elevated_source: String,
    system: OwnedToken,
    system_il: u32,
}

fn prepare_tokens(console: u32, notes: &mut Vec<String>, log: &Path) -> Result<Prepared, String> {
    let user = console_user_token(console)?;
    let user_primary = token::duplicate_primary(user.handle())?;
    drop(user);
    let user_il = token::integrity_rid(user_primary.handle())?;
    let elevation_type = token::elevation_type(user_primary.handle())?;
    notes.push(format!("user-il={user_il} elevation-type={elevation_type}"));
    log_line(
        log,
        &format!("user token il={user_il} elevation-type={elevation_type}"),
    );
    let linked = token::linked_token(user_primary.handle());
    let linked_il = match linked.as_ref() {
        Ok(token) => match token::integrity_rid(token.handle()) {
            Ok(il) => {
                notes.push(format!("linked-il={il}"));
                log_line(log, &format!("linked token il={il}"));
                Some(il)
            }
            Err(error) => {
                notes.push(format!("linked-il-error={error}"));
                None
            }
        },
        Err(error) => {
            notes.push(format!("linked-token={error}"));
            log_line(log, &format!("linked token: {error}"));
            None
        }
    };

    let elevated = pick_token(
        &user_primary,
        user_il,
        linked.as_ref().ok(),
        linked_il,
        IL_HIGH,
    )
    .ok_or_else(|| "no high-integrity token for the elevated probe".to_string())?;
    let medium = match pick_token(
        &user_primary,
        user_il,
        linked.as_ref().ok(),
        linked_il,
        IL_MEDIUM,
    ) {
        Some(token) => token,
        None => {
            let high_handle = if user_il == IL_HIGH {
                user_primary.handle()
            } else if linked_il == Some(IL_HIGH) {
                linked.as_ref().map_err(|error| error.clone())?.handle()
            } else {
                return Err("no high-integrity token to lower to medium".to_string());
            };
            let lowered = token::duplicate_at_medium(high_handle)?;
            notes.push("medium-token=integrity-label".to_string());
            (lowered, "integrity-label".to_string())
        }
    };
    let system_source = token::current_process_token()?;
    let system = token::duplicate_primary(system_source.handle())?;
    drop(system_source);
    token::set_session(system.handle(), console)?;
    let system_il = token::integrity_rid(system.handle()).unwrap_or(0);
    let medium_il = token::integrity_rid(medium.0.handle()).unwrap_or(0);
    let elevated_il = token::integrity_rid(elevated.0.handle()).unwrap_or(0);
    log_line(
        log,
        &format!(
            "prepared medium={} il={medium_il} elevated={} il={elevated_il} system-il={system_il}",
            medium.1, elevated.1
        ),
    );
    Ok(Prepared {
        user_il,
        elevation_type,
        linked_il,
        medium_il,
        medium_source: medium.1,
        medium: medium.0,
        elevated_il,
        elevated_source: elevated.1,
        elevated: elevated.0,
        system,
        system_il,
    })
}

fn pick_token(
    user: &OwnedToken,
    user_il: u32,
    linked: Option<&OwnedToken>,
    linked_il: Option<u32>,
    want: u32,
) -> Option<(OwnedToken, String)> {
    if user_il == want {
        return token::duplicate_primary(user.handle())
            .ok()
            .map(|token| (token, "user".to_string()));
    }
    if linked_il == Some(want)
        && let Some(linked) = linked
        && let Ok(token) = token::duplicate_primary(linked.handle())
    {
        return Some((token, "linked".to_string()));
    }
    None
}

fn console_user_token(session: u32) -> Result<OwnedToken, String> {
    let mut token = HANDLE::default();
    unsafe {
        WTSQueryUserToken(session, &mut token)
            .map_err(|error| explain(error, &format!("WTSQueryUserToken {session}")))?;
    }
    Ok(OwnedToken(token))
}

fn launch_child(
    args: &Args,
    role: &str,
    token: &OwnedToken,
    log: &Path,
) -> Result<(u32, HANDLE), String> {
    let exe = util::exe_path()?;
    let command = format!(
        "\"{}\" --role {role} --port {} --output-dir \"{}\" --runner-image \"{}\"",
        exe.display(),
        args.port,
        args.output_dir.display(),
        args.runner_image
    );
    log_line(log, &format!("command {command}"));
    let exe_wide = wide(&exe.display().to_string());
    let mut command_wide = wide(&command);
    let mut desktop = wide(r"winsta0\default");
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        lpDesktop: util::pwstr(&mut desktop),
        ..STARTUPINFOW::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessAsUserW(
            Some(token.handle()),
            PCWSTR(exe_wide.as_ptr()),
            Some(util::pwstr(&mut command_wide)),
            None,
            None,
            false,
            CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            None,
            PCWSTR::null(),
            &startup,
            &mut info,
        )
        .map_err(|error| explain(error, "CreateProcessAsUserW"))?;
        let _ = CloseHandle(info.hThread);
    }
    Ok((info.dwProcessId, info.hProcess))
}

fn service_command(args: &Args, port: u16) -> Result<String, String> {
    let exe = util::exe_path()?;
    Ok(format!(
        "\"{}\" --service --port {port} --output-dir \"{}\" --runner-image \"{}\"",
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

fn service_name_ptr() -> PCWSTR {
    static NAME: OnceLock<Vec<u16>> = OnceLock::new();
    let name = NAME.get_or_init(|| wide(SERVICE_NAME));
    PCWSTR(name.as_ptr())
}
