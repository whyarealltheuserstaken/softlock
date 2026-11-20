use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_NOT_ALL_ASSIGNED, ERROR_SERVICE_DOES_NOT_EXIST,
    ERROR_SERVICE_MARKED_FOR_DELETE, GetLastError, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
    SE_TCB_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
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
    CreateEventW, GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_SYNCHRONIZE, SetEvent,
    WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use super::measure;
use super::session::{self, console_session};
use super::util::{self, PROBE_DIR, SERVICE_NAME, explain, wide};
use crate::hosted;

static STATUS_HANDLE: AtomicIsize = AtomicIsize::new(0);
static STOP_EVENT: AtomicIsize = AtomicIsize::new(0);
static WATCH_PID: AtomicU32 = AtomicU32::new(0);
static CI_RECONNECT: AtomicBool = AtomicBool::new(false);

pub fn install(args: &[String]) -> u8 {
    let watch = match watch_pid(args) {
        Ok(pid) => pid,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let dir = match probe_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    if let Err(error) = exe_is_in_probe_dir() {
        eprintln!("{error}");
        return 2;
    }
    let _ = std::fs::remove_file(dir.join("ready"));
    let _ = std::fs::remove_file(dir.join("service.json"));
    let _ = std::fs::remove_file(dir.join("request"));
    let _ = std::fs::remove_file(dir.join("stop"));
    let _ = std::fs::remove_file(dir.join("reconnect.on"));
    match exercise_install(watch, hosted_now()) {
        Ok(()) => {
            if wait_for_file(&dir.join("ready"), Duration::from_secs(20)) {
                0
            } else {
                eprintln!("service did not become ready");
                1
            }
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

pub fn uninstall() -> u8 {
    match exercise_remove() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

pub fn request(args: &[String]) -> u8 {
    let _ = args;
    let dir = match probe_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let result = dir.join("service.json");
    let _ = std::fs::remove_file(&result);
    if let Err(error) = util::write_text(&dir.join("request"), "disconnect\n") {
        eprintln!("{error}");
        return 1;
    }
    if !wait_for_file(&result, Duration::from_secs(20)) {
        eprintln!("service disconnect did not report");
        return 1;
    }
    match std::fs::read_to_string(&result) {
        Ok(text) => {
            println!("{text}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            0
        }
        Err(error) => {
            eprintln!("read service report: {error}");
            1
        }
    }
}

pub fn run() -> u8 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let watch = match watch_pid(&args) {
        Ok(pid) => pid,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    WATCH_PID.store(watch, Ordering::SeqCst);
    CI_RECONNECT.store(util::has_flag(&args, "--ci-reconnect"), Ordering::SeqCst);
    if probe_dir().is_err() {
        eprintln!("refusing: probe directory is not a real directory");
        return 2;
    }
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
    let Ok(dir) = probe_dir() else {
        return;
    };
    let _ = enable_privilege(SE_TCB_NAME);
    if session::school_workstation() {
        return;
    }
    if notify_ready(&dir).is_err() {
        return;
    }
    let _ = util::write_text(&dir.join("ready"), "ready\n");
    while !stop_requested(&dir) {
        if probe_dir().is_err() {
            delete_own_service();
            return;
        }
        if !watch_alive() {
            delete_own_service();
            return;
        }
        if dir.join("request").is_file() && !dir.join("service.json").is_file() {
            let console = console_session();
            let report = measure::measure_disconnect(console, reconnect_enabled(&dir));
            let _ = measure::publish(&dir.join("service.json"), &report);
            let _ = std::fs::remove_file(dir.join("request"));
        } else if reconnect_enabled(&dir) && session::session_is_disconnected(console_session()) {
            let _ = reconnect(console_session());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn notify_ready(dir: &Path) -> Result<(), String> {
    let _ = dir;
    super::notify::arm()
}

fn reconnect_enabled(dir: &Path) -> bool {
    CI_RECONNECT.load(Ordering::SeqCst) && dir.join("reconnect.on").is_file()
}

fn stop_requested(dir: &Path) -> bool {
    if dir.join("stop").is_file() {
        return true;
    }
    let event = stop_event();
    if event.0.is_null() {
        return false;
    }
    unsafe { WaitForSingleObject(event, 0) == WAIT_OBJECT_0 }
}

fn reconnect(session: u32) -> Result<(), String> {
    if session::connect_session(session) {
        Ok(())
    } else {
        Err("session did not become active".to_string())
    }
}

fn exercise_install(watch: u32, ci_reconnect: bool) -> Result<(), String> {
    let scm = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) }
        .map_err(|error| explain(error, "OpenSCManagerW"))?;
    let name = wide(SERVICE_NAME);
    remove_existing(scm, &name);
    let binary = service_command(watch, ci_reconnect)?;
    let binary_wide = wide(&binary);
    let display = wide("OS lock probe");
    let service = create_service(scm, &name, &display, &binary_wide)?;
    let started = unsafe { StartServiceW(service, None) };
    close_service(service);
    close_scm(scm);
    started.map_err(|error| explain(error, "StartServiceW"))?;
    Ok(())
}

fn exercise_remove() -> Result<(), String> {
    let scm = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) }
        .map_err(|error| explain(error, "OpenSCManagerW"))?;
    let name = wide(SERVICE_NAME);
    remove_existing(scm, &name);
    close_scm(scm);
    Ok(())
}

fn service_command(watch: u32, ci_reconnect: bool) -> Result<String, String> {
    let exe = util::exe_path()?;
    let mut command = format!("\"{}\" service --watch-pid {watch}", exe.display());
    if ci_reconnect {
        command.push_str(" --ci-reconnect");
    }
    Ok(command)
}

fn hosted_now() -> bool {
    hosted::hosted_runner(
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
        std::env::var("RUNNER_ENVIRONMENT").ok().as_deref(),
    )
}

fn watch_pid(args: &[String]) -> Result<u32, String> {
    let text = util::flag_value(args, "--watch-pid")?;
    let pid = text
        .parse::<u32>()
        .map_err(|_| "watch pid is not a number".to_string())?;
    if pid == 0 {
        return Err("watch pid is missing".to_string());
    }
    Ok(pid)
}

fn probe_dir() -> Result<PathBuf, String> {
    let path = PathBuf::from(PROBE_DIR);
    if util::path_is_reparse(Path::new(r"C:\ProgramData")) || util::path_is_reparse(&path) {
        return Err("probe directory is a reparse point".to_string());
    }
    if !path.is_dir() {
        return Err("probe directory is missing".to_string());
    }
    util::require_trusted_owner(&path)?;
    Ok(path)
}

fn exe_is_in_probe_dir() -> Result<(), String> {
    let exe = util::exe_path()?;
    if util::path_is_reparse(&exe) {
        return Err("service binary is a reparse point".to_string());
    }
    util::require_trusted_owner(&exe)?;
    let dir = probe_dir()?;
    let Some(parent) = exe.parent() else {
        return Err("service binary is not in the probe directory".to_string());
    };
    if !parent
        .to_string_lossy()
        .eq_ignore_ascii_case(&dir.to_string_lossy())
    {
        return Err("service binary is not in the probe directory".to_string());
    }
    Ok(())
}

fn watch_alive() -> bool {
    let pid = WATCH_PID.load(Ordering::SeqCst);
    if pid == 0 {
        return false;
    }
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else {
            return false;
        };
        let state = WaitForSingleObject(handle, 0);
        let _ = CloseHandle(handle);
        state != WAIT_OBJECT_0
    }
}

fn delete_own_service() {
    let Ok(scm) =
        (unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) })
    else {
        return;
    };
    let name = wide(SERVICE_NAME);
    let Ok(service) = (unsafe { OpenServiceW(scm, PCWSTR(name.as_ptr()), SERVICE_ALL_ACCESS) })
    else {
        close_scm(scm);
        return;
    };
    let _ = delete_service(service);
    close_scm(scm);
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
                let marked = util::win32_code(&error) == ERROR_SERVICE_MARKED_FOR_DELETE.0;
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
    if let Err(error) = unsafe { ControlService(service, SERVICE_CONTROL_STOP, &mut status) }
        && util::win32_code(&error) != 1062
        && !current_stopped(service)
    {
        return false;
    }
    wait_stopped(service, Duration::from_secs(20))
}

fn delete_service(service: SC_HANDLE) -> bool {
    let deleted = unsafe { DeleteService(service) };
    unsafe {
        let _ = CloseServiceHandle(service);
    }
    match deleted {
        Ok(()) => true,
        Err(error) => {
            let code = util::win32_code(&error);
            code == ERROR_SERVICE_MARKED_FOR_DELETE.0 || code == ERROR_SERVICE_DOES_NOT_EXIST.0
        }
    }
}

fn wait_stopped(service: SC_HANDLE, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if current_stopped(service) {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    current_stopped(service)
}

fn current_stopped(service: SC_HANDLE) -> bool {
    let mut status = SERVICE_STATUS::default();
    unsafe {
        QueryServiceStatus(service, &mut status).is_ok() && status.dwCurrentState == SERVICE_STOPPED
    }
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
        if last == ERROR_NOT_ALL_ASSIGNED {
            return Err("privilege is not present on the token".to_string());
        }
        Ok(())
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

fn service_name_ptr() -> PCWSTR {
    static NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let name = NAME.get_or_init(|| wide(SERVICE_NAME));
    PCWSTR(name.as_ptr())
}

fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if path.is_file() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    path.is_file()
}
