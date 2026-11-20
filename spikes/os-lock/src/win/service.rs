use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};
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
    CreateEventW, GetCurrentProcess, OpenProcessToken, SetEvent, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use super::measure;
use super::session::{self, console_session};
use super::util::{self, SERVICE_NAME, explain, wide};

static STATUS_HANDLE: AtomicIsize = AtomicIsize::new(0);
static STOP_EVENT: AtomicIsize = AtomicIsize::new(0);
static OUTPUT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn install(args: &[String]) -> u8 {
    let dir = match util::flag_path(args, "--output-dir") {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("create {}: {error}", dir.display());
        return 1;
    }
    let _ = std::fs::remove_file(dir.join("ready"));
    let _ = std::fs::remove_file(dir.join("service.json"));
    let _ = std::fs::remove_file(dir.join("request"));
    let _ = std::fs::remove_file(dir.join("stop"));
    match exercise_install(&dir) {
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
    let dir = match util::flag_path(args, "--output-dir") {
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
    let dir = match util::flag_path(&args, "--output-dir") {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let _ = OUTPUT.set(dir);
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
    let Some(dir) = OUTPUT.get() else {
        return;
    };
    let _ = enable_privilege(SE_TCB_NAME);
    if session::school_workstation() {
        return;
    }
    if notify_ready(dir).is_err() {
        return;
    }
    let _ = util::write_text(&dir.join("ready"), "ready\n");
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline && !stop_requested(dir) {
        if dir.join("request").is_file() && !dir.join("service.json").is_file() {
            let console = console_session();
            let report = measure::measure_disconnect(console, reconnect_enabled(dir));
            let _ = measure::publish(&dir.join("service.json"), &report);
            let _ = std::fs::remove_file(dir.join("request"));
        } else if reconnect_enabled(dir)
            && !dir.join("hold").is_file()
            && session::session_is_disconnected(console_session())
        {
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
    dir.join("reconnect.on").is_file() && !dir.join("reconnect.off").is_file()
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

fn exercise_install(dir: &Path) -> Result<(), String> {
    let scm = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) }
        .map_err(|error| explain(error, "OpenSCManagerW"))?;
    let name = wide(SERVICE_NAME);
    remove_existing(scm, &name);
    let binary = service_command(dir)?;
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

fn service_command(dir: &Path) -> Result<String, String> {
    let exe = util::exe_path()?;
    Ok(format!(
        "\"{}\" service --output-dir \"{}\"",
        exe.display(),
        dir.display()
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
