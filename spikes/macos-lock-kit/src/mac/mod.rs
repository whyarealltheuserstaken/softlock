//! macOS collection and the two lock paths. This module is the only place
//! that may call the private symbol or start the screen saver.

use std::ffi::{CStr, CString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use softlock_macos::{
    CommandText, FailClosedEnd, KitInvocation, KitPermission, LOGIN_FRAMEWORK_IMAGE,
    PRIVATE_LOCK_SYMBOL, SCREEN_SAVER_ENGINE_CANDIDATES, fail_closed_after_error, may_lock,
    parse_defaults_value, password_after_screensaver, permit,
};

use crate::args::{Args, ExercisePath, Mode};
use crate::commands::{engine_path_allowed, observation_argv_allowed, read_only_argv_allowed};
use crate::helper::{COMPLETE_LINE, HELPER_ENV, HelperAction, helper_should_lock};
use crate::redact::sanitize;
use crate::report::{
    ExerciseRecord, Facts, KitReport, MISSING_SYMBOL_CI_TEST, build_report, render,
    safety_violation,
};
use crate::text::one_line;

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
    let mut record = match invoke_private() {
        Ok(elapsed) => {
            let mut record = ExerciseRecord::blank(ExercisePath::Private.label());
            record.private_call = "returned".to_string();
            record.private_call_returned_ms = Some(elapsed);
            record.state = "os-locked".to_string();
            record.result = "os-locked".to_string();
            record.detail = "private-call-returned-ms is the time until the call returned. It is not proof the lock screen was visible.".to_string();
            record.lock_screen_appeared = ask("Did the real lock screen appear?");
            if record.lock_screen_appeared == "yes" {
                record.input_reached_desktop_without_password =
                    ask("Did a click or a key reach the desktop without the account password?");
            } else if record.lock_screen_appeared == "no" {
                record.detail = format!(
                    "{}; the private call returned and the lock screen was not visible",
                    record.detail
                );
                apply_fallback(&facts, true, &mut record);
                if record.documented_path == "started" {
                    record.lock_screen_appeared = ask("Did the screen saver lock screen appear?");
                    record.input_reached_desktop_without_password =
                        ask("Did a click or a key reach the desktop without the account password?");
                }
            }
            record
        }
        Err(error) => {
            let mut record = ExerciseRecord::blank(ExercisePath::Private.label());
            record.private_call = "failed".to_string();
            record.detail = error;
            apply_fallback(&facts, true, &mut record);
            record
        }
    };
    if record.state == "os-locked" && record.lock_screen_appeared == "not-asked" {
        record.lock_screen_appeared = ask("Did the real lock screen appear?");
        record.input_reached_desktop_without_password =
            ask("Did a click or a key reach the desktop without the account password?");
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
        record.detail = "password after screen saver is off or the delay is not zero, so the documented path was not started".to_string();
        return publish(dir, &build_report(&facts, Some(&record)));
    }
    let mut armed = arm_helper(HelperAction::Documented, dir);
    let mut record = ExerciseRecord::blank(ExercisePath::Documented.label());
    match start_engine(&facts) {
        Ok(elapsed) => {
            record.documented_path = "started".to_string();
            record.documented_started_ms = Some(elapsed);
            record.state = "os-locked".to_string();
            record.result = "os-locked".to_string();
        }
        Err(error) => {
            record.documented_path = "failed".to_string();
            record.detail = error;
            apply_fallback(&facts, false, &mut record);
        }
    }
    if record.state == "os-locked" {
        record.lock_screen_appeared = ask("Did the real lock screen appear?");
        record.input_reached_desktop_without_password =
            ask("Did a click or a key reach the desktop without the account password?");
    }
    let code = publish(dir, &build_report(&facts, Some(&record)));
    stand_down(&mut armed);
    exit_code(code, &record)
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
        FailClosedEnd::LockDocumented => match start_engine(facts) {
            Ok(elapsed) => {
                record.documented_path = "started".to_string();
                record.documented_started_ms = Some(elapsed);
                record.state = "os-locked".to_string();
                record.result = "os-locked".to_string();
            }
            Err(error) => {
                record.documented_path = "failed".to_string();
                record.state = "never-unlocked".to_string();
                record.result = "error-never-unlocked".to_string();
                record.fail_closed = FailClosedEnd::NeverUnlocked.as_str().to_string();
                record.detail = format!("{}; {error}", record.detail);
            }
        },
        FailClosedEnd::LockPrivate => match invoke_private() {
            Ok(elapsed) => {
                record.private_call = "returned".to_string();
                record.private_call_returned_ms = Some(elapsed);
                record.state = "os-locked".to_string();
                record.result = "os-locked".to_string();
            }
            Err(error) => {
                record.private_call = "failed".to_string();
                record.state = "never-unlocked".to_string();
                record.result = "error-never-unlocked".to_string();
                record.fail_closed = FailClosedEnd::NeverUnlocked.as_str().to_string();
                record.detail = format!("{}; {error}", record.detail);
            }
        },
        FailClosedEnd::NeverUnlocked => {
            record.state = "never-unlocked".to_string();
            record.result = "error-never-unlocked".to_string();
        }
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
        FailClosedEnd::LockPrivate => {
            if invoke_private().is_ok() {
                return Ok("lock-private");
            }
            if immediate(facts) && start_engine(facts).is_ok() {
                return Ok("lock-documented");
            }
            Err("never-unlocked".to_string())
        }
        FailClosedEnd::LockDocumented => {
            if start_engine(facts).is_ok() {
                Ok("lock-documented")
            } else {
                Err("never-unlocked".to_string())
            }
        }
        FailClosedEnd::NeverUnlocked => Err("never-unlocked".to_string()),
    }
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
        if let Some(mut stdin) = self.child.stdin.take() {
            let _ = writeln!(stdin, "{COMPLETE_LINE}");
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
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

fn arm_helper(action: HelperAction, dir: &Path) -> Option<Armed> {
    let exe = std::env::current_exe().ok()?;
    let child = Command::new(exe)
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
    password_after_screensaver(
        parse_defaults_value(&facts.ask_host),
        parse_defaults_value(&facts.ask_user),
        parse_defaults_value(&facts.delay_host),
        parse_defaults_value(&facts.delay_user),
    )
    .can_require_immediate_password()
}

fn ask(prompt: &str) -> String {
    if !std::io::IsTerminal::is_terminal(&io::stdin()) {
        return "unanswered".to_string();
    }
    let _ = writeln!(io::stderr(), "{prompt} [yes/no]");
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return "unanswered".to_string();
    }
    match line.trim() {
        "yes" => "yes".to_string(),
        "no" => "no".to_string(),
        _ => "unanswered".to_string(),
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
