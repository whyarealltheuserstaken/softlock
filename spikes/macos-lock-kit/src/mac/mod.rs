//! macOS collection and the two lock paths. This module is the only place
//! that may call the private symbol or start the screen saver.

use std::ffi::{CStr, CString};
use std::io::{self, Read, Write};
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use softlock_macos::{
    CommandText, FailClosedEnd, LOGIN_FRAMEWORK_IMAGE, LockSetting, PRIVATE_LOCK_SYMBOL,
    SCREEN_SAVER_ENGINE_CANDIDATES, combine_lock_setting, fail_closed_after_error,
    interpret_screen_lock_status, parse_defaults_value, password_after_screensaver,
};

use crate::args::{Args, ExercisePath, Mode};
use crate::commands::{engine_path_allowed, observation_argv_allowed, read_only_argv_allowed};
use crate::helper::{COMPLETE_LINE, HELPER_ENV, HelperAction, helper_should_lock};
use crate::kit_gate::{KitInvocation, KitPermission, may_lock, permit};
use crate::lock_attempt::{Attempt, CallWait, InterruptPlan, interrupt_plan, judge};
use crate::redact::sanitize;
use crate::report::{
    ExerciseRecord, Facts, KitReport, MISSING_SYMBOL_CI_TEST, build_report, render,
    safety_violation,
};
use crate::signals::{self, detach_helper_session};
use crate::text::one_line;

const CALL_TIMEOUT: Duration = Duration::from_secs(3);
const LOCK_POLL: Duration = Duration::from_secs(2);
const SCREEN_LOCK_TIMEOUT: Duration = Duration::from_secs(3);

const CORE_GRAPHICS: &str = "/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics";
const CORE_FOUNDATION: &str = "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";

pub fn run(args: Args) -> i32 {
    let root = is_root();
    let github = std::env::var("GITHUB_ACTIONS").ok().as_deref() == Some("true");
    let parent_armed = std::env::var(HELPER_ENV).ok().as_deref() == Some("1");
    let invocation = match args.mode {
        Mode::ReadOnly => KitInvocation::ReadOnly,
        Mode::Exercise { acknowledged, .. } => KitInvocation::Exercise { acknowledged },
        Mode::Helper(_) => KitInvocation::Helper { parent_armed },
    };
    let permission = permit(invocation, github, root);
    if matches!(args.mode, Mode::ReadOnly) && args.mode.may_request_a_lock() {
        eprintln!("read-only mode requested a lock");
        return 1;
    }
    match args.mode {
        Mode::Helper(action) => helper(action, permission, &args.output_dir),
        Mode::ReadOnly => {
            if !matches!(permission, KitPermission::ReadOnly) {
                eprintln!("{}", refusal(permission));
                return 1;
            }
            publish(&args.output_dir, &build_report(&collect(root), None))
        }
        Mode::Exercise { which, .. } => {
            if !may_lock(permission) {
                eprintln!("{}", refusal(permission));
                return 1;
            }
            match which {
                ExercisePath::Private => exercise_private(&args.output_dir),
                ExercisePath::Documented => exercise_documented(&args.output_dir),
            }
        }
    }
}

fn refusal(permission: KitPermission) -> &'static str {
    match permission {
        KitPermission::Refuse(text) => text,
        KitPermission::ReadOnly | KitPermission::MayLock => "the kit refused this run",
    }
}

fn exercise_private(dir: &Path) -> i32 {
    let facts = collect(false);
    if !facts.symbol_present {
        let mut record = ExerciseRecord::blank(ExercisePath::Private.label());
        record.detail = format!(
            "symbol missing; this is a recorded miss, not a crash. CI test {MISSING_SYMBOL_CI_TEST} covers that chooser row"
        );
        return publish(dir, &build_report(&facts, Some(&record)));
    }
    let mut armed = arm_helper(HelperAction::Private, dir);
    let mut record = ExerciseRecord::blank(ExercisePath::Private.label());
    if signals::tripped() {
        return stop_for_signal(dir, &facts, &mut record, &mut armed);
    }
    note_private(&mut record, attempt_private());
    if signals::tripped() {
        return stop_for_signal(dir, &facts, &mut record, &mut armed);
    }
    if record.state != "os-locked" {
        apply_fallback(&facts, true, &mut record);
    }
    if signals::tripped() {
        return stop_for_signal(dir, &facts, &mut record, &mut armed);
    }
    ask_observations(&mut record);
    if signals::tripped() {
        return stop_for_signal(dir, &facts, &mut record, &mut armed);
    }
    let code = publish(dir, &build_report(&facts, Some(&record)));
    stand_down(&mut armed);
    exit_code(code, &record)
}

fn exercise_documented(dir: &Path) -> i32 {
    let facts = collect(false);
    if !immediate(&facts) {
        let mut record = ExerciseRecord::blank(ExercisePath::Documented.label());
        record.result = "refuse".to_string();
        record.documented_path = "refused".to_string();
        record.fail_closed = FailClosedEnd::NeverUnlocked.as_str().to_string();
        record.detail = "the screen lock setting is unknown or cannot require an immediate password, so the documented path was not started".to_string();
        return publish(dir, &build_report(&facts, Some(&record)));
    }
    let mut armed = arm_helper(HelperAction::Documented, dir);
    let mut record = ExerciseRecord::blank(ExercisePath::Documented.label());
    if signals::tripped() {
        return stop_for_signal(dir, &facts, &mut record, &mut armed);
    }
    note_documented(&mut record, attempt_documented(&facts));
    if signals::tripped() {
        return stop_for_signal(dir, &facts, &mut record, &mut armed);
    }
    if record.state != "os-locked" {
        apply_fallback(&facts, false, &mut record);
    }
    if signals::tripped() {
        return stop_for_signal(dir, &facts, &mut record, &mut armed);
    }
    ask_observations(&mut record);
    if signals::tripped() {
        return stop_for_signal(dir, &facts, &mut record, &mut armed);
    }
    let code = publish(dir, &build_report(&facts, Some(&record)));
    stand_down(&mut armed);
    exit_code(code, &record)
}

fn stop_for_signal(
    dir: &Path,
    facts: &Facts,
    record: &mut ExerciseRecord,
    armed: &mut Option<Armed>,
) -> i32 {
    if interrupt_plan(record.state == "os-locked") == InterruptPlan::FallbackThenExit {
        let private_failed = record.private_call != "returned";
        apply_fallback(facts, private_failed, record);
    }
    if record.detail.is_empty() {
        record.detail = "a signal arrived; the kit did not unlock".to_string();
    }
    let code = publish(dir, &build_report(facts, Some(record)));
    if let Some(armed) = armed.as_mut() {
        armed.finished = true;
    }
    std::process::exit(exit_code(code, record));
}

fn apply_fallback(facts: &Facts, private_failed: bool, record: &mut ExerciseRecord) {
    let documented_failed = record.documented_path == "failed";
    let end = fail_closed_after_error(
        facts.symbol_present,
        private_failed,
        documented_failed,
        immediate(facts),
    );
    record.fail_closed = end.as_str().to_string();
    match end {
        FailClosedEnd::LockDocumented => {
            note_documented(record, attempt_documented(facts));
            if record.state != "os-locked" {
                never_unlocked(record);
            }
        }
        FailClosedEnd::LockPrivate => {
            note_private(record, attempt_private());
            if record.state != "os-locked" {
                never_unlocked(record);
            }
        }
        FailClosedEnd::NeverUnlocked => never_unlocked(record),
    }
}

fn never_unlocked(record: &mut ExerciseRecord) {
    record.state = "never-unlocked".to_string();
    record.result = "error-never-unlocked".to_string();
    record.fail_closed = FailClosedEnd::NeverUnlocked.as_str().to_string();
}

fn note_private(record: &mut ExerciseRecord, attempt: Attempt) {
    match attempt {
        Attempt::Locked {
            returned_ms,
            until_locked_ms,
        } => {
            record.private_call = "returned".to_string();
            record.private_call_returned_ms = returned_ms;
            record.ms_until_locked = Some(until_locked_ms);
            record.state = "os-locked".to_string();
            record.result = "os-locked".to_string();
            record.detail = "ms-until-locked is how long until the session reported locked. private-call-returned-ms is only how long the call took to return.".to_string();
        }
        Attempt::StillRunning => {
            record.private_call = "still-running".to_string();
            push_detail(
                record,
                "the private call was still running when the wait ended",
            );
        }
        Attempt::Failed {
            detail,
            returned_ms,
        } => {
            record.private_call = "failed".to_string();
            record.private_call_returned_ms = returned_ms;
            push_detail(record, &detail);
        }
    }
}

fn note_documented(record: &mut ExerciseRecord, attempt: Attempt) {
    match attempt {
        Attempt::Locked {
            returned_ms,
            until_locked_ms,
        } => {
            record.documented_path = "started".to_string();
            record.documented_started_ms = returned_ms;
            record.ms_until_locked = Some(until_locked_ms);
            record.state = "os-locked".to_string();
            record.result = "os-locked".to_string();
        }
        Attempt::StillRunning => {
            record.documented_path = "failed".to_string();
            push_detail(record, "the screen saver wait timed out");
        }
        Attempt::Failed {
            detail,
            returned_ms,
        } => {
            record.documented_path = "failed".to_string();
            record.documented_started_ms = returned_ms;
            push_detail(record, &detail);
        }
    }
}

fn push_detail(record: &mut ExerciseRecord, detail: &str) {
    if record.detail.is_empty() {
        record.detail = detail.to_string();
    } else if !record.detail.contains(detail) {
        record.detail = format!("{}; {detail}", record.detail);
    }
}

fn exit_code(publish_code: i32, record: &ExerciseRecord) -> i32 {
    if publish_code != 0 {
        publish_code
    } else if record.result == "error-never-unlocked" {
        4
    } else {
        0
    }
}

fn helper(action: HelperAction, permission: KitPermission, dir: &Path) -> i32 {
    detach_helper_session();
    let mut text = String::new();
    let _ = io::stdin().read_to_string(&mut text);
    if !helper_should_lock(action, &text, may_lock(permission)) {
        return 0;
    }
    let facts = collect(false);
    let outcome = helper_lock(&facts);
    let body = match &outcome {
        Ok(label) => format!("helper-end={label}\n"),
        Err(error) => format!(
            "helper-end=never-unlocked\ndetail={}\n",
            one_line(&sanitize(error))
        ),
    };
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join("helper-result.txt"), body);
    if outcome.is_ok() { 0 } else { 4 }
}

fn helper_lock(facts: &Facts) -> Result<&'static str, String> {
    let end = fail_closed_after_error(facts.symbol_present, false, false, immediate(facts));
    match end {
        FailClosedEnd::LockPrivate => match attempt_private() {
            Attempt::Locked { .. } => Ok("lock-private"),
            _ => {
                if immediate(facts) {
                    match attempt_documented(facts) {
                        Attempt::Locked { .. } => Ok("lock-documented"),
                        other => Err(attempt_error(other)),
                    }
                } else {
                    Err("never-unlocked".to_string())
                }
            }
        },
        FailClosedEnd::LockDocumented => match attempt_documented(facts) {
            Attempt::Locked { .. } => Ok("lock-documented"),
            other => Err(attempt_error(other)),
        },
        FailClosedEnd::NeverUnlocked => Err("never-unlocked".to_string()),
    }
}

fn attempt_error(attempt: Attempt) -> String {
    match attempt {
        Attempt::Locked { .. } => "locked".to_string(),
        Attempt::StillRunning => "still-running".to_string(),
        Attempt::Failed { detail, .. } => detail,
    }
}

fn attempt_private() -> Attempt {
    let started = Instant::now();
    let call = wait_private();
    if signals::tripped() {
        return judge(CallWait::Failed("interrupted".to_string()), None);
    }
    let locked = match &call {
        CallWait::Returned(_) => poll_locked(started),
        _ => None,
    };
    if signals::tripped() {
        return judge(CallWait::Failed("interrupted".to_string()), None);
    }
    judge(call, locked)
}

fn attempt_documented(facts: &Facts) -> Attempt {
    let started = Instant::now();
    match start_engine(facts) {
        Err(error) => Attempt::Failed {
            detail: error,
            returned_ms: None,
        },
        Ok(returned_ms) => {
            if signals::tripped() {
                return Attempt::Failed {
                    detail: "interrupted".to_string(),
                    returned_ms: Some(returned_ms),
                };
            }
            match poll_locked(started) {
                Some(until_locked_ms) => Attempt::Locked {
                    returned_ms: Some(returned_ms),
                    until_locked_ms,
                },
                None => Attempt::Failed {
                    detail: "the screen saver started and the session was not locked".to_string(),
                    returned_ms: Some(returned_ms),
                },
            }
        }
    }
}

fn wait_private() -> CallWait {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(invoke_private());
    });
    let deadline = Instant::now() + CALL_TIMEOUT;
    loop {
        if signals::tripped() {
            return CallWait::Failed("interrupted".to_string());
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(ms)) => return CallWait::Returned(ms),
            Ok(Err(error)) => return CallWait::Failed(error),
            Err(RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                return CallWait::TimedOut;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return CallWait::Failed("the private call worker stopped".to_string());
            }
        }
    }
}

fn poll_locked(started: Instant) -> Option<u64> {
    let deadline = Instant::now() + LOCK_POLL;
    loop {
        if signals::tripped() {
            return None;
        }
        if session_is_locked() {
            return Some(elapsed_ms(started.elapsed()));
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

struct SessionApi {
    copy: unsafe extern "C" fn() -> *const std::ffi::c_void,
    get: unsafe extern "C" fn(
        *const std::ffi::c_void,
        *const std::ffi::c_void,
    ) -> *const std::ffi::c_void,
    bool_value: unsafe extern "C" fn(*const std::ffi::c_void) -> u8,
    release: unsafe extern "C" fn(*const std::ffi::c_void),
    type_id: unsafe extern "C" fn(*const std::ffi::c_void) -> libc::c_ulong,
    bool_type: unsafe extern "C" fn() -> libc::c_ulong,
    key: *const std::ffi::c_void,
}

fn session_is_locked() -> bool {
    let Some(api) = session_api() else {
        return false;
    };
    unsafe {
        let dict = (api.copy)();
        if dict.is_null() {
            return false;
        }
        let value = (api.get)(dict, api.key);
        let locked = !value.is_null()
            && (api.type_id)(value) == (api.bool_type)()
            && (api.bool_value)(value) != 0;
        (api.release)(dict);
        locked
    }
}

fn session_api() -> Option<&'static SessionApi> {
    use std::sync::OnceLock;
    static API: OnceLock<Option<SessionApi>> = OnceLock::new();
    API.get_or_init(load_session_api).as_ref()
}

fn load_session_api() -> Option<SessionApi> {
    unsafe {
        let cg = dlopen_path(CORE_GRAPHICS)?;
        let cf = dlopen_path(CORE_FOUNDATION)?;
        let copy = std::mem::transmute::<
            *mut std::ffi::c_void,
            unsafe extern "C" fn() -> *const std::ffi::c_void,
        >(dlsym_name(cg, "CGSessionCopyCurrentDictionary")?);
        let get = std::mem::transmute::<
            *mut std::ffi::c_void,
            unsafe extern "C" fn(
                *const std::ffi::c_void,
                *const std::ffi::c_void,
            ) -> *const std::ffi::c_void,
        >(dlsym_name(cf, "CFDictionaryGetValue")?);
        let bool_value = std::mem::transmute::<
            *mut std::ffi::c_void,
            unsafe extern "C" fn(*const std::ffi::c_void) -> u8,
        >(dlsym_name(cf, "CFBooleanGetValue")?);
        let release = std::mem::transmute::<
            *mut std::ffi::c_void,
            unsafe extern "C" fn(*const std::ffi::c_void),
        >(dlsym_name(cf, "CFRelease")?);
        let type_id = std::mem::transmute::<
            *mut std::ffi::c_void,
            unsafe extern "C" fn(*const std::ffi::c_void) -> libc::c_ulong,
        >(dlsym_name(cf, "CFGetTypeID")?);
        let bool_type = std::mem::transmute::<
            *mut std::ffi::c_void,
            unsafe extern "C" fn() -> libc::c_ulong,
        >(dlsym_name(cf, "CFBooleanGetTypeID")?);
        let key_slot = dlsym_name(cg, "kCGSSessionScreenIsLocked")?;
        let key = *(key_slot as *const *const std::ffi::c_void);
        if key.is_null() {
            return None;
        }
        Some(SessionApi {
            copy,
            get,
            bool_value,
            release,
            type_id,
            bool_type,
            key,
        })
    }
}

fn dlopen_path(path: &str) -> Option<*mut std::ffi::c_void> {
    let path = CString::new(path).ok()?;
    let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LAZY) };
    if handle.is_null() { None } else { Some(handle) }
}

fn dlsym_name(handle: *mut std::ffi::c_void, name: &str) -> Option<*mut std::ffi::c_void> {
    let name = CString::new(name).ok()?;
    let symbol = unsafe { libc::dlsym(handle, name.as_ptr()) };
    if symbol.is_null() { None } else { Some(symbol) }
}

struct Armed {
    child: Child,
    finished: bool,
}

impl Armed {
    fn stand_down(&mut self) {
        if self.finished {
            return;
        }
        if !signals::tripped() {
            if let Some(fd) = signals::take_fd() {
                let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
                let _ = writeln!(file, "{COMPLETE_LINE}");
            }
        }
        let _ = self.child.wait();
        self.finished = true;
    }
}

impl Drop for Armed {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Some(fd) = signals::take_fd() {
            unsafe {
                libc::close(fd);
            }
        }
        let _ = self.child.wait();
    }
}

fn arm_helper(action: HelperAction, dir: &Path) -> Option<Armed> {
    let exe = std::env::current_exe().ok()?;
    let mut child = Command::new(exe)
        .args([
            "--helper",
            action.as_str(),
            "--output-dir",
            &dir.display().to_string(),
        ])
        .env(HELPER_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdin = child.stdin.take()?;
    signals::install(stdin.into_raw_fd());
    Some(Armed {
        child,
        finished: false,
    })
}

fn stand_down(armed: &mut Option<Armed>) {
    if let Some(armed) = armed.as_mut() {
        armed.stand_down();
    }
}

fn publish(dir: &Path, report: &KitReport) -> i32 {
    let mut report = report.clone();
    let violation = safety_violation(&report).map(str::to_string);
    if let Some(why) = &violation {
        report.result = "fail".to_string();
        if report.detail.is_empty() {
            report.detail = why.clone();
        }
    }
    let rendered = render(&report);
    println!("{rendered}");
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join("result.txt"), &rendered);
    if let Ok(json) = serde_json::to_string_pretty(&report) {
        let _ = std::fs::write(dir.join("result.json"), format!("{json}\n"));
    }
    if violation.is_some() { 1 } else { 0 }
}

fn collect(root: bool) -> Facts {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("macos-lock-kit"));
    let exe_text = exe.to_string_lossy().into_owned();
    let otool = capture("/usr/bin/otool", &["-L", &exe_text]);
    let (symbol_present, symbol_detail) = probe_symbol();
    Facts {
        sw_vers: capture("/usr/bin/sw_vers", &[]).stdout,
        uname_m: capture("/usr/bin/uname", &["-m"]).stdout,
        uid_text: capture("/usr/bin/id", &["-u"]).stdout,
        hw_model: capture("/usr/sbin/sysctl", &["-n", "hw.model"]).stdout,
        ask_host: capture(
            "/usr/bin/defaults",
            &[
                "-currentHost",
                "read",
                "com.apple.screensaver",
                "askForPassword",
            ],
        )
        .stdout,
        ask_user: capture(
            "/usr/bin/defaults",
            &["read", "com.apple.screensaver", "askForPassword"],
        )
        .stdout,
        delay_host: capture(
            "/usr/bin/defaults",
            &[
                "-currentHost",
                "read",
                "com.apple.screensaver",
                "askForPasswordDelay",
            ],
        )
        .stdout,
        delay_user: capture(
            "/usr/bin/defaults",
            &["read", "com.apple.screensaver", "askForPasswordDelay"],
        )
        .stdout,
        screen_lock: capture_timed(
            "/usr/sbin/sysadminctl",
            &["-screenLock", "status"],
            SCREEN_LOCK_TIMEOUT,
        ),
        status: capture("/usr/bin/fdesetup", &["status"]),
        isactive: capture("/usr/bin/fdesetup", &["isactive"]),
        symbol_present,
        symbol_detail,
        engine_path: find_engine(),
        otool_checked: otool.exit_code == 0,
        otool: if otool.stdout.is_empty() {
            otool.stderr
        } else {
            otool.stdout
        },
        root,
    }
}

fn capture(program: &str, args: &[&str]) -> CommandText {
    if !read_only_argv_allowed(program, args) {
        return CommandText::new(126, "", "command rejected");
    }
    match Command::new(program).args(args).output() {
        Ok(output) => CommandText::new(
            output.status.code().unwrap_or(1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        Err(error) => CommandText::new(127, "", error.to_string()),
    }
}

fn capture_timed(program: &str, args: &[&str], timeout: Duration) -> CommandText {
    if !read_only_argv_allowed(program, args) {
        return CommandText::new(126, "", "command rejected");
    }
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return CommandText::new(127, "", error.to_string()),
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }
                return CommandText::new(status.code().unwrap_or(1), stdout, stderr);
            }
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return CommandText::new(124, "", "unavailable");
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => return CommandText::new(127, "", error.to_string()),
        }
    }
}

fn find_engine() -> Option<String> {
    SCREEN_SAVER_ENGINE_CANDIDATES
        .iter()
        .find(|path| Path::new(path).is_file())
        .map(|path| (*path).to_string())
}

fn probe_symbol() -> (bool, String) {
    let Ok(path) = CString::new(LOGIN_FRAMEWORK_IMAGE) else {
        return (false, "framework path is not a c string".to_string());
    };
    let Ok(name) = CString::new(PRIVATE_LOCK_SYMBOL) else {
        return (false, "symbol name is not a c string".to_string());
    };
    unsafe {
        let handle = libc::dlopen(path.as_ptr(), libc::RTLD_LAZY);
        if handle.is_null() {
            return (false, dlerror_string());
        }
        let symbol = libc::dlsym(handle, name.as_ptr());
        let present = !symbol.is_null();
        libc::dlclose(handle);
        if present {
            (true, "resolved".to_string())
        } else {
            (false, "symbol missing".to_string())
        }
    }
}

fn invoke_private() -> Result<u64, String> {
    let started = Instant::now();
    let path = CString::new(LOGIN_FRAMEWORK_IMAGE).map_err(|error| error.to_string())?;
    let name = CString::new(PRIVATE_LOCK_SYMBOL).map_err(|error| error.to_string())?;
    unsafe {
        let handle = libc::dlopen(path.as_ptr(), libc::RTLD_LAZY);
        if handle.is_null() {
            return Err(dlerror_string());
        }
        let symbol = libc::dlsym(handle, name.as_ptr());
        if symbol.is_null() {
            return Err("symbol missing".to_string());
        }
        // The framework stays loaded. Closing it can unload code the lock
        // screen is still using. The handle is process-lifetime.
        let function: unsafe extern "C" fn() = std::mem::transmute(symbol);
        function();
    }
    Ok(elapsed_ms(started.elapsed()))
}

fn start_engine(facts: &Facts) -> Result<u64, String> {
    let Some(path) = facts.engine_path.as_deref() else {
        return Err("screen saver engine is missing".to_string());
    };
    if !engine_path_allowed(path) {
        return Err("screen saver engine path is not a documented candidate".to_string());
    }
    let started = Instant::now();
    let mut child = Command::new(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    for _ in 0..40 {
        if signals::tripped() {
            return Err("interrupted".to_string());
        }
        if engine_running() {
            return Ok(elapsed_ms(started.elapsed()));
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("screen saver engine exited {status}"));
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err("screen saver engine did not start".to_string())
}

fn engine_running() -> bool {
    let args = ["-x", "ScreenSaverEngine"];
    if !observation_argv_allowed("/usr/bin/pgrep", &args) {
        return false;
    }
    Command::new("/usr/bin/pgrep")
        .args(args)
        .output()
        .ok()
        .is_some_and(|output| output.status.success() && !output.stdout.is_empty())
}

fn immediate(facts: &Facts) -> bool {
    let ask = password_after_screensaver(
        parse_defaults_value(&facts.ask_host),
        parse_defaults_value(&facts.ask_user),
        parse_defaults_value(&facts.delay_host),
        parse_defaults_value(&facts.delay_user),
    );
    let screen = interpret_screen_lock_status(&facts.screen_lock.stdout, &facts.screen_lock.stderr);
    combine_lock_setting(ask, screen).setting == LockSetting::Immediate
}

fn ask_observations(record: &mut ExerciseRecord) {
    if record.state != "os-locked" {
        return;
    }
    if record.lock_screen_appeared == "not-asked" {
        record.lock_screen_appeared = ask("Did the real lock screen appear?");
    }
    if record.input_reached_desktop_without_password == "not-asked" {
        record.input_reached_desktop_without_password =
            ask("Did a click or a key reach the desktop without the account password?");
    }
    if record.external_display_covered == "not-asked" {
        record.external_display_covered = ask_words(
            "With the external display attached, did the lock cover both screens?",
            &["yes", "no", "none"],
        );
    }
}

fn ask(prompt: &str) -> String {
    ask_words(prompt, &["yes", "no"])
}

fn ask_words(prompt: &str, words: &[&str]) -> String {
    if signals::tripped() || !std::io::IsTerminal::is_terminal(&io::stdin()) {
        return "unanswered".to_string();
    }
    let _ = writeln!(io::stderr(), "{prompt} [{}]", words.join("/"));
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() || signals::tripped() {
        return "unanswered".to_string();
    }
    let trimmed = line.trim();
    if words.contains(&trimmed) {
        trimmed.to_string()
    } else {
        "unanswered".to_string()
    }
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn dlerror_string() -> String {
    unsafe {
        let ptr = libc::dlerror();
        if ptr.is_null() {
            return "dlopen failed".to_string();
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

fn elapsed_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
