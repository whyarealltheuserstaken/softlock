//! Machine-readable kit result. The log lines and `result.json` come from this.

use serde::Serialize;
use softlock_macos::{
    CommandText, LockArm, LockSetting, choose, combine_lock_setting, console_filevault, disk_label,
    interpret_isactive, interpret_screen_lock_status, interpret_status, parse_defaults_value,
    password_after_screensaver, preference_label, refuse_to_arm_message,
};

use crate::redact::{contains_marker, leaks_personal, model_identifier, numeric_uid, sanitize};
use crate::text::one_line;

pub const MISSING_SYMBOL_CI_TEST: &str = "symbol_missing_requests_the_documented_path";
pub const HARDWARE_ONLY: &str = "only hardware proves that a path showed a lock screen, how long that took to appear, whether a click or a key reached the desktop without the account password, and whether a FileVault pre-boot screen appeared";

#[derive(Clone, Debug)]
pub struct Facts {
    pub sw_vers: String,
    pub uname_m: String,
    pub uid_text: String,
    pub hw_model: String,
    pub ask_host: String,
    pub ask_user: String,
    pub delay_host: String,
    pub delay_user: String,
    pub screen_lock: CommandText,
    pub status: CommandText,
    pub isactive: CommandText,
    pub symbol_present: bool,
    pub symbol_detail: String,
    pub engine_path: Option<String>,
    pub otool: String,
    pub otool_checked: bool,
    pub root: bool,
}

#[derive(Clone, Debug)]
pub struct ExerciseRecord {
    pub which: String,
    pub private_call: String,
    pub private_call_returned_ms: Option<u64>,
    pub documented_path: String,
    pub documented_started_ms: Option<u64>,
    pub ms_until_locked: Option<u64>,
    pub lock_screen_appeared: String,
    pub external_display_covered: String,
    pub input_reached_desktop_without_password: String,
    pub state: String,
    pub result: String,
    pub fail_closed: String,
    pub detail: String,
}

impl ExerciseRecord {
    pub fn blank(which: &str) -> Self {
        Self {
            which: which.to_string(),
            private_call: "not-called".to_string(),
            private_call_returned_ms: None,
            documented_path: "not-started".to_string(),
            documented_started_ms: None,
            ms_until_locked: None,
            lock_screen_appeared: "not-asked".to_string(),
            external_display_covered: "not-asked".to_string(),
            input_reached_desktop_without_password: "not-asked".to_string(),
            state: "never-unlocked".to_string(),
            result: "recorded-miss".to_string(),
            fail_closed: "not-needed".to_string(),
            detail: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct KitReport {
    pub mode: String,
    pub result: String,
    pub state: String,
    pub private_symbol: String,
    pub private_symbol_detail: String,
    pub private_call: String,
    pub private_call_returned_ms: Option<u64>,
    pub documented_path: String,
    pub documented_engine: String,
    pub documented_started_ms: Option<u64>,
    pub screen_saver_engine_present: bool,
    pub immediate_password: bool,
    pub ask_for_password: String,
    pub ask_for_password_delay: String,
    pub screen_lock_status: String,
    pub screen_lock_detail: String,
    pub screen_lock_disagreement: bool,
    pub arm: String,
    pub chooser_if_symbol_missing: String,
    pub chooser_if_call_failed: String,
    pub chooser_if_call_succeeded: String,
    pub refuse_message: String,
    pub fdesetup_status: String,
    pub fdesetup_status_exit: i32,
    pub fdesetup_isactive: String,
    pub fdesetup_isactive_exit: i32,
    pub filevault: String,
    pub filevault_pre_boot_observed: bool,
    pub settings_changed: bool,
    pub filevault_changed: bool,
    pub serial_collected: bool,
    pub linked_login_framework: bool,
    pub otool_checked: bool,
    pub uid: String,
    pub uid_is_root: bool,
    pub runner_lock_allowed: bool,
    pub product_name: String,
    pub product_version: String,
    pub build_version: String,
    pub uname_m: String,
    pub hw_model: String,
    pub ms_until_locked: Option<u64>,
    pub lock_screen_appeared: String,
    pub external_display_covered: String,
    pub input_reached_desktop_without_password: String,
    pub fail_closed: String,
    pub missing_symbol_ci_test: String,
    pub hardware_only: String,
    pub detail: String,
}

pub fn build_report(facts: &Facts, exercise: Option<&ExerciseRecord>) -> KitReport {
    let ask = password_after_screensaver(
        parse_defaults_value(&facts.ask_host),
        parse_defaults_value(&facts.ask_user),
        parse_defaults_value(&facts.delay_host),
        parse_defaults_value(&facts.delay_user),
    );
    let screen = interpret_screen_lock_status(&facts.screen_lock.stdout, &facts.screen_lock.stderr);
    let setting = combine_lock_setting(ask, screen);
    let immediate = setting.setting == LockSetting::Immediate;
    let symbol = facts.symbol_present;
    let status = interpret_status(&facts.status);
    let isactive = interpret_isactive(&facts.isactive);
    let filevault = console_filevault(status, isactive);
    let version = parse_sw_vers(&facts.sw_vers);
    let serial = serial_in_facts(facts);
    let refuse = if immediate {
        String::new()
    } else {
        sanitize(&refuse_to_arm_message())
    };
    let (
        mode,
        result,
        state,
        private_call,
        private_ms,
        documented,
        documented_ms,
        appeared,
        input,
        fail_closed,
        detail,
        arm,
        until_locked,
        external_display,
    ) = match exercise {
        None => (
            "read-only".to_string(),
            "pass".to_string(),
            "never-unlocked".to_string(),
            "not-called".to_string(),
            None,
            "not-started".to_string(),
            None,
            "not-asked".to_string(),
            "not-asked".to_string(),
            "not-needed".to_string(),
            String::new(),
            "not-applied".to_string(),
            None,
            "not-asked".to_string(),
        ),
        Some(record) => (
            format!("exercise-{}", record.which),
            record.result.clone(),
            record.state.clone(),
            record.private_call.clone(),
            record.private_call_returned_ms,
            record.documented_path.clone(),
            record.documented_started_ms,
            record.lock_screen_appeared.clone(),
            record.input_reached_desktop_without_password.clone(),
            record.fail_closed.clone(),
            sanitize(&record.detail),
            exercise_arm(record, symbol, immediate),
            record.ms_until_locked,
            record.external_display_covered.clone(),
        ),
    };
    KitReport {
        mode,
        result,
        state,
        private_symbol: if symbol { "present" } else { "missing" }.to_string(),
        private_symbol_detail: sanitize(&facts.symbol_detail),
        private_call,
        private_call_returned_ms: private_ms,
        documented_path: documented,
        documented_engine: facts
            .engine_path
            .as_deref()
            .map(sanitize)
            .unwrap_or_else(|| "missing".to_string()),
        documented_started_ms: documented_ms,
        screen_saver_engine_present: facts.engine_path.is_some(),
        immediate_password: immediate,
        ask_for_password: preference_label(ask.ask),
        ask_for_password_delay: preference_label(ask.delay_seconds),
        screen_lock_status: screen.as_str().to_string(),
        screen_lock_detail: command_body(&facts.screen_lock),
        screen_lock_disagreement: setting.disagreement,
        arm,
        chooser_if_symbol_missing: choose(false, false, immediate).as_str().to_string(),
        chooser_if_call_failed: choose(symbol, false, immediate).as_str().to_string(),
        chooser_if_call_succeeded: choose(symbol, true, immediate).as_str().to_string(),
        refuse_message: refuse,
        fdesetup_status: command_body(&facts.status),
        fdesetup_status_exit: facts.status.exit_code,
        fdesetup_isactive: command_body(&facts.isactive),
        fdesetup_isactive_exit: facts.isactive.exit_code,
        filevault: disk_label(filevault).to_string(),
        filevault_pre_boot_observed: false,
        settings_changed: false,
        filevault_changed: false,
        serial_collected: serial,
        linked_login_framework: links_login_framework(&facts.otool),
        otool_checked: facts.otool_checked,
        uid: numeric_uid(&facts.uid_text),
        uid_is_root: facts.root,
        runner_lock_allowed: false,
        product_name: version.0,
        product_version: version.1,
        build_version: version.2,
        uname_m: sanitize(facts.uname_m.trim()),
        hw_model: model_identifier(&facts.hw_model),
        ms_until_locked: until_locked,
        lock_screen_appeared: appeared,
        external_display_covered: external_display,
        input_reached_desktop_without_password: input,
        fail_closed,
        missing_symbol_ci_test: MISSING_SYMBOL_CI_TEST.to_string(),
        hardware_only: HARDWARE_ONLY.to_string(),
        detail,
    }
}

fn exercise_arm(record: &ExerciseRecord, symbol: bool, immediate: bool) -> String {
    if record.result == "recorded-miss" {
        return "not-applied".to_string();
    }
    if record.result == "refuse" {
        return LockArm::RefuseToArm.as_str().to_string();
    }
    if record.private_call == "returned" && record.documented_path == "not-started" {
        return LockArm::PrivateCall.as_str().to_string();
    }
    if record.documented_path == "started" {
        return LockArm::DocumentedPath.as_str().to_string();
    }
    choose(symbol, record.private_call == "returned", immediate)
        .as_str()
        .to_string()
}

pub fn safety_violation(report: &KitReport) -> Option<&'static str> {
    if report.serial_collected {
        return Some("a hardware serial marker was in the collected text");
    }
    if report.linked_login_framework {
        return Some("the kit binary links login.framework");
    }
    if !report.otool_checked {
        return Some("the kit could not confirm its linked libraries");
    }
    if report.settings_changed || report.filevault_changed {
        return Some("the kit changed a setting");
    }
    if report.uid_is_root || report.uid == "0" {
        return Some("the kit ran as root");
    }
    if report.runner_lock_allowed {
        return Some("the kit allowed a runner lock");
    }
    if report.filevault_pre_boot_observed {
        return Some("the kit claimed a pre-boot observation");
    }
    if report.state == "unlocked" || report.result == "unlocked" {
        return Some("the kit reported unlocked");
    }
    if report.mode == "read-only"
        && (report.private_call != "not-called" || report.documented_path != "not-started")
    {
        return Some("the read-only run invoked a lock path");
    }
    let rendered = render(report);
    if contains_marker(&rendered) || leaks_personal(&rendered) {
        return Some("the rendered result still contains a serial or a personal detail");
    }
    None
}

pub fn render(report: &KitReport) -> String {
    let mut lines = Vec::new();
    push(&mut lines, "mode", &report.mode);
    push(&mut lines, "result", &report.result);
    push(&mut lines, "state", &report.state);
    push(&mut lines, "private-symbol", &report.private_symbol);
    push(
        &mut lines,
        "private-symbol-detail",
        &report.private_symbol_detail,
    );
    push(&mut lines, "private-call", &report.private_call);
    push_opt(
        &mut lines,
        "private-call-returned-ms",
        report.private_call_returned_ms,
    );
    push(&mut lines, "documented-path", &report.documented_path);
    push(&mut lines, "documented-engine", &report.documented_engine);
    push_opt(
        &mut lines,
        "documented-started-ms",
        report.documented_started_ms,
    );
    push_bool(
        &mut lines,
        "screen-saver-engine-present",
        report.screen_saver_engine_present,
    );
    push_bool(&mut lines, "immediate-password", report.immediate_password);
    push(&mut lines, "ask-for-password", &report.ask_for_password);
    push(
        &mut lines,
        "ask-for-password-delay",
        &report.ask_for_password_delay,
    );
    push(&mut lines, "screen-lock-status", &report.screen_lock_status);
    push(&mut lines, "screen-lock-detail", &report.screen_lock_detail);
    push_bool(
        &mut lines,
        "screen-lock-disagreement",
        report.screen_lock_disagreement,
    );
    push(&mut lines, "arm", &report.arm);
    push(
        &mut lines,
        "chooser-if-symbol-missing",
        &report.chooser_if_symbol_missing,
    );
    push(
        &mut lines,
        "chooser-if-call-failed",
        &report.chooser_if_call_failed,
    );
    push(
        &mut lines,
        "chooser-if-call-succeeded",
        &report.chooser_if_call_succeeded,
    );
    if !report.refuse_message.is_empty() {
        push(&mut lines, "refuse-message", &report.refuse_message);
    }
    push(&mut lines, "fdesetup-status", &report.fdesetup_status);
    push(
        &mut lines,
        "fdesetup-status-exit",
        &report.fdesetup_status_exit.to_string(),
    );
    push(&mut lines, "fdesetup-isactive", &report.fdesetup_isactive);
    push(
        &mut lines,
        "fdesetup-isactive-exit",
        &report.fdesetup_isactive_exit.to_string(),
    );
    push(&mut lines, "filevault", &report.filevault);
    push_bool(
        &mut lines,
        "filevault-pre-boot-observed",
        report.filevault_pre_boot_observed,
    );
    push_bool(&mut lines, "settings-changed", report.settings_changed);
    push_bool(&mut lines, "filevault-changed", report.filevault_changed);
    push_bool(&mut lines, "serial-collected", report.serial_collected);
    push_bool(
        &mut lines,
        "linked-login-framework",
        report.linked_login_framework,
    );
    push_bool(&mut lines, "otool-checked", report.otool_checked);
    push(&mut lines, "uid", &report.uid);
    push_bool(&mut lines, "uid-is-root", report.uid_is_root);
    push_bool(
        &mut lines,
        "runner-lock-allowed",
        report.runner_lock_allowed,
    );
    push(&mut lines, "product-name", &report.product_name);
    push(&mut lines, "product-version", &report.product_version);
    push(&mut lines, "build-version", &report.build_version);
    push(&mut lines, "uname-m", &report.uname_m);
    push(&mut lines, "hw-model", &report.hw_model);
    push_opt(&mut lines, "ms-until-locked", report.ms_until_locked);
    push(
        &mut lines,
        "lock-screen-appeared",
        &report.lock_screen_appeared,
    );
    push(
        &mut lines,
        "external-display-covered",
        &report.external_display_covered,
    );
    push(
        &mut lines,
        "input-reached-desktop-without-password",
        &report.input_reached_desktop_without_password,
    );
    push(&mut lines, "fail-closed", &report.fail_closed);
    push(
        &mut lines,
        "missing-symbol-ci-test",
        &report.missing_symbol_ci_test,
    );
    push(&mut lines, "hardware-only", &report.hardware_only);
    if !report.detail.is_empty() {
        push(&mut lines, "detail", &report.detail);
    }
    lines.join("\n") + "\n"
}

pub fn links_login_framework(otool: &str) -> bool {
    otool.lines().any(|line| line.contains("login.framework"))
}

fn serial_in_facts(facts: &Facts) -> bool {
    let blobs = [
        facts.sw_vers.as_str(),
        facts.uname_m.as_str(),
        facts.uid_text.as_str(),
        facts.hw_model.as_str(),
        facts.ask_host.as_str(),
        facts.ask_user.as_str(),
        facts.delay_host.as_str(),
        facts.delay_user.as_str(),
        facts.screen_lock.stdout.as_str(),
        facts.screen_lock.stderr.as_str(),
        facts.status.stdout.as_str(),
        facts.status.stderr.as_str(),
        facts.isactive.stdout.as_str(),
        facts.isactive.stderr.as_str(),
        facts.symbol_detail.as_str(),
        facts.otool.as_str(),
        facts.engine_path.as_deref().unwrap_or(""),
    ];
    blobs.iter().any(|blob| contains_marker(blob))
}

fn command_body(command: &CommandText) -> String {
    let text = if !command.stdout.trim().is_empty() {
        command.stdout.as_str()
    } else {
        command.stderr.as_str()
    };
    let clean = sanitize(text);
    if clean.trim().is_empty() {
        "empty".to_string()
    } else {
        one_line(clean.trim())
    }
}

fn parse_sw_vers(text: &str) -> (String, String, String) {
    let mut name = "unknown".to_string();
    let mut version = "unknown".to_string();
    let mut build = "unknown".to_string();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = sanitize(value.trim());
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "ProductName" => name = value,
            "ProductVersion" => version = value,
            "BuildVersion" => build = value,
            _ => {}
        }
    }
    (name, version, build)
}

fn push(lines: &mut Vec<String>, key: &str, value: &str) {
    lines.push(format!("{key}={}", one_line(&sanitize(value))));
}

fn push_bool(lines: &mut Vec<String>, key: &str, value: bool) {
    lines.push(format!("{key}={}", if value { "true" } else { "false" }));
}

fn push_opt(lines: &mut Vec<String>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        lines.push(format!("{key}={value}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use softlock_macos::FailClosedEnd;

    fn sample() -> Facts {
        Facts {
            sw_vers: "ProductName:\tmacOS\nProductVersion:\t26.7.2\nBuildVersion:\t25G332\n"
                .to_string(),
            uname_m: "arm64\n".to_string(),
            uid_text: "501\n".to_string(),
            hw_model: "Mac14,0\n".to_string(),
            ask_host: "1\n".to_string(),
            ask_user: "1\n".to_string(),
            delay_host: "0\n".to_string(),
            delay_user: "0\n".to_string(),
            screen_lock: CommandText::new(0, "screenLock delay is immediate\n", ""),
            status: CommandText::new(0, "FileVault is On.\n", ""),
            isactive: CommandText::new(0, "true\n", ""),
            symbol_present: true,
            symbol_detail: "resolved".to_string(),
            engine_path: Some(
                "/System/Library/CoreServices/ScreenSaverEngine.app/Contents/MacOS/ScreenSaverEngine"
                    .to_string(),
            ),
            otool: "/usr/lib/libSystem.B.dylib\n".to_string(),
            otool_checked: true,
            root: false,
        }
    }

    #[test]
    fn a_read_only_report_does_not_lock_and_names_the_ci_row() {
        let report = build_report(&sample(), None);
        assert!(safety_violation(&report).is_none());
        let text = render(&report);
        assert!(text.contains("mode=read-only\n"));
        assert!(text.contains("result=pass\n"));
        assert!(text.contains("state=never-unlocked\n"));
        assert!(text.contains("private-call=not-called\n"));
        assert!(text.contains("documented-path=not-started\n"));
        assert!(text.contains("serial-collected=false\n"));
        assert!(text.contains("settings-changed=false\n"));
        assert!(text.contains("filevault-changed=false\n"));
        assert!(text.contains("linked-login-framework=false\n"));
        assert!(text.contains("runner-lock-allowed=false\n"));
        assert!(text.contains("filevault-pre-boot-observed=false\n"));
        assert!(text.contains("filevault=on\n"));
        assert!(text.contains("immediate-password=true\n"));
        assert!(text.contains("screen-lock-status=immediate\n"));
        assert!(text.contains("screen-lock-disagreement=false\n"));
        assert!(text.contains("external-display-covered=not-asked\n"));
        assert!(!text.contains("ms-until-locked="));
        assert!(text.contains("chooser-if-symbol-missing=documented\n"));
        assert!(text.contains("chooser-if-call-failed=documented\n"));
        assert!(text.contains("chooser-if-call-succeeded=private\n"));
        assert!(text.contains(&format!(
            "missing-symbol-ci-test={MISSING_SYMBOL_CI_TEST}\n"
        )));
        assert!(text.contains("hardware-only="));
        assert!(!text.contains("state=unlocked"));
        assert_eq!(report.filevault, "on");
    }

    #[test]
    fn password_off_refuses_and_a_serial_fails_closed_without_printing_it() {
        let mut facts = sample();
        facts.ask_host = "0\n".to_string();
        facts.ask_user = "0\n".to_string();
        facts.screen_lock = CommandText::new(0, "screenLock delay is off\n", "");
        facts.symbol_present = false;
        facts.status = CommandText::new(1, "", "You need to be root to run this tool.\n");
        facts.isactive = CommandText::new(1, "", "permission denied\n");
        facts.hw_model = format!("Mac14,0 {}=SECRET\n", crate::redact::NEEDLES[0]);
        let report = build_report(&facts, None);
        assert!(report.serial_collected);
        assert!(!report.immediate_password);
        assert_eq!(report.chooser_if_symbol_missing, "refuse");
        assert_eq!(report.filevault, "unknown");
        assert!(safety_violation(&report).is_some());
        let text = render(&report);
        assert!(!crate::redact::contains_marker(&text));
        assert!(text.contains("redacted"));
        assert!(text.contains("refuse-message="));
        assert!(text.contains("state=never-unlocked"));
        assert!(!crate::redact::leaks_personal(&text));
    }

    #[test]
    fn a_name_a_home_path_and_another_device_are_not_in_the_result() {
        let mut facts = sample();
        facts.status = CommandText::new(
            0,
            "FileVault is On.\nDeferred enablement appears to be active for user 'Henry'\n",
            "",
        );
        facts.symbol_detail =
            "resolved /Users/Henry/Library henry@example.com 550e8400-e29b-41d4-a716-446655440000"
                .to_string();
        facts.hw_model = "MacBookPro16,2\n".to_string();
        let report = build_report(&facts, None);
        assert!(!report.serial_collected);
        assert!(safety_violation(&report).is_none(), "{report:?}");
        let text = render(&report);
        let json = serde_json::to_string(&report).expect("json");
        for body in [text.as_str(), json.as_str()] {
            assert!(!body.contains("Henry"), "{body}");
            assert!(!body.contains("/Users/"), "{body}");
            assert!(!body.contains('@'), "{body}");
            assert!(!body.contains("550e8400"), "{body}");
            assert!(!crate::redact::leaks_personal(body), "{body}");
        }
        assert!(text.contains("filevault=on"));
        assert!(text.contains("hw-model=MacBookPro16,2"));
    }

    #[test]
    fn a_linked_framework_and_an_unlocked_state_are_violations() {
        let mut facts = sample();
        facts.otool =
            "/System/Library/PrivateFrameworks/login.framework/Versions/A/login\n".to_string();
        facts.otool_checked = true;
        let linked = build_report(&facts, None);
        assert!(linked.linked_login_framework);
        assert!(safety_violation(&linked).unwrap().contains("links"));
        let mut exercise = ExerciseRecord {
            which: "private".to_string(),
            private_call: "not-called".to_string(),
            private_call_returned_ms: None,
            documented_path: "not-started".to_string(),
            documented_started_ms: None,
            ms_until_locked: None,
            lock_screen_appeared: "not-asked".to_string(),
            external_display_covered: "not-asked".to_string(),
            input_reached_desktop_without_password: "not-asked".to_string(),
            state: "unlocked".to_string(),
            result: "pass".to_string(),
            fail_closed: FailClosedEnd::NeverUnlocked.as_str().to_string(),
            detail: String::new(),
        };
        let unlocked = build_report(&sample(), Some(&exercise));
        assert!(safety_violation(&unlocked).unwrap().contains("unlocked"));
        exercise.state = "never-unlocked".to_string();
        exercise.result = "recorded-miss".to_string();
        let miss = build_report(&sample(), Some(&exercise));
        assert_eq!(miss.mode, "exercise-private");
        assert_eq!(miss.arm, "not-applied");
        assert_eq!(miss.private_call, "not-called");
    }
}
