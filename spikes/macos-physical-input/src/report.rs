//! Kit report. The log is rendered from this, and the exit code follows it.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use crate::classify::{POST_SOURCES, classify, draft_name};
use crate::scrub::{contains_serial, scrub};
use crate::text::one_line;

pub const KIT_ID: &str = "m05-physical-input";
pub const M14_NAME: &str = "m14";

/// `println!` panics on EPIPE after Terminal closes. This does not.
pub fn write_line(mut out: impl Write, line: &str) {
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

pub fn write_stdout(line: &str) {
    write_line(io::stdout(), line);
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionReport {
    pub product_name: String,
    pub product_version: String,
    pub build_version: String,
    pub uname_m: String,
    pub hw_model: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventRow {
    pub status: String,
    pub outcome: Option<String>,
    pub kind: String,
    pub state_id: Option<i64>,
    pub source_pid: Option<i64>,
    pub poster: Option<String>,
    pub poster_pid: Option<i64>,
    pub tap_saw: Option<bool>,
    pub window_received: Option<bool>,
    pub draft: Option<String>,
    pub note: String,
    pub detection_method: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecureRow {
    pub status: String,
    pub outcome: Option<String>,
    pub disable_event: String,
    pub secure_enabled: String,
    pub window_received: Option<bool>,
    pub note: String,
    pub detection_method: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FallbackRow {
    pub status: String,
    pub outcome: Option<String>,
    pub disable_event: Option<String>,
    pub dead_ms: Option<i64>,
    pub healthy_ms: Option<i64>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KitReport {
    pub kit: String,
    pub mode: String,
    pub version: VersionReport,
    pub tap_created: bool,
    pub tap_released: bool,
    pub tap_option: String,
    pub keyboard_count: i32,
    pub pointing_count: i32,
    pub trackpad: Option<bool>,
    pub physical_key: EventRow,
    pub physical_click: EventRow,
    pub physical_scroll: EventRow,
    pub posts: Vec<EventRow>,
    pub secure: SecureRow,
    pub usb_keyboard: EventRow,
    pub slow: FallbackRow,
    pub marker: FallbackRow,
    pub classification: String,
    pub classification_rule: String,
    pub m14_named: bool,
    pub serial_collected: bool,
    pub locked_session: bool,
    pub restored: bool,
    pub interrupted: bool,
    pub no_input: bool,
    pub probe_crash: Option<String>,
}

pub fn event_row(
    status: &str,
    outcome: Option<&str>,
    kind: &str,
    note: &str,
    method: &str,
) -> EventRow {
    EventRow {
        status: status.to_string(),
        outcome: outcome.map(str::to_string),
        kind: kind.to_string(),
        state_id: None,
        source_pid: None,
        poster: None,
        poster_pid: None,
        tap_saw: None,
        window_received: None,
        draft: None,
        note: note.to_string(),
        detection_method: method.to_string(),
    }
}

pub fn recorded_event(
    kind: &str,
    state_id: i64,
    source_pid: i64,
    window_received: bool,
    tap_saw: bool,
    poster: Option<&str>,
    poster_pid: Option<i64>,
) -> EventRow {
    let class = classify(state_id, source_pid);
    EventRow {
        status: "recorded".to_string(),
        outcome: Some("recorded".to_string()),
        kind: kind.to_string(),
        state_id: Some(state_id),
        source_pid: Some(source_pid),
        poster: poster.map(str::to_string),
        poster_pid,
        tap_saw: Some(tap_saw),
        window_received: Some(window_received),
        draft: Some(draft_name(class).to_string()),
        note: format!(
            "state-id={state_id} source-pid={source_pid} window-received={window_received}"
        ),
        detection_method: "listen-only-tap".to_string(),
    }
}

pub fn runner_skip(kind: &str, subject: &str) -> EventRow {
    event_row(
        "not-run-on-runner",
        Some("not-run-on-runner"),
        kind,
        &format!(
            "{subject} needs a person at the Mac. The GitHub runner has no physical keyboard."
        ),
        "not-run-on-runner",
    )
}

pub fn instructions(kit: bool) -> &'static str {
    if kit {
        "This run creates a listen-only session tap, the same probe m03 used.\n\
         Type one key, click, and scroll the trackpad.\n\
         The kit then posts from a second process. The probe window must be key first. Leave any password field until asked.\n\
         Then focus a password field when asked. Leave it when that row is done.\n\
         Plug in a spare USB keyboard only if you have one.\n\
         Type skip to skip a row. Control-C releases the tap and exits non-zero. Input is never swallowed."
    } else {
        "Runner slice: listen-only session tap and CGEventPost from a second process under each source state.\n\
         Physical, secure-input, and USB rows stay not-run-on-runner. The tap is released before exit."
    }
}

pub fn failure_reasons(report: &KitReport) -> Vec<String> {
    let mut reasons = Vec::new();
    if report.serial_collected {
        reasons.push("a hardware serial number was collected".to_string());
    }
    if report.locked_session {
        reasons.push("the kit must not lock the session".to_string());
    }
    if report.interrupted {
        reasons.push("the kit was interrupted".to_string());
    }
    if report.tap_created && !report.tap_released {
        reasons.push("the listen-only tap was not released".to_string());
    }
    if !report.restored {
        reasons.push("the kit did not release the tap".to_string());
    }
    if report.tap_option != "listen-only" {
        reasons.push("the kit must use a listen-only tap".to_string());
    }
    if let Some(crash) = &report.probe_crash {
        reasons.push(format!("probe crash: {crash}"));
    }
    if contains_serial(&report.version.hw_model) {
        reasons.push("serial text reached the report".to_string());
    }
    if report.mode == "runner" && report.tap_created {
        for (index, source) in POST_SOURCES.iter().enumerate() {
            let Some(post) = report.posts.get(index) else {
                reasons.push(format!("missing second-process post for {source}"));
                continue;
            };
            if post.poster.as_deref() != Some("second-process") {
                reasons.push(format!("{source} was not posted from a second process"));
            }
            if post.tap_saw != Some(true) {
                reasons.push(format!("{source} tap-saw is not true"));
            }
            if post.state_id.is_none() || post.source_pid.is_none() {
                reasons.push(format!("{source} is missing state id or source pid"));
            }
        }
        if report.posts.len() != 3 {
            reasons.push("runner must record three second-process posts".to_string());
        }
    }
    if report.m14_named != report.classification_rule.contains(M14_NAME)
        && report.classification != "pending-physical"
    {
        reasons.push("m14 must be named when the rule is confirmed or replaced".to_string());
    }
    if (report.classification == "confirmed" || report.classification == "replaced")
        && !report.m14_named
    {
        reasons.push("the finding path must name m14".to_string());
    }
    reasons
}

pub fn exit_nonzero(report: &KitReport) -> bool {
    report.interrupted || !failure_reasons(report).is_empty()
}

pub fn row_token(row: &EventRow) -> String {
    row.outcome.clone().unwrap_or_else(|| row.status.clone())
}

pub fn secure_token(row: &SecureRow) -> String {
    row.outcome.clone().unwrap_or_else(|| row.status.clone())
}

pub fn render(report: &KitReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("kit={}\n", report.kit));
    out.push_str(&format!("mode={}\n", report.mode));
    out.push_str(instructions(report.mode == "kit"));
    out.push('\n');
    out.push_str(&format!(
        "sw_vers-product={}\n",
        one_line(&report.version.product_name)
    ));
    out.push_str(&format!(
        "sw_vers-version={}\n",
        one_line(&report.version.product_version)
    ));
    out.push_str(&format!(
        "sw_vers-build={}\n",
        one_line(&report.version.build_version)
    ));
    out.push_str(&format!("uname-m={}\n", one_line(&report.version.uname_m)));
    out.push_str(&format!(
        "hw-model={}\n",
        one_line(&scrub(&report.version.hw_model))
    ));
    out.push_str(&format!("tap-created={}\n", report.tap_created));
    out.push_str(&format!("tap-released={}\n", report.tap_released));
    out.push_str(&format!("tap-option={}\n", report.tap_option));
    out.push_str(&format!("keyboard-count={}\n", report.keyboard_count));
    out.push_str(&format!("pointing-count={}\n", report.pointing_count));
    out.push_str(&format!(
        "trackpad={}\n",
        match report.trackpad {
            Some(true) => "true",
            Some(false) => "false",
            None => "unread",
        }
    ));
    write_event(&mut out, "physical-key", &report.physical_key);
    write_event(&mut out, "physical-click", &report.physical_click);
    write_event(&mut out, "physical-scroll", &report.physical_scroll);
    let labels = ["post-private", "post-combined", "post-hid"];
    for (label, post) in labels.iter().zip(report.posts.iter()) {
        write_event(&mut out, label, post);
    }
    out.push_str(&format!("secure={}\n", secure_token(&report.secure)));
    out.push_str(&format!(
        "secure-disable-event={}\n",
        one_line(&report.secure.disable_event)
    ));
    out.push_str(&format!(
        "secure-enabled={}\n",
        one_line(&report.secure.secure_enabled)
    ));
    out.push_str(&format!(
        "secure-window-received={}\n",
        fmt_flag(report.secure.window_received)
    ));
    out.push_str(&format!(
        "secure-detection={}\n",
        one_line(&report.secure.detection_method)
    ));
    write_event(&mut out, "usb-keyboard", &report.usb_keyboard);
    out.push_str(&format!(
        "slow={}\n",
        report
            .slow
            .outcome
            .clone()
            .unwrap_or_else(|| report.slow.status.clone())
    ));
    out.push_str(&format!(
        "marker={}\n",
        report
            .marker
            .outcome
            .clone()
            .unwrap_or_else(|| report.marker.status.clone())
    ));
    out.push_str(&format!("classification={}\n", report.classification));
    out.push_str(&format!(
        "classification-rule={}\n",
        one_line(&report.classification_rule)
    ));
    out.push_str(&format!("m14-named={}\n", report.m14_named));
    out.push_str(&format!("serial-collected={}\n", report.serial_collected));
    out.push_str(&format!("locked-session={}\n", report.locked_session));
    out.push_str(&format!("restored={}\n", report.restored));
    out.push_str(&format!("interrupted={}\n", report.interrupted));
    out.push_str(&format!("no-input={}\n", report.no_input));
    if let Some(crash) = &report.probe_crash {
        out.push_str(&format!("probe-crash={}\n", one_line(crash)));
    }
    let result = if exit_nonzero(report) { "fail" } else { "pass" };
    out.push_str(&format!("result={result}\n"));
    out
}

fn write_event(out: &mut String, key: &str, row: &EventRow) {
    out.push_str(&format!("{key}={}\n", row_token(row)));
    out.push_str(&format!("{key}-state-id={}\n", fmt_num(row.state_id)));
    out.push_str(&format!("{key}-source-pid={}\n", fmt_num(row.source_pid)));
    out.push_str(&format!(
        "{key}-window-received={}\n",
        fmt_flag(row.window_received)
    ));
    out.push_str(&format!("{key}-tap-saw={}\n", fmt_flag(row.tap_saw)));
    out.push_str(&format!(
        "{key}-poster={}\n",
        one_line(row.poster.as_deref().unwrap_or("none"))
    ));
    out.push_str(&format!("{key}-poster-pid={}\n", fmt_num(row.poster_pid)));
    out.push_str(&format!(
        "{key}-draft={}\n",
        one_line(row.draft.as_deref().unwrap_or("none"))
    ));
    out.push_str(&format!(
        "{key}-detection={}\n",
        one_line(&row.detection_method)
    ));
}

fn fmt_flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "none",
    }
}

fn fmt_num(value: Option<i64>) -> String {
    match value {
        Some(n) => n.to_string(),
        None => "none".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> KitReport {
        KitReport {
            kit: KIT_ID.into(),
            mode: "runner".into(),
            version: VersionReport {
                product_name: "macOS".into(),
                product_version: "26.7.2".into(),
                build_version: "25G332".into(),
                uname_m: "arm64".into(),
                hw_model: "VirtualMac2,1".into(),
            },
            tap_created: true,
            tap_released: true,
            tap_option: "listen-only".into(),
            keyboard_count: 0,
            pointing_count: 0,
            trackpad: None,
            physical_key: runner_skip("key", "A physical key"),
            physical_click: runner_skip("click", "A physical click"),
            physical_scroll: runner_skip("scroll", "A trackpad scroll"),
            posts: vec![
                recorded_event(
                    "post-private",
                    -1,
                    11,
                    true,
                    true,
                    Some("second-process"),
                    Some(11),
                ),
                recorded_event(
                    "post-combined",
                    0,
                    12,
                    true,
                    true,
                    Some("second-process"),
                    Some(12),
                ),
                recorded_event(
                    "post-hid",
                    1,
                    13,
                    true,
                    true,
                    Some("second-process"),
                    Some(13),
                ),
            ],
            secure: SecureRow {
                status: "not-run-on-runner".into(),
                outcome: Some("not-run-on-runner".into()),
                disable_event: "none".into(),
                secure_enabled: "unread".into(),
                window_received: None,
                note: "ci".into(),
                detection_method: "not-run-on-runner".into(),
            },
            usb_keyboard: runner_skip("usb", "A spare USB keyboard"),
            slow: FallbackRow {
                status: "not-applicable".into(),
                outcome: Some("not-run".into()),
                disable_event: None,
                dead_ms: None,
                healthy_ms: None,
                note: "m03 already measured the timeout on the runner".into(),
            },
            marker: FallbackRow {
                status: "not-applicable".into(),
                outcome: Some("not-run".into()),
                disable_event: None,
                dead_ms: None,
                healthy_ms: None,
                note: "m03 already measured the marker on the runner".into(),
            },
            classification: "pending-physical".into(),
            classification_rule: "hid-and-pid-0-is-physical".into(),
            m14_named: true,
            serial_collected: false,
            locked_session: false,
            restored: true,
            interrupted: false,
            no_input: false,
            probe_crash: None,
        }
    }

    #[test]
    fn physical_key_click_and_trackpad_scroll_each_have_a_row() {
        let report = sample();
        let text = render(&report);
        assert!(text.contains("physical-key="));
        assert!(text.contains("physical-click="));
        assert!(text.contains("physical-scroll="));
        assert!(text.contains("physical-key-state-id="));
        assert!(text.contains("physical-key-source-pid="));
        assert!(text.contains("physical-key-window-received="));
    }

    #[test]
    fn a_post_under_each_of_the_three_source_states_has_the_same_columns_from_a_second_process() {
        let report = sample();
        assert_eq!(report.posts.len(), 3);
        for post in &report.posts {
            assert_eq!(post.poster.as_deref(), Some("second-process"));
            assert!(post.state_id.is_some());
            assert!(post.source_pid.is_some());
            assert!(post.window_received.is_some());
        }
        let text = render(&report);
        assert!(text.contains("post-private-poster=second-process"));
        assert!(text.contains("post-combined-poster=second-process"));
        assert!(text.contains("post-hid-poster=second-process"));
        assert!(failure_reasons(&report).is_empty());
    }

    #[test]
    fn secure_input_has_a_row_for_disable_silence_or_keys() {
        let text = render(&sample());
        assert!(text.contains("secure="));
        assert!(text.contains("secure-disable-event="));
        assert!(text.contains("secure-window-received="));
    }

    #[test]
    fn usb_keyboard_hot_plug_has_a_row_or_says_no_second_keyboard() {
        let text = render(&sample());
        assert!(text.contains("usb-keyboard="));
        let mut kit = sample();
        kit.usb_keyboard = event_row(
            "recorded",
            Some("no-second-keyboard"),
            "usb",
            "the machine had no second keyboard",
            "pre-prompt",
        );
        assert_eq!(
            kit.usb_keyboard.outcome.as_deref(),
            Some("no-second-keyboard")
        );
    }

    #[test]
    fn kit_instructions_match_the_post_order() {
        let text = instructions(true);
        assert!(text.contains("posts from a second process"));
        assert!(text.contains("probe window must be key"));
        assert!(text.contains("Leave any password field until asked"));
        assert!(text.contains("Leave it when that row is done"));
        let posts_at = text.find("posts from a second process").unwrap();
        let password_at = text.find("focus a password field").unwrap();
        assert!(
            posts_at < password_at,
            "second-process posts must be described before the password prompt"
        );
    }

    #[test]
    fn a_failure_releases_the_tap_and_does_not_leave_input_blocked() {
        use crate::guard::TapGuard;
        use std::panic::{AssertUnwindSafe, catch_unwind};
        use std::sync::atomic::{AtomicBool, Ordering};

        static RELEASED: AtomicBool = AtomicBool::new(false);
        RELEASED.store(false, Ordering::SeqCst);
        let unwound = catch_unwind(AssertUnwindSafe(|| {
            let _guard = TapGuard::new(|| RELEASED.store(true, Ordering::SeqCst));
            panic!("probe panicked");
        }));
        assert!(unwound.is_err());
        assert!(
            RELEASED.load(Ordering::SeqCst),
            "TapGuard Drop must run dispatch's release path"
        );

        let mut stuck = sample();
        stuck.tap_released = false;
        stuck.restored = false;
        let reasons = failure_reasons(&stuck);
        assert!(reasons.iter().any(|reason| reason.contains("not released")));
        assert!(exit_nonzero(&stuck));
        let ok = sample();
        assert!(ok.tap_released && ok.restored);
        assert!(!exit_nonzero(&ok));
        assert_eq!(ok.tap_option, "listen-only");
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(json.contains("listen-only"));
        assert!(!json.to_ascii_lowercase().contains("serial number"));
    }
}
